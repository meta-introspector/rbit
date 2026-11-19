//! Immutable completed job attempts and the repository's deployment workflow configuration.
mod active;
pub use active::ActiveAttempt;
mod configuration;
mod intents;
pub use configuration::{Configuration, DeploymentWriteFence};
pub(in crate::sqlite) use intents::record_intent;
pub use intents::{Intent, NewIntent};

use crate::StorageError;
use rbit_core::{
    RepoId, Timestamp,
    actions::{JobId, RunId, Status},
};
use sqlx::SqlitePool;

use super::actions_rows::status;

#[derive(Debug, Clone)]
pub struct ActionsDeploymentStore {
    pool: SqlitePool,
}

/// A completed attempt, retained independently of mutable jobs and run retention.
#[derive(Debug, Clone)]
pub struct Attempt {
    pub id: i64,
    pub repo_id: RepoId,
    pub run_id: RunId,
    pub run_number: i64,
    pub run_available: bool,
    pub run_attempt: i64,
    pub job_id: JobId,
    pub job_attempt: i64,
    pub job_key: String,
    pub job_name: String,
    pub workflow_path: String,
    pub reference: String,
    pub sha: String,
    pub workflow_sha: String,
    pub origin_attempt_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub artifact_digest: Option<String>,
    pub trusted: bool,
    pub is_fork: bool,
    pub event: String,
    pub environment: Option<String>,
    pub environment_url: Option<String>,
    pub status: Status,
    pub created_at: Timestamp,
    pub started_at: Option<Timestamp>,
    pub finished_at: Option<Timestamp>,
}

#[derive(Debug, sqlx::FromRow)]
struct AttemptRow {
    id: i64,
    repo_id: i64,
    run_id: i64,
    run_number: i64,
    run_available: bool,
    run_attempt: i64,
    job_id: i64,
    job_attempt: i64,
    job_key: String,
    job_name: String,
    workflow_path: String,
    reference: String,
    sha: String,
    workflow_sha: String,
    origin_attempt_id: Option<i64>,
    artifact_id: Option<i64>,
    artifact_digest: Option<String>,
    trusted: bool,
    is_fork: bool,
    event: String,
    environment: Option<String>,
    environment_url: Option<String>,
    status: i64,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

impl TryFrom<AttemptRow> for Attempt {
    type Error = StorageError;
    fn try_from(row: AttemptRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            repo_id: RepoId(row.repo_id),
            run_id: RunId(row.run_id),
            run_number: row.run_number,
            run_available: row.run_available,
            run_attempt: row.run_attempt,
            job_id: JobId(row.job_id),
            job_attempt: row.job_attempt,
            job_key: row.job_key,
            job_name: row.job_name,
            workflow_path: row.workflow_path,
            reference: row.reference,
            sha: row.sha,
            workflow_sha: row.workflow_sha,
            origin_attempt_id: row.origin_attempt_id,
            artifact_id: row.artifact_id,
            artifact_digest: row.artifact_digest,
            trusted: row.trusted,
            is_fork: row.is_fork,
            event: row.event,
            environment: row.environment,
            environment_url: row.environment_url,
            status: status(row.status)?,
            created_at: Timestamp(row.created_at),
            started_at: row.started_at.map(Timestamp),
            finished_at: row.finished_at.map(Timestamp),
        })
    }
}

macro_rules! select_attempt {
    ($clause:expr) => {
        concat!(
            "SELECT id, repo_id, run_id, run_number, run_attempt, job_id, job_attempt, job_key, job_name, workflow_path, \
             ref AS reference, sha, workflow_sha, origin_attempt_id, artifact_id, artifact_digest, trusted, is_fork, event, environment, environment_url, status, created_at, started_at, \
             finished_at, EXISTS(SELECT 1 FROM actions_runs r WHERE r.id = actions_job_attempt_history.run_id \
             AND r.repo_id = actions_job_attempt_history.repo_id AND r.run_number = actions_job_attempt_history.run_number) \
             AS run_available FROM actions_job_attempt_history ",
            $clause
        )
    };
}

impl ActionsDeploymentStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Completed environment attempts, newest first, in one bounded page.
    pub async fn attempts(&self, repo: RepoId, before: Option<i64>, limit: u32) -> Result<Vec<Attempt>, StorageError> {
        let rows: Vec<AttemptRow> = sqlx::query_as(select_attempt!(
            "WHERE repo_id = ?1 AND environment IS NOT NULL AND (?2 IS NULL OR id < ?2) ORDER BY id DESC LIMIT ?3"
        ))
        .bind(repo.0)
        .bind(before)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Attempt::try_from).collect()
    }

    /// Whether an artifact's producing job completed successfully in this exact recorded run attempt.
    pub async fn artifact_job(&self, repo: RepoId, run_number: i64, run_attempt: i64, job: JobId) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM actions_job_attempt_history WHERE repo_id = ? AND run_number = ? \
             AND run_attempt = ? AND job_id = ? AND status = 1 AND trusted = 1 AND is_fork = 0 AND event <> 'pull_request')",
        )
        .bind(repo.0)
        .bind(run_number)
        .bind(run_attempt)
        .bind(job.0)
        .fetch_one(&self.pool)
        .await?)
    }

    /// An exact completed attempt, scoped to its repository.
    pub async fn attempt(&self, repo: RepoId, id: i64) -> Result<Option<Attempt>, StorageError> {
        let row: Option<AttemptRow> =
            sqlx::query_as(select_attempt!("WHERE repo_id = ? AND id = ?")).bind(repo.0).bind(id).fetch_optional(&self.pool).await?;
        row.map(Attempt::try_from).transpose()
    }
}
