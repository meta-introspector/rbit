//! Deployment intent and new workflow run are recorded together or rolled back together.
use super::ActionsDeploymentStore;
use crate::{StorageError, sqlite::actions_runs::NewRun};
use rbit_core::{AccountId, actions::RunId};
use sqlx::SqliteConnection;

/// The exact source identity selected for one execution of the configured current workflow.
#[derive(Debug)]
pub struct NewIntent<'a> {
    pub actor: AccountId,
    pub operation: &'a str,
    pub environment: &'a str,
    pub source_attempt: i64,
    pub source_sha: &'a str,
    pub artifact: Option<(i64, &'a str)>,
    pub configuration_id: i64,
    pub configuration_generation: i64,
    pub workflow_sha: &'a str,
}

pub(in crate::sqlite) async fn record_intent(
    connection: &mut SqliteConnection,
    run: &NewRun<'_>,
    run_id: RunId,
    run_number: i64,
    intent: &NewIntent<'_>,
) -> Result<(), StorageError> {
    let changed = sqlx::query(
            "INSERT INTO actions_deployment_intents (repo_id, actor_id, operation, environment, source_attempt_id, source_sha, \
             artifact_id, artifact_digest, workflow_path, workflow_sha, configuration_id, configuration_generation, run_id, created_at, workflow_ref, run_number, origin_attempt_id, origin_artifact_id, origin_artifact_digest) \
             SELECT ?1, ?2, ?3, c.environment, a.id, a.sha, ?7, ?8, c.workflow_path, ?10, c.id, c.generation, ?13, ?14, ?16, ?17, o.id, COALESCE(?7, a.artifact_id), COALESCE(?8, a.artifact_digest) \
             FROM actions_job_attempt_history a JOIN actions_deployment_workflows c ON c.repo_id = a.repo_id \
             JOIN actions_job_attempt_history o ON o.id = COALESCE(a.origin_attempt_id, a.id) AND o.repo_id = a.repo_id AND o.sha = a.sha \
             WHERE a.repo_id = ?1 AND a.id = ?5 AND a.sha = ?6 AND a.status = 1 AND a.trusted = 1 AND a.is_fork = 0 \
             AND a.event <> 'pull_request' AND c.environment = ?4 AND c.id = ?11 AND c.generation = ?12 \
             AND c.workflow_path = ?9 AND ?10 = ?15 AND \
             ((?3 = 'promote' AND a.environment = c.source_environment COLLATE NOCASE) \
              OR (?3 = 'rollback' AND a.environment = c.environment COLLATE NOCASE))",
        )
        .bind(run.repo.0)
        .bind(intent.actor.0)
        .bind(intent.operation)
        .bind(intent.environment)
        .bind(intent.source_attempt)
        .bind(intent.source_sha)
        .bind(intent.artifact.map(|(id, _)| id))
        .bind(intent.artifact.map(|(_, digest)| digest))
        .bind(run.workflow_path)
        .bind(intent.workflow_sha)
        .bind(intent.configuration_id)
        .bind(intent.configuration_generation)
        .bind(run_id.0)
        .bind(run.now.0)
        .bind(run.sha)
        .bind(run.reference)
        .bind(run_number)
        .execute(&mut *connection)
        .await?;
    if changed.rows_affected() != 1 {
        return Err(StorageError::Conflict("the selected deployment source or configuration changed"));
    }
    Ok(())
}

/// Audited requested identity, separately from the resulting workflow's observed outcome.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Intent {
    pub id: i64,
    pub actor_id: Option<i64>,
    pub operation: String,
    pub environment: String,
    pub source_attempt_id: i64,
    pub source_sha: String,
    pub origin_attempt_id: i64,
    pub origin_artifact_id: Option<i64>,
    pub origin_artifact_digest: Option<String>,
    pub artifact_id: Option<i64>,
    pub artifact_digest: Option<String>,
    pub workflow_path: String,
    pub workflow_sha: String,
    pub workflow_ref: String,
    pub run_id: Option<i64>,
    pub run_number: i64,
    pub workflow_status: Option<i64>,
    pub created_at: i64,
}

impl ActionsDeploymentStore {
    /// Intent-bound runs are restarted through a new current-workflow deployment request.
    pub async fn is_deployment_run(&self, repo: rbit_core::RepoId, run: rbit_core::actions::RunId) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions_deployment_intents WHERE repo_id = ? AND run_id = ?)")
            .bind(repo.0)
            .bind(run.0)
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn intents(&self, repo: rbit_core::RepoId, limit: u32) -> Result<Vec<Intent>, StorageError> {
        Ok(sqlx::query_as(
            "SELECT i.id, i.actor_id, i.operation, i.environment, i.source_attempt_id, i.source_sha, i.origin_attempt_id, i.origin_artifact_id, i.origin_artifact_digest, i.artifact_id, \
             i.artifact_digest, i.workflow_path, i.workflow_sha, i.workflow_ref, i.run_id, i.run_number, \
             r.status AS workflow_status, i.created_at FROM actions_deployment_intents i \
             LEFT JOIN actions_runs r ON r.id = i.run_id AND r.repo_id = i.repo_id AND r.run_number = i.run_number \
             WHERE i.repo_id = ? ORDER BY i.id DESC LIMIT ?",
        )
        .bind(repo.0)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?)
    }
}
