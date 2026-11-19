//! Private review progress and a short publication fence against mutable policy changes.
use super::ReviewStore;
use crate::{StorageError, sqlite::pool::WriteTransaction};
use rbit_core::{AccountId, IssueId, Timestamp};

/// Keeps SQLite writers out through a guarded Git CAS. Dropping rolls back without writes.
#[derive(Debug)]
pub struct ReviewWriteFence {
    _transaction: WriteTransaction,
}

impl ReviewStore {
    pub async fn publication_fence(&self) -> Result<ReviewWriteFence, StorageError> {
        Ok(ReviewWriteFence { _transaction: self.pool.begin_immediate().await? })
    }

    pub async fn progress(&self, pull: IssueId, reviewer: AccountId) -> Result<Vec<(String, String)>, StorageError> {
        Ok(sqlx::query_as(
            "SELECT path, fingerprint FROM pull_review_progress WHERE pull_id = ? AND reviewer_id = ? ORDER BY path LIMIT 1000",
        )
        .bind(pull.0)
        .bind(reviewer.0)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn set_progress(
        &self,
        pull: IssueId,
        reviewer: AccountId,
        path: &str,
        fingerprint: Option<&str>,
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        if let Some(fingerprint) = fingerprint {
            let count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM pull_review_progress WHERE pull_id = ? AND reviewer_id = ? AND path != ?")
                    .bind(pull.0)
                    .bind(reviewer.0)
                    .bind(path)
                    .fetch_one(&mut *tx)
                    .await?;
            if count >= 1000 {
                sqlx::query("DELETE FROM pull_review_progress WHERE pull_id = ? AND reviewer_id = ? AND path = (SELECT path FROM pull_review_progress WHERE pull_id = ? AND reviewer_id = ? ORDER BY updated_at, path LIMIT 1)").bind(pull.0).bind(reviewer.0).bind(pull.0).bind(reviewer.0).execute(&mut *tx).await?;
            }
            sqlx::query("INSERT INTO pull_review_progress (pull_id, reviewer_id, path, fingerprint, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (pull_id, reviewer_id, path) DO UPDATE SET fingerprint = excluded.fingerprint, updated_at = excluded.updated_at")
                .bind(pull.0).bind(reviewer.0).bind(path).bind(fingerprint).bind(Timestamp::now().seconds()).execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM pull_review_progress WHERE pull_id = ? AND reviewer_id = ? AND path = ?")
                .bind(pull.0)
                .bind(reviewer.0)
                .bind(path)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn last_reviewed_head(&self, pull: IssueId, reviewer: AccountId) -> Result<Option<String>, StorageError> {
        Ok(sqlx::query_scalar("SELECT commit_sha FROM pull_reviews WHERE pull_id = ? AND reviewer_id = ? AND state != 'pending' ORDER BY submitted_at DESC, id DESC LIMIT 1")
            .bind(pull.0).bind(reviewer.0).fetch_optional(&self.pool).await?)
    }
}
