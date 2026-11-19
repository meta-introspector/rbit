//! Board-scoped iteration catalogs with current owner authority and revision fencing.
use super::{ProjectError, ProjectStore, Result, WriteTransaction, authority, board};
use rbit_collaboration::projects::{Iteration, IterationInput, MAX_ITERATIONS};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    title: String,
    starts_on: String,
    ends_on: String,
    archived: bool,
    revision: i64,
}
impl From<Row> for Iteration {
    fn from(r: Row) -> Self {
        Self { id: r.id, title: r.title, starts_on: r.starts_on, ends_on: r.ends_on, archived: r.archived, revision: r.revision }
    }
}
pub(super) async fn list(tx: &mut WriteTransaction, id: i64) -> Result<Vec<Iteration>> {
    let rows: Vec<Row> = sqlx::query_as("SELECT id, title, starts_on, ends_on, archived, revision FROM project_iterations WHERE board_id = ? ORDER BY starts_on, id LIMIT ?")
        .bind(id).bind(MAX_ITERATIONS).fetch_all(&mut **tx).await?;
    Ok(rows.into_iter().map(Into::into).collect())
}
impl ProjectStore {
    pub async fn iterations(&self, org: i64, actor: i64, id: i64) -> Result<Vec<Iteration>> {
        let mut tx = self.pool.begin().await?;
        authority(&mut tx, org, actor, false).await?;
        board(&mut tx, org, id).await?;
        list(&mut tx, id).await
    }
    /// `existing` is the exact id/revision to update; absent creates a new iteration.
    pub async fn save_iteration(&self, org: i64, actor: i64, id: i64, existing: Option<(i64, i64)>, input: &IterationInput) -> Result<i64> {
        input.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, true).await?;
        board(&mut tx, org, id).await?;
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM project_iterations WHERE board_id = ? AND title = ? COLLATE NOCASE AND id <> ?)",
        )
        .bind(id)
        .bind(input.title.trim())
        .bind(existing.map_or(0, |e| e.0))
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Err(ProjectError::Conflict);
        }
        let result = if let Some((iteration, revision)) = existing {
            let changed = sqlx::query("UPDATE project_iterations SET title = ?, starts_on = ?, ends_on = ?, archived = ?, revision = revision + 1 WHERE board_id = ? AND id = ? AND revision = ?")
                .bind(input.title.trim()).bind(&input.starts_on).bind(&input.ends_on).bind(input.archived).bind(id).bind(iteration).bind(revision).execute(&mut *tx).await?.rows_affected();
            if changed == 0 {
                return Err(ProjectError::Conflict);
            }
            iteration
        } else {
            let count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM project_iterations WHERE board_id = ?").bind(id).fetch_one(&mut *tx).await?;
            if count >= MAX_ITERATIONS {
                return Err(ProjectError::Invalid("iteration limit reached"));
            }
            sqlx::query_scalar("INSERT INTO project_iterations(board_id, title, starts_on, ends_on, archived, created_at) VALUES(?, ?, ?, ?, ?, unixepoch()) RETURNING id")
                .bind(id).bind(input.title.trim()).bind(&input.starts_on).bind(&input.ends_on).bind(input.archived).fetch_one(&mut *tx).await?
        };
        sqlx::query("UPDATE project_boards SET revision = revision + 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn delete_iteration(&self, org: i64, actor: i64, id: i64, iteration: i64, revision: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, true).await?;
        board(&mut tx, org, id).await?;
        let used: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_card_planning WHERE board_id = ? AND iteration_id = ?)")
            .bind(id)
            .bind(iteration)
            .fetch_one(&mut *tx)
            .await?;
        if used {
            return Err(ProjectError::Invalid("unassign cards or archive this iteration instead"));
        }
        let changed = sqlx::query("DELETE FROM project_iterations WHERE board_id = ? AND id = ? AND revision = ?")
            .bind(id)
            .bind(iteration)
            .bind(revision)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            return Err(ProjectError::Conflict);
        }
        sqlx::query("UPDATE project_boards SET revision = revision + 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
