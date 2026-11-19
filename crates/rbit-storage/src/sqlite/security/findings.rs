//! Scoped finding reads, audited dismissal fields and bounded gate evidence.
use super::{Occurrence, SecurityStore, SecurityWriteFence};
use crate::StorageError;
use rbit_core::{AccountId, RepoId, Timestamp};
impl SecurityStore {
    pub async fn occurrences(&self, repo: RepoId, scan: i64, offset: u32, limit: u32) -> Result<Vec<Occurrence>, StorageError> {
        Ok(sqlx::query_as("SELECT f.id, f.fingerprint, o.rule, o.message, o.path, o.line, o.severity, f.reason, f.note, f.dismissed_by, f.dismissed_at FROM security_occurrences o JOIN security_findings f ON f.id = o.finding_id JOIN security_scans s ON s.id = o.scan_id WHERE f.repo_id = ? AND s.repo_id = ? AND s.id = ? ORDER BY o.severity DESC, f.id LIMIT ? OFFSET ?")
            .bind(repo.0).bind(repo.0).bind(scan).bind(limit.min(1000)).bind(offset).fetch_all(&self.pool).await?)
    }
    pub async fn dismissed(&self, repo: RepoId, scanner: &str) -> Result<Vec<String>, StorageError> {
        Ok(sqlx::query_scalar(
            "SELECT fingerprint FROM security_findings WHERE repo_id = ? AND scanner = ? AND reason IS NOT NULL LIMIT 10000",
        )
        .bind(repo.0)
        .bind(scanner)
        .fetch_all(&self.pool)
        .await?)
    }
}

impl SecurityWriteFence {
    pub async fn dismiss(mut self, repo: RepoId, id: i64, actor: AccountId, reason: Option<(&str, &str)>) -> Result<bool, StorageError> {
        let (reason, note) = reason.map_or((None, None), |(reason, note)| (Some(reason), Some(note)));
        let changed = sqlx::query(
            "UPDATE security_findings SET dismissed_by = ?, dismissed_at = ?, reason = ?, note = ? WHERE repo_id = ? AND id = ?",
        )
        .bind(reason.map(|_| actor.0))
        .bind(reason.map(|_| Timestamp::now().seconds()))
        .bind(reason)
        .bind(note)
        .bind(repo.0)
        .bind(id)
        .execute(&mut *self.transaction)
        .await?
        .rows_affected()
            > 0;
        self.transaction.commit().await?;
        Ok(changed)
    }
}
