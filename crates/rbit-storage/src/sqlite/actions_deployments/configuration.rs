//! Deployment configuration changes hold the same writer fence as current authority checks.
use super::ActionsDeploymentStore;
use crate::StorageError;
use rbit_core::{RepoId, Timestamp};
use sqlx::{Sqlite, Transaction};

/// Excludes policy writers until a checked configuration or deployment run is persisted.
#[derive(Debug)]
pub struct DeploymentWriteFence {
    pub(in crate::sqlite) transaction: Transaction<'static, Sqlite>,
}

/// A configured route from staging to a target environment, executed by a current workflow.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Configuration {
    pub id: i64,
    pub environment: String,
    pub source_environment: String,
    pub workflow_path: String,
    pub generation: i64,
    pub updated_at: i64,
}

impl ActionsDeploymentStore {
    pub async fn publication_fence(&self) -> Result<DeploymentWriteFence, StorageError> {
        Ok(DeploymentWriteFence { transaction: self.pool.begin_with("BEGIN IMMEDIATE").await? })
    }

    pub async fn configurations(&self, repo: RepoId) -> Result<Vec<Configuration>, StorageError> {
        Ok(sqlx::query_as(
            "SELECT id, environment, source_environment, workflow_path, generation, updated_at \
             FROM actions_deployment_workflows WHERE repo_id = ? ORDER BY environment LIMIT 100",
        )
        .bind(repo.0)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn configuration(&self, repo: RepoId, environment: &str) -> Result<Option<Configuration>, StorageError> {
        Ok(sqlx::query_as(
            "SELECT id, environment, source_environment, workflow_path, generation, updated_at \
             FROM actions_deployment_workflows WHERE repo_id = ? AND environment = ?",
        )
        .bind(repo.0)
        .bind(environment)
        .fetch_optional(&self.pool)
        .await?)
    }
}

impl DeploymentWriteFence {
    /// Persists an administrator's checked mapping. Stable identity plus generation bind future submissions.
    pub async fn put_configuration(
        mut self,
        repo: RepoId,
        environment: &str,
        source_environment: &str,
        workflow_path: &str,
        now: Timestamp,
    ) -> Result<Configuration, StorageError> {
        let configured = sqlx::query_as(
            "INSERT INTO actions_deployment_workflows (repo_id, environment, source_environment, workflow_path, generation, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 1, ?5) ON CONFLICT (repo_id, environment) DO UPDATE SET \
             source_environment = excluded.source_environment, workflow_path = excluded.workflow_path, \
             generation = actions_deployment_workflows.generation + 1, updated_at = excluded.updated_at \
             RETURNING id, environment, source_environment, workflow_path, generation, updated_at",
        )
        .bind(repo.0)
        .bind(environment)
        .bind(source_environment)
        .bind(workflow_path)
        .bind(now.0)
        .fetch_one(&mut *self.transaction)
        .await?;
        self.transaction.commit().await?;
        Ok(configured)
    }
}
