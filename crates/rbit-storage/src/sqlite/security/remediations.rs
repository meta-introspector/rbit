//! Idempotent dependency update branches; publication never overwrites a human-edited branch.
use super::{SecurityStore, SecurityWriteFence};
use crate::StorageError;
use rbit_core::{RepoId, Timestamp};
use serde::Serialize;
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Remediation {
    pub id: i64,
    pub base_sha: String,
    pub package: String,
    pub target_version: String,
    pub branch: String,
    pub commit_sha: Option<String>,
    pub pull_number: Option<i64>,
}
impl SecurityStore {
    pub async fn remediations(&self, repo: RepoId) -> Result<Vec<Remediation>, StorageError> {
        Ok(sqlx::query_as("SELECT id, base_sha, package, target_version, branch, commit_sha, pull_number FROM security_remediations WHERE repo_id = ? ORDER BY id DESC LIMIT 100").bind(repo.0).fetch_all(&self.pool).await?)
    }
    pub async fn record_remediation(&self, repo: RepoId, id: i64, commit: &str, pull: Option<i64>) -> Result<(), StorageError> {
        let changed = sqlx::query("UPDATE security_remediations SET commit_sha = ?, pull_number = coalesce(?, pull_number) WHERE repo_id = ? AND id = ? AND (commit_sha IS NULL OR commit_sha = ?)").bind(commit).bind(pull).bind(repo.0).bind(id).bind(commit).execute(&self.pool).await?;
        if changed.rows_affected() == 0 {
            return Err(StorageError::Conflict("dependency proposal changed"));
        }
        Ok(())
    }
}
impl SecurityWriteFence {
    pub async fn reserve_remediation(
        mut self,
        repo: RepoId,
        base: &str,
        name: &str,
        target: &str,
        branch: &str,
    ) -> Result<Remediation, StorageError> {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM security_remediations WHERE repo_id = ?")
            .bind(repo.0)
            .fetch_one(&mut *self.transaction)
            .await?;
        let existing: Option<Remediation> = sqlx::query_as("SELECT id, base_sha, package, target_version, branch, commit_sha, pull_number FROM security_remediations WHERE repo_id = ? AND package = ? AND target_version = ?").bind(repo.0).bind(name).bind(target).fetch_optional(&mut *self.transaction).await?;
        if let Some(existing) = existing {
            self.transaction.commit().await?;
            return Ok(existing);
        }
        if count >= 100 {
            return Err(StorageError::Conflict("dependency update limit reached; use manual updates"));
        }
        let row = sqlx::query_as("INSERT INTO security_remediations(repo_id, base_sha, package, target_version, branch, created_at) VALUES (?, ?, ?, ?, ?, ?) RETURNING id, base_sha, package, target_version, branch, commit_sha, pull_number").bind(repo.0).bind(base).bind(name).bind(target).bind(branch).bind(Timestamp::now().seconds()).fetch_one(&mut *self.transaction).await?;
        self.transaction.commit().await?;
        Ok(row)
    }
}
