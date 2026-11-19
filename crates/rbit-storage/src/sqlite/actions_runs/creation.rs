//! Run creation accepts a checked deployment fence without taking a nested SQLite writer lock.
use super::{ActionsRunStore, NewRun, Run, RunId, RunRow, Status, corrupt, initial_status, insert_job, placeholder_status, settle};
use crate::{
    StorageError,
    sqlite::actions_deployments::{DeploymentWriteFence, NewIntent, record_intent},
};
use sqlx::SqliteConnection;

impl ActionsRunStore {
    /// Creates a run with its next number and its jobs.
    pub async fn create_run(&self, new: &NewRun<'_>) -> Result<Run, StorageError> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let run = insert_run(&mut transaction, new, None).await?;
        transaction.commit().await?;
        Ok(run)
    }

    /// Creates a deployment run and its immutable intent under an already checked publication fence.
    pub async fn create_deployment_run(
        &self,
        new: &NewRun<'_>,
        mut fence: DeploymentWriteFence,
        intent: &NewIntent<'_>,
    ) -> Result<Run, StorageError> {
        if !new.trusted
            || new.is_fork
            || new.event != rbit_core::actions::RunEvent::WorkflowDispatch
            || new.actor != Some(intent.actor)
            || !new.reference.starts_with("refs/heads/")
            || !new.jobs.iter().any(|job| job.environment.is_some_and(|name| name.eq_ignore_ascii_case(intent.environment)))
            || new.jobs.iter().filter_map(|job| job.environment).any(|name| !name.eq_ignore_ascii_case(intent.environment))
        {
            return Err(StorageError::Conflict("a deployment must execute a trusted dispatch with the selected environment gate"));
        }
        let run = insert_run(&mut fence.transaction, new, Some(intent)).await?;
        fence.transaction.commit().await?;
        Ok(run)
    }
}

async fn insert_run(connection: &mut SqliteConnection, new: &NewRun<'_>, intent: Option<&NewIntent<'_>>) -> Result<Run, StorageError> {
    if new.jobs.is_empty() != new.error.is_some() {
        return Err(corrupt("new run", "a run has jobs, or an error instead"));
    }
    let number: i64 = sqlx::query_scalar(
        "INSERT INTO actions_run_numbers (repo_id, last) VALUES (?, 1) \
             ON CONFLICT (repo_id) DO UPDATE SET last = last + 1 RETURNING last",
    )
    .bind(new.repo.0)
    .fetch_one(&mut *connection)
    .await?;
    let failed = new.error.is_some();
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO actions_runs (repo_id, run_number, workflow_path, workflow_name, title, event, ref, sha, actor_id, \
             trigger_actor_id, pull_number, is_fork, trusted, status, concurrency_group, cancel_in_progress, event_payload, \
             dispatch_inputs, error, created_at, finished_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21) RETURNING id",
    )
    .bind(new.repo.0)
    .bind(number)
    .bind(new.workflow_path)
    .bind(new.workflow_name)
    .bind(new.title)
    .bind(new.event.as_str())
    .bind(new.reference)
    .bind(new.sha)
    .bind(new.actor.map(|id| id.0))
    .bind(new.trigger_actor.map(|id| id.0))
    .bind(new.pull_number)
    .bind(new.is_fork)
    .bind(new.trusted)
    .bind(if failed { Status::Failure } else { placeholder_status(new.start) }.code())
    .bind(new.concurrency_group)
    .bind(new.cancel_in_progress)
    .bind(new.event_payload)
    .bind(new.dispatch_inputs)
    .bind(new.error)
    .bind(new.now.0)
    .bind(failed.then_some(new.now.0))
    .fetch_one(&mut *connection)
    .await?;
    if let Some(intent) = intent {
        record_intent(connection, new, RunId(run_id), number, intent).await?;
    }
    let mut waiting = Vec::new();
    for job in new.jobs {
        // A job with its own `if:` waits for the server's decision like one with needs.
        let status = initial_status(new.start, !job.needs.is_empty() || job.condition);
        let job_id = insert_job(connection, RunId(run_id), new.repo, new.trusted, job, status, new.now).await?;
        if status == Status::Waiting {
            waiting.push(job_id);
        }
    }
    if !failed {
        settle(connection, RunId(run_id), waiting, new.now).await?;
    }
    let row: RunRow = sqlx::query_as(select_run!("WHERE id = ?")).bind(run_id).fetch_one(&mut *connection).await?;
    Run::try_from(row)
}
