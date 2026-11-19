//! Live repository visibility and write grants, checked inside the relationship transaction.
use super::{RelationError, Result};
use crate::sqlite::{pool::WriteTransaction, repositories::readable_by};

macro_rules! writable {
    () => { "(?2 IS NOT NULL AND NOT EXISTS(SELECT 1 FROM repo_archives a WHERE a.repo_id = r.id) AND EXISTS(SELECT 1 FROM user_logins u WHERE u.account_id = ?2 AND u.suspended_at IS NULL AND (u.is_admin OR r.owner_id = u.account_id OR EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id AND c.permission IN ('write', 'admin')) OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id AND (m.role = 'owner' OR COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') = 'write' OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id WHERE t.org_id = r.owner_id AND tm.user_id = u.account_id AND tr.repo_id = r.id AND tr.permission IN ('write', 'admin')))))))" };
}
pub(super) use writable;
macro_rules! active {
    () => {
        "(?2 IS NULL OR EXISTS(SELECT 1 FROM user_logins WHERE account_id = ?2 AND suspended_at IS NULL))"
    };
}
pub(super) use active;

#[derive(sqlx::FromRow)]
pub(super) struct Endpoint {
    pub repo: i64,
    pub owner: i64,
    pub organization: bool,
    pub writable: bool,
}

pub(super) async fn endpoint(tx: &mut WriteTransaction, issue: i64, actor: Option<i64>) -> Result<Endpoint> {
    sqlx::query_as(concat!(
        "SELECT r.id AS repo, r.owner_id AS owner, a.kind = 'organization' AS organization, ",
        writable!(),
        " AS writable FROM issues i JOIN repositories r ON r.id = i.repo_id JOIN accounts a ON a.id = r.owner_id WHERE i.id = ?1 AND ",
        active!(),
        " AND ",
        readable_by!("?2", "COALESCE((SELECT is_admin FROM user_logins WHERE account_id = ?2), 0)")
    ))
    .bind(issue)
    .bind(actor)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(RelationError::NotFound)
}

pub(super) fn require_pair(source: &Endpoint, target: &Endpoint) -> Result<()> {
    if source.repo != target.repo && !(source.organization && source.owner == target.owner) {
        return Err(RelationError::Invalid("relationships require one repository or one organization"));
    }
    if !source.writable || !target.writable {
        return Err(RelationError::Forbidden);
    }
    Ok(())
}
