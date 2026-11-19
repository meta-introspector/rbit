//! One bounded batch joins both visible endpoints before exposing any links or progress.
use super::{
    RelationError, RelationStore, Result,
    authority::{active, endpoint, writable},
};
use crate::{
    StorageError,
    sqlite::{pool::WriteTransaction, repositories::readable_by},
};
use rbit_collaboration::issue_relations::{MAX_BATCH, RelatedIssue, Relations};
use std::collections::BTreeMap;

#[derive(sqlx::FromRow)]
struct Row {
    requested: i64,
    id: i64,
    issue_id: i64,
    owner: String,
    repository: String,
    number: i64,
    title: String,
    state: String,
    issue_kind: String,
    relationship: String,
    writable: bool,
    completed: bool,
    source_open: bool,
}
impl RelationStore {
    pub async fn get(&self, actor: Option<i64>, issue: i64) -> Result<Relations> {
        let mut tx = self.pool.begin().await?;
        let source = endpoint(&mut tx, issue, actor).await?;
        let mut data = batch(&mut tx, actor, &[issue]).await?.remove(&issue).unwrap_or_default();
        data.can_edit = source.writable;
        for item in &mut data.items {
            item.can_remove &= source.writable;
        }
        Ok(data)
    }

    pub async fn batch(&self, actor: Option<i64>, issues: &[i64]) -> Result<BTreeMap<i64, Relations>> {
        let mut tx = self.pool.begin().await?;
        batch(&mut tx, actor, issues).await
    }
}

pub(crate) async fn batch(tx: &mut WriteTransaction, actor: Option<i64>, issues: &[i64]) -> Result<BTreeMap<i64, Relations>> {
    if issues.len() > MAX_BATCH {
        return Err(RelationError::Invalid("too many issues"));
    }
    let ids = serde_json::to_string(issues).map_err(|error| StorageError::Corrupt(error.to_string()))?;
    // Only ids selected from visible sources enter the join. Visibility on the related
    // repository is evaluated independently, so neither rows nor totals disclose it.
    let rows: Vec<Row> = sqlx::query_as(concat!(
        "WITH requested AS (SELECT i.id, i.repo_id, i.state, r.owner_id, ", writable!(), " AS writable FROM json_each(?1) j JOIN issues i ON i.id = j.value JOIN repositories r ON r.id = i.repo_id WHERE ", active!(), " AND ", readable_by!("?2", "COALESCE((SELECT is_admin FROM user_logins WHERE account_id = ?2), 0)"), ") SELECT q.id AS requested, e.id, i.id AS issue_id, a.name AS owner, r.name AS repository, i.number, i.title, i.state, i.kind AS issue_kind, CASE WHEN e.source_id = q.id THEN e.kind WHEN e.kind = 'child' THEN 'parent' ELSE 'blocking' END AS relationship, ", writable!(), " AND q.writable AS writable, q.state = 'open' AS source_open, (i.state = 'closed' AND COALESCE(i.state_reason IN ('completed', 'merged'), 0)) AS completed FROM requested q JOIN issue_relations e ON e.source_id = q.id OR e.target_id = q.id JOIN issues i ON i.id = CASE WHEN e.source_id = q.id THEN e.target_id ELSE e.source_id END JOIN repositories r ON r.id = i.repo_id JOIN accounts a ON a.id = r.owner_id WHERE (r.id = q.repo_id OR (a.kind = 'organization' AND r.owner_id = q.owner_id)) AND ", readable_by!("?2", "COALESCE((SELECT is_admin FROM user_logins WHERE account_id = ?2), 0)"), " ORDER BY q.id, e.id"))
        .bind(ids).bind(actor).fetch_all(&mut **tx).await?;
    let mut result: BTreeMap<i64, Relations> = BTreeMap::new();
    for row in rows {
        let data = result.entry(row.requested).or_default();
        if row.relationship == "child" {
            data.progress.total += 1;
            data.progress.completed += u32::from(row.completed);
        }
        if row.relationship == "blocked_by" && row.state == "open" && row.source_open {
            data.progress.blocked = true;
        }
        data.items.push(RelatedIssue {
            id: row.id,
            issue_id: row.issue_id,
            owner: row.owner,
            repository: row.repository,
            number: row.number,
            title: row.title,
            state: row.state,
            issue_kind: row.issue_kind,
            relationship: row.relationship,
            can_remove: row.writable,
        });
    }
    Ok(result)
}
