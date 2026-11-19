//! Bounded repository tag protection rules; insertion limits are checked atomically.
use super::pool::Pool;
use crate::StorageError;
use rbit_access::{protection::BranchPattern, tag_protection::MAX_TAG_RULES};
use rbit_core::RepoId;

#[derive(Debug, Clone)]
pub struct TagProtectionStore {
    pool: Pool,
}
impl TagProtectionStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, repo: RepoId) -> Result<Vec<(i64, BranchPattern)>, StorageError> {
        let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, pattern FROM tag_protections WHERE repo_id = ? ORDER BY pattern")
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await?;
        rows.into_iter()
            .map(|(id, pattern)| Ok((id, BranchPattern::parse(&pattern).map_err(|e| StorageError::Corrupt(e.to_string()))?)))
            .collect()
    }
    pub async fn create(&self, repo: RepoId, pattern: &BranchPattern) -> Result<Option<i64>, StorageError> {
        sqlx::query_scalar("INSERT INTO tag_protections (repo_id, pattern) SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM tag_protections WHERE repo_id = ?1) < ?3 RETURNING id")
            .bind(repo.0).bind(pattern.as_str()).bind(MAX_TAG_RULES).fetch_optional(&self.pool).await
            .map_err(|e| StorageError::from_insert(e, "pattern"))
    }
    pub async fn delete(&self, repo: RepoId, id: i64) -> Result<bool, StorageError> {
        Ok(sqlx::query("DELETE FROM tag_protections WHERE repo_id = ? AND id = ?")
            .bind(repo.0)
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected()
            > 0)
    }
}

/// Recheck release identity, current tag rules and live publisher grants in the same
/// write transaction that publishes a completed upload. A rule added during streaming
/// or a release retargeted during streaming therefore cannot be bypassed.
pub(crate) async fn authorize_asset(
    tx: &mut super::pool::WriteTransaction,
    new: &rbit_core::release::NewAsset<'_>,
    admitted_admin: bool,
) -> Result<(), StorageError> {
    let tag: String = sqlx::query_scalar("SELECT tag_name FROM releases WHERE id = ? AND repo_id = ?")
        .bind(new.release_id.0)
        .bind(new.repo_id.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(StorageError::ProtectedRelease)?;
    authorize_publication(
        tx,
        new.repo_id,
        &tag,
        new.uploader_id.filter(|_| new.source == rbit_core::release::ReleaseSource::User),
        admitted_admin,
    )
    .await
}

/// Evaluate current protection and live grants while the caller holds the write lock.
pub(crate) async fn authorize_publication(
    tx: &mut super::pool::WriteTransaction,
    repo: RepoId,
    tag: &str,
    user: Option<rbit_core::AccountId>,
    admitted_admin: bool,
) -> Result<(), StorageError> {
    use rbit_access::{RepoAccess, tag_protection};
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT pattern FROM tag_protections WHERE repo_id = ?").bind(repo.0).fetch_all(&mut **tx).await?;
    let patterns: Vec<BranchPattern> = rows
        .iter()
        .map(|pattern| BranchPattern::parse(pattern).map_err(|error| StorageError::Corrupt(error.to_string())))
        .collect::<Result<_, _>>()?;
    if tag_protection::matching(&patterns, tag).is_none() {
        return Ok(());
    }
    if !admitted_admin {
        return Err(StorageError::ProtectedRelease);
    }
    let admin: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repositories r JOIN user_logins u ON u.account_id = ?2 WHERE r.id = ?1 AND u.suspended_at IS NULL AND NOT EXISTS(SELECT 1 FROM repo_archives a WHERE a.repo_id = r.id) AND (u.is_admin OR r.owner_id = u.account_id OR EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id AND c.permission = 'admin') OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id AND (m.role = 'owner' OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id WHERE t.org_id = r.owner_id AND tm.user_id = u.account_id AND tr.repo_id = r.id AND tr.permission = 'admin')))))")
        .bind(repo.0).bind(user.map(|id| id.0)).fetch_one(&mut **tx).await?;
    let access = if admin { RepoAccess::Admin } else { RepoAccess::None };
    if tag_protection::may_publish(&patterns, tag, access, user.is_some()) { Ok(()) } else { Err(StorageError::ProtectedRelease) }
}
