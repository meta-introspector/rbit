//! Transactional organization boards with live membership and repository visibility checks.

mod iterations;
mod planning;
mod read;
mod views;

use super::pool::WriteTransaction;
use rbit_collaboration::projects::{Board, BoardInput, MAX_BOARDS, MAX_CARDS};

use super::{pool::Pool, repositories::readable_by};
use crate::StorageError;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("not found")]
    NotFound,
    #[error("organization ownership is required")]
    Forbidden,
    #[error("the project changed; reload before trying again")]
    Conflict,
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for ProjectError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.into())
    }
}

type Result<T> = std::result::Result<T, ProjectError>;

#[derive(Debug, Clone)]
pub struct ProjectStore {
    pool: Pool,
}

#[derive(sqlx::FromRow)]
struct BoardRow {
    id: i64,
    org_id: i64,
    title: String,
    columns_json: String,
    revision: i64,
}

impl TryFrom<BoardRow> for Board {
    type Error = ProjectError;
    fn try_from(row: BoardRow) -> Result<Self> {
        let columns = serde_json::from_str(&row.columns_json).map_err(|error| StorageError::Corrupt(error.to_string()))?;
        Ok(Self { id: row.id, org_id: row.org_id, title: row.title, columns, revision: row.revision })
    }
}

/// Read authorization is in the same snapshot as returned records; mutations hold the
/// write lock while rechecking membership, active identity, repository scope and revisions.
async fn authority(tx: &mut WriteTransaction, org: i64, actor: i64, manage: bool) -> Result<bool> {
    let row: Option<(bool, bool)> = sqlx::query_as("SELECT u.is_admin, COALESCE(m.role = 'owner', 0) FROM user_logins u JOIN accounts a ON a.id = ?1 AND a.kind = 'organization' LEFT JOIN org_members m ON m.org_id = a.id AND m.user_id = u.account_id WHERE u.account_id = ?2 AND u.suspended_at IS NULL AND (u.is_admin OR m.user_id IS NOT NULL)")
        .bind(org).bind(actor).fetch_optional(&mut **tx).await?;
    let (admin, owner) = row.ok_or(ProjectError::NotFound)?;
    if manage && !admin && !owner {
        return Err(ProjectError::Forbidden);
    }
    Ok(admin || owner)
}

async fn board(tx: &mut WriteTransaction, org: i64, id: i64) -> Result<Board> {
    let row: BoardRow = sqlx::query_as("SELECT id, org_id, title, columns_json, revision FROM project_boards WHERE org_id = ? AND id = ?")
        .bind(org)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ProjectError::NotFound)?;
    row.try_into()
}

async fn visible_issue(tx: &mut WriteTransaction, org: i64, actor: i64, issue: i64) -> Result<()> {
    let visible: bool = sqlx::query_scalar(concat!(
        "SELECT EXISTS(SELECT 1 FROM issues i JOIN repositories r ON r.id = i.repo_id WHERE r.owner_id = ?1 AND i.id = ?3 AND ",
        readable_by!("?2", "(SELECT is_admin FROM user_logins WHERE account_id = ?2)"),
        ")"
    ))
    .bind(org)
    .bind(actor)
    .bind(issue)
    .fetch_one(&mut **tx)
    .await?;
    if visible { Ok(()) } else { Err(ProjectError::NotFound) }
}

impl ProjectStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, org: i64, actor: i64) -> Result<Vec<Board>> {
        let mut tx = self.pool.begin().await?;
        authority(&mut tx, org, actor, false).await?;
        let rows: Vec<BoardRow> =
            sqlx::query_as("SELECT id, org_id, title, columns_json, revision FROM project_boards WHERE org_id = ? ORDER BY id LIMIT ?")
                .bind(org)
                .bind(MAX_BOARDS)
                .fetch_all(&mut *tx)
                .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn create(&self, org: i64, actor: i64, input: &BoardInput) -> Result<Board> {
        input.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, true).await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_boards WHERE org_id = ?").bind(org).fetch_one(&mut *tx).await?;
        if count >= MAX_BOARDS {
            return Err(ProjectError::Invalid("organization board limit reached"));
        }
        let columns = serde_json::to_string(&input.columns).map_err(|error| StorageError::Corrupt(error.to_string()))?;
        let row: BoardRow = sqlx::query_as("INSERT INTO project_boards(org_id, title, columns_json, created_at) VALUES(?, ?, ?, unixepoch()) RETURNING id, org_id, title, columns_json, revision")
            .bind(org).bind(input.title.trim()).bind(columns).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        row.try_into()
    }

    /// Column identities are their positions. Existing cards retain their position;
    /// shrinking is refused while a card occupies a removed column.
    pub async fn update(&self, org: i64, actor: i64, id: i64, revision: i64, input: &BoardInput) -> Result<()> {
        input.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, true).await?;
        let current = board(&mut tx, org, id).await?;
        if current.revision != revision {
            return Err(ProjectError::Conflict);
        }
        let length = i64::try_from(input.columns.len()).map_err(|_| ProjectError::Invalid("too many columns"))?;
        let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_cards WHERE board_id = ? AND column_index >= ?)")
            .bind(id)
            .bind(length)
            .fetch_one(&mut *tx)
            .await?;
        if occupied {
            return Err(ProjectError::Invalid("move cards out of removed columns first"));
        }
        let columns = serde_json::to_string(&input.columns).map_err(|error| StorageError::Corrupt(error.to_string()))?;
        sqlx::query("UPDATE project_boards SET title = ?, columns_json = ?, revision = revision + 1 WHERE id = ?")
            .bind(input.title.trim())
            .bind(columns)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn delete(&self, org: i64, actor: i64, id: i64, revision: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, true).await?;
        if board(&mut tx, org, id).await?.revision != revision {
            return Err(ProjectError::Conflict);
        }
        sqlx::query("DELETE FROM project_boards WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn resolve_issue(&self, org: i64, actor: i64, repository: &str, number: i64) -> Result<i64> {
        let mut tx = self.pool.begin().await?;
        authority(&mut tx, org, actor, false).await?;
        let issue: i64 = sqlx::query_scalar("SELECT i.id FROM issues i JOIN repositories r ON r.id = i.repo_id WHERE r.owner_id = ? AND r.name = ? COLLATE NOCASE AND i.number = ?")
            .bind(org).bind(repository).bind(number).fetch_optional(&mut *tx).await?.ok_or(ProjectError::NotFound)?;
        visible_issue(&mut tx, org, actor, issue).await?;
        Ok(issue)
    }

    /// Adding an existing card is idempotent and never changes its placement.
    pub async fn add(&self, org: i64, actor: i64, id: i64, issue: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, false).await?;
        board(&mut tx, org, id).await?;
        visible_issue(&mut tx, org, actor, issue).await?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_cards WHERE board_id = ? AND issue_id = ?)")
            .bind(id)
            .bind(issue)
            .fetch_one(&mut *tx)
            .await?;
        if exists {
            return Ok(());
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_cards WHERE board_id = ?").bind(id).fetch_one(&mut *tx).await?;
        if count >= MAX_CARDS {
            return Err(ProjectError::Invalid("board card limit reached"));
        }
        sqlx::query("INSERT OR IGNORE INTO project_cards(board_id, issue_id, column_index, revision) SELECT id, ?, 0, revision + 1 FROM project_boards WHERE id = ?")
            .bind(issue)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE project_boards SET revision = revision + 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// `None` removes the card; a column moves it. Revision checks prevent lost updates.
    pub async fn move_card(&self, org: i64, actor: i64, id: i64, issue: i64, revision: i64, column: Option<i64>) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        authority(&mut tx, org, actor, false).await?;
        let current = board(&mut tx, org, id).await?;
        visible_issue(&mut tx, org, actor, issue).await?;
        let changed = if let Some(column) = column {
            let completed: bool = sqlx::query_scalar(
                "SELECT state = 'closed' AND COALESCE(state_reason IN ('completed', 'merged'), 0) FROM issues WHERE id = ?",
            )
            .bind(issue)
            .fetch_one(&mut *tx)
            .await?;
            if completed {
                return Err(ProjectError::Invalid("completed work follows its issue state; reopen the issue to move it"));
            }

            if column < 0 || usize::try_from(column).map_or(true, |index| index >= current.columns.len()) {
                return Err(ProjectError::Invalid("unknown column"));
            }
            sqlx::query(
                "UPDATE project_cards SET column_index = ?, revision = (SELECT revision + 1 FROM project_boards WHERE id = project_cards.board_id) WHERE board_id = ? AND issue_id = ? AND revision = ?",
            )
            .bind(column)
            .bind(id)
            .bind(issue)
            .bind(revision)
            .execute(&mut *tx)
            .await?
            .rows_affected()
        } else {
            sqlx::query("DELETE FROM project_cards WHERE board_id = ? AND issue_id = ? AND revision = ?")
                .bind(id)
                .bind(issue)
                .bind(revision)
                .execute(&mut *tx)
                .await?
                .rows_affected()
        };
        if changed == 0 {
            return Err(ProjectError::Conflict);
        }
        sqlx::query("UPDATE project_boards SET revision = revision + 1 WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
