//! Personal and shared views hide repository-bound criteria when access is lost.
use super::{ProjectError, ProjectStore, Result, StorageError, WriteTransaction, authority, board, readable_by};
use rbit_collaboration::projects::{Filter, MAX_VIEWS, SavedView, ViewInput};

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    owner_id: Option<i64>,
    title: String,
    repository: Option<String>,
    criteria_json: String,
    revision: i64,
}
pub(super) async fn list(tx: &mut WriteTransaction, org: i64, actor: i64, id: i64, manage: bool) -> Result<Vec<SavedView>> {
    let rows: Vec<Row> = sqlx::query_as(concat!("SELECT v.id, v.owner_id, v.title, r.name AS repository, v.criteria_json, v.revision FROM project_views v LEFT JOIN repositories r ON r.id = v.repository_id WHERE v.board_id = ?3 AND (v.owner_id IS NULL OR v.owner_id = ?2) AND (v.repository_id IS NULL OR (r.owner_id = ?1 AND ", readable_by!("?2", "(SELECT is_admin FROM user_logins WHERE account_id = ?2)"), ")) ORDER BY v.id LIMIT ?4"))
        .bind(org).bind(actor).bind(id).bind(MAX_VIEWS * 2).fetch_all(&mut **tx).await?;
    rows.into_iter()
        .map(|row| {
            let mut filter: Filter = serde_json::from_str(&row.criteria_json).map_err(|e| StorageError::Corrupt(e.to_string()))?;
            filter.repository = row.repository.unwrap_or_default();
            filter.validate().map_err(ProjectError::Invalid)?;
            Ok(SavedView {
                id: row.id,
                title: row.title,
                shared: row.owner_id.is_none(),
                revision: row.revision,
                filter,
                can_edit: row.owner_id == Some(actor) || manage,
            })
        })
        .collect()
}
async fn repository(tx: &mut WriteTransaction, org: i64, actor: i64, name: &str) -> Result<Option<i64>> {
    if name.is_empty() {
        return Ok(None);
    }
    let id: Option<i64> = sqlx::query_scalar(concat!(
        "SELECT r.id FROM repositories r WHERE r.owner_id = ?1 AND r.name = ?3 COLLATE NOCASE AND ",
        readable_by!("?2", "(SELECT is_admin FROM user_logins WHERE account_id = ?2)")
    ))
    .bind(org)
    .bind(actor)
    .bind(name)
    .fetch_optional(&mut **tx)
    .await?;
    id.map(Some).ok_or(ProjectError::NotFound)
}
impl ProjectStore {
    pub async fn views(&self, org: i64, actor: i64, id: i64) -> Result<Vec<SavedView>> {
        let mut tx = self.pool.begin().await?;
        let manage = authority(&mut tx, org, actor, false).await?;
        board(&mut tx, org, id).await?;
        list(&mut tx, org, actor, id, manage).await
    }
    pub async fn save_view(&self, org: i64, actor: i64, id: i64, existing: Option<(i64, i64)>, input: &ViewInput) -> Result<i64> {
        input.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        let manage = authority(&mut tx, org, actor, input.shared).await?;
        let current = board(&mut tx, org, id).await?;
        if input.filter.column.is_some_and(|column| usize::try_from(column).map_or(true, |column| column >= current.columns.len())) {
            return Err(ProjectError::Invalid("unknown column"));
        }
        let repository_id = repository(&mut tx, org, actor, input.filter.repository.trim()).await?;
        let iteration_id = input.filter.iteration.filter(|id| *id > 0);
        if let Some(iteration) = iteration_id {
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM project_iterations WHERE board_id = ? AND id = ?)")
                .bind(id)
                .bind(iteration)
                .fetch_one(&mut *tx)
                .await?;
            if !exists {
                return Err(ProjectError::NotFound);
            }
        }
        let owner = if input.shared { None } else { Some(actor) };
        if let Some((view_id, revision)) = existing {
            let views = list(&mut tx, org, actor, id, manage).await?;
            let old = views.iter().find(|v| v.id == view_id).ok_or(ProjectError::NotFound)?;
            if !old.can_edit {
                return Err(ProjectError::Forbidden);
            }
            if old.revision != revision {
                return Err(ProjectError::Conflict);
            }
            if old.shared != input.shared {
                return Err(ProjectError::Invalid("create a new view to change its sharing scope"));
            }
        }
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM project_views WHERE board_id = ? AND owner_id IS ? AND title = ? COLLATE NOCASE AND id <> ?)",
        )
        .bind(id)
        .bind(owner)
        .bind(input.title.trim())
        .bind(existing.map_or(0, |e| e.0))
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Err(ProjectError::Conflict);
        }
        let mut filter = input.filter.clone();
        filter.repository.clear();
        let json = serde_json::to_string(&filter).map_err(|e| StorageError::Corrupt(e.to_string()))?;
        let result = if let Some((view, revision)) = existing {
            let changed = sqlx::query("UPDATE project_views SET title = ?, repository_id = ?, iteration_id = ?, criteria_json = ?, revision = revision + 1 WHERE board_id = ? AND id = ? AND revision = ?")
                .bind(input.title.trim()).bind(repository_id).bind(iteration_id).bind(json).bind(id).bind(view).bind(revision).execute(&mut *tx).await?.rows_affected();
            if changed == 0 {
                return Err(ProjectError::Conflict);
            }
            view
        } else {
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_views WHERE board_id = ? AND owner_id IS ?")
                .bind(id)
                .bind(owner)
                .fetch_one(&mut *tx)
                .await?;
            if count >= MAX_VIEWS {
                return Err(ProjectError::Invalid("saved view limit reached"));
            }
            sqlx::query_scalar("INSERT INTO project_views(board_id, owner_id, title, repository_id, iteration_id, criteria_json, created_at) VALUES(?, ?, ?, ?, ?, ?, unixepoch()) RETURNING id")
                .bind(id).bind(owner).bind(input.title.trim()).bind(repository_id).bind(iteration_id).bind(json).fetch_one(&mut *tx).await?
        };
        tx.commit().await?;
        Ok(result)
    }
    pub async fn delete_view(&self, org: i64, actor: i64, id: i64, view: i64, revision: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        let manage = authority(&mut tx, org, actor, false).await?;
        board(&mut tx, org, id).await?;
        let views = list(&mut tx, org, actor, id, manage).await?;
        let current = views.iter().find(|v| v.id == view).ok_or(ProjectError::NotFound)?;
        if !current.can_edit {
            return Err(ProjectError::Forbidden);
        }
        if current.revision != revision {
            return Err(ProjectError::Conflict);
        }
        sqlx::query("DELETE FROM project_views WHERE id = ?").bind(view).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
