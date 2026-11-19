//! One-statement live grants for individual and batched repository viewers.
use super::{AccountId, GRANT_QUERY_BATCH_SIZE, GrantsRow, HashMap, RepoId, RepositoryStore, StorageError, ViewerGrants, parse_grants};

impl RepositoryStore {
    /// Everything `viewer` was granted on repository `repo` (owned by `owner`), in one query.
    pub async fn grants(&self, repo: RepoId, owner: AccountId, viewer: AccountId) -> Result<ViewerGrants, StorageError> {
        let (role, base, team, collaborator): (Option<String>, Option<String>, Option<i64>, Option<String>) = sqlx::query_as(concat!(
            "SELECT (SELECT role FROM org_members WHERE org_id = ?1 AND user_id = ?3), \
             (SELECT member_permission FROM org_profiles WHERE account_id = ?1), \
             (SELECT MAX(CASE tr.permission WHEN 'admin' THEN 3 WHEN 'write' THEN 2 ELSE 1 END) \
              FROM team_repositories tr JOIN team_members tm ON tm.team_id = tr.team_id JOIN teams t ON t.id = tr.team_id \
              WHERE tr.repo_id = ?2 AND t.org_id = ?1 AND tm.user_id = ?3), \
             ",
            scoped_collaborator!("?2", "?3")
        ))
        .bind(owner.0)
        .bind(repo.0)
        .bind(viewer.0)
        .fetch_one(&self.pool)
        .await?;
        parse_grants(role.as_deref(), base.as_deref(), team, collaborator.as_deref())
    }

    /// Fresh grants for all requested users, deduplicated and queried in bounded batches.
    /// Users with no grants still have an entry, with the owner's default base permission.
    pub async fn grants_for(
        &self,
        repo: RepoId,
        owner: AccountId,
        users: &[AccountId],
    ) -> Result<HashMap<AccountId, ViewerGrants>, StorageError> {
        let mut ids: Vec<i64> = users.iter().map(|id| id.0).collect();
        ids.sort_unstable();
        ids.dedup();
        let mut grants = HashMap::with_capacity(ids.len());
        for batch in ids.chunks(GRANT_QUERY_BATCH_SIZE) {
            let rows: Vec<GrantsRow> = sqlx::query_as(concat!(
                "SELECT v.value AS user_id, \
                 (SELECT role FROM org_members WHERE org_id = ?1 AND user_id = v.value) AS role, \
                 (SELECT member_permission FROM org_profiles WHERE account_id = ?1) AS base, \
                 (SELECT MAX(CASE tr.permission WHEN 'admin' THEN 3 WHEN 'write' THEN 2 ELSE 1 END) \
                  FROM team_repositories tr JOIN team_members tm ON tm.team_id = tr.team_id JOIN teams t ON t.id = tr.team_id \
                  WHERE tr.repo_id = ?2 AND t.org_id = ?1 AND tm.user_id = v.value) AS team, \
                 ",
                scoped_collaborator!("?2", "v.value"),
                " AS collaborator \
                 FROM json_each(?3) v"
            ))
            .bind(owner.0)
            .bind(repo.0)
            .bind(format!("{batch:?}"))
            .fetch_all(&self.pool)
            .await?;
            for row in rows {
                grants.insert(
                    AccountId(row.user_id),
                    parse_grants(row.role.as_deref(), row.base.as_deref(), row.team, row.collaborator.as_deref())?,
                );
            }
        }
        Ok(grants)
    }
}
