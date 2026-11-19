//! Current unfinished environment jobs are read separately from immutable completed history.
use super::ActionsDeploymentStore;
use crate::StorageError;
use rbit_core::RepoId;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ActiveAttempt {
    pub job_id: i64,
    pub job_name: String,
    pub job_attempt: i64,
    pub run_number: i64,
    pub sha: String,
    pub workflow_sha: String,
    pub origin_attempt_id: Option<i64>,
    pub artifact_id: Option<i64>,
    pub artifact_digest: Option<String>,
    pub environment: String,
    pub environment_url: Option<String>,
    pub status: i64,
}

impl ActionsDeploymentStore {
    pub async fn active_attempts(&self, repo: RepoId) -> Result<Vec<ActiveAttempt>, StorageError> {
        Ok(sqlx::query_as(
            "SELECT j.id AS job_id, j.name AS job_name, j.attempt AS job_attempt, r.run_number, COALESCE(i.source_sha, r.sha) AS sha, r.sha AS workflow_sha, \
             i.origin_attempt_id, i.origin_artifact_id AS artifact_id, i.origin_artifact_digest AS artifact_digest, \
             j.environment, j.environment_url, j.status FROM actions_jobs j JOIN actions_runs r ON r.id = j.run_id \
             LEFT JOIN actions_deployment_intents i ON i.repo_id = r.repo_id AND i.run_number = r.run_number \
             WHERE j.repo_id = ? AND j.environment IS NOT NULL AND j.status NOT BETWEEN 1 AND 4 \
             ORDER BY j.id DESC LIMIT 100",
        )
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await?)
    }
}
