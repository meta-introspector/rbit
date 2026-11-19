//! Creation defaults must not overwrite settings written before event delivery.
use super::ActionsSettingsStore;
use crate::StorageError;
use rbit_core::{RepoId, Timestamp};

impl ActionsSettingsStore {
    /// Initialize only an absent row. A delayed or replayed creation event cannot
    /// replace an administrator's already persisted Actions switch or permissions.
    pub async fn initialize(&self, repo: RepoId, enabled: bool, now: Timestamp) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO actions_repositories (repo_id, enabled, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT (repo_id) DO NOTHING",
        )
        .bind(repo.0)
        .bind(enabled)
        .bind(now.0)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
