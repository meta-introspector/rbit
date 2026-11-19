//! Atomic, bounded issue relationship graphs with current endpoint authority.
mod authority;
pub(super) mod read;
use super::pool::{Pool, WriteTransaction};
use crate::StorageError;
use authority::{endpoint, require_pair};
use rbit_collaboration::issue_relations::{MAX_LINKS, MAX_WALK, RelationInput};

#[derive(Debug, thiserror::Error)]
pub enum RelationError {
    #[error("not found")]
    NotFound,
    #[error("write access to both repositories is required")]
    Forbidden,
    #[error("relationship cannot be added; check its direction and limits")]
    Conflict,
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Storage(#[from] StorageError),
}
impl From<sqlx::Error> for RelationError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.into())
    }
}
type Result<T> = std::result::Result<T, RelationError>;
#[derive(Debug, Clone)]
pub struct RelationStore {
    pool: Pool,
}

impl RelationStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// A parent depends on its children; a blocked issue depends on its blockers.
    /// The combined directed graph must remain acyclic. Conflicts deliberately do
    /// not disclose which hidden edge or capacity bound prevented an insertion.
    pub async fn add(&self, actor: i64, source: i64, input: &RelationInput) -> Result<i64> {
        input.validate().map_err(RelationError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        let from = endpoint(&mut tx, source, Some(actor)).await?;
        let target: i64 = sqlx::query_scalar("SELECT i.id FROM issues i JOIN repositories r ON r.id = i.repo_id WHERE r.owner_id = ?1 AND i.number = ?2 AND ((?3 = '' AND r.id = ?4) OR r.name = ?3 COLLATE NOCASE)")
            .bind(from.owner).bind(input.number).bind(input.repository.trim()).bind(from.repo).fetch_optional(&mut *tx).await?.ok_or(RelationError::NotFound)?;
        let to = endpoint(&mut tx, target, Some(actor)).await?;
        require_pair(&from, &to)?;
        let existing: Option<i64> = sqlx::query_scalar("SELECT id FROM issue_relations WHERE source_id = ? AND target_id = ? AND kind = ?")
            .bind(source)
            .bind(target)
            .bind(input.kind.as_str())
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(id) = existing {
            return Ok(id);
        }
        admissible(&mut tx, source, target, input.kind.as_str()).await?;
        let id = sqlx::query_scalar(
            "INSERT INTO issue_relations(source_id, target_id, kind, created_at) VALUES(?, ?, ?, unixepoch()) RETURNING id",
        )
        .bind(source)
        .bind(target)
        .bind(input.kind.as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Only the exact edge can be removed; deletion and re-addition allocate a new id.
    pub async fn remove(&self, actor: i64, issue: i64, edge: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        endpoint(&mut tx, issue, Some(actor)).await?;
        let (source, target): (i64, i64) =
            sqlx::query_as("SELECT source_id, target_id FROM issue_relations WHERE id = ? AND (source_id = ? OR target_id = ?)")
                .bind(edge)
                .bind(issue)
                .bind(issue)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(RelationError::NotFound)?;
        let from = endpoint(&mut tx, source, Some(actor)).await?;
        let to = endpoint(&mut tx, target, Some(actor)).await?;
        require_pair(&from, &to)?;
        sqlx::query("DELETE FROM issue_relations WHERE id = ?").bind(edge).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn admissible(tx: &mut WriteTransaction, source: i64, target: i64, kind: &str) -> Result<()> {
    if source == target {
        return Err(RelationError::Conflict);
    }
    let parent: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM issue_relations WHERE target_id = ? AND kind = 'child')")
        .bind(target)
        .fetch_one(&mut **tx)
        .await?;
    if kind == "child" && parent {
        return Err(RelationError::Conflict);
    }
    for endpoint in [source, target] {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM issue_relations WHERE source_id = ? OR target_id = ?")
            .bind(endpoint)
            .bind(endpoint)
            .fetch_one(&mut **tx)
            .await?;
        if count >= MAX_LINKS {
            return Err(RelationError::Conflict);
        }
    }
    // UNION deduplicates diamonds; LIMIT bounds visited nodes, each with bounded fanout.
    let reached: Vec<i64> = sqlx::query_scalar("WITH RECURSIVE reached(id) AS (VALUES(?1) UNION SELECT e.target_id FROM issue_relations e JOIN reached ON e.source_id = reached.id LIMIT ?2) SELECT id FROM reached")
        .bind(target).bind(MAX_WALK + 1).fetch_all(&mut **tx).await?;
    if reached.contains(&source) || reached.len() > usize::try_from(MAX_WALK).unwrap_or(1000) {
        return Err(RelationError::Conflict);
    }
    Ok(())
}
