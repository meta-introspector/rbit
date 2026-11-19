//! Scope replacement and effective rights always use current ownership and membership.

use rbit_identity_models::agents::{AgentAccess, AgentScope, MAX_SCOPES};

use super::{AccountId, AgentAccountStore, AgentError, Result, WriteTransaction, owned};

const REPOSITORY_PERMISSION: &str = concat!("SELECT ", super::queries::repository_permission_sql!("?1", "?2"));

pub(super) async fn replace(tx: &mut WriteTransaction, actor: AccountId, agent: AccountId, scopes: &[AgentScope]) -> Result<()> {
    if scopes.is_empty() || scopes.len() > MAX_SCOPES {
        return Err(AgentError::Invalid("select one to 32 scopes"));
    }
    sqlx::query("DELETE FROM agent_scopes WHERE agent_id = ?").bind(agent.0).execute(&mut **tx).await?;
    for scope in scopes {
        let (repo, org) = match *scope {
            AgentScope::Repository { id, .. } => (Some(id), None),
            AgentScope::Organization { id, .. } => {
                let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accounts a JOIN org_members m ON m.org_id = a.id WHERE a.id = ? AND a.kind = 'organization' AND m.user_id = ? AND m.role = 'owner' AND NOT EXISTS(SELECT 1 FROM org_agent_policies p WHERE p.org_id = a.id AND p.allow_agents = 0))")
                    .bind(id).bind(actor.0).fetch_one(&mut **tx).await?;
                if !allowed {
                    return Err(AgentError::NotFound);
                }
                (None, Some(id))
            }
        };
        if let Some(repo) = repo {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM repositories WHERE id = ?)").bind(repo).fetch_one(&mut **tx).await?;
            if !exists {
                return Err(AgentError::NotFound);
            }
        }
        sqlx::query("INSERT INTO agent_scopes(agent_id, repo_id, org_id, permission) VALUES (?, ?, ?, ?)")
            .bind(agent.0)
            .bind(repo)
            .bind(org)
            .bind(scope.access().level())
            .execute(&mut **tx)
            .await?;
        if let Some(repo) = repo {
            let allowed: Option<i64> = sqlx::query_scalar(REPOSITORY_PERMISSION).bind(agent.0).bind(repo).fetch_one(&mut **tx).await?;
            if allowed.unwrap_or(0) < scope.access().level() {
                return Err(AgentError::NotFound);
            }
        }
    }
    Ok(())
}

impl AgentAccountStore {
    pub async fn scopes(&self, actor: AccountId, agent: AccountId) -> Result<Vec<AgentScope>> {
        let mut tx = self.pool.begin().await?;
        owned(&mut tx, actor, agent).await?;
        let rows: Vec<(Option<i64>, Option<i64>, i64)> =
            sqlx::query_as("SELECT repo_id, org_id, permission FROM agent_scopes WHERE agent_id = ? ORDER BY id LIMIT 32")
                .bind(agent.0)
                .fetch_all(&mut *tx)
                .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(repo, org, level)| {
                let access = if level == 2 { AgentAccess::Write } else { AgentAccess::Read };
                repo.map(|id| AgentScope::Repository { id, access }).or_else(|| org.map(|id| AgentScope::Organization { id, access }))
            })
            .collect())
    }

    /// The live ceiling of all matching scopes; no scope means no access, even to public repositories.
    pub async fn repository_permission(&self, agent: AccountId, repo: rbit_core::RepoId) -> Result<Option<AgentAccess>> {
        let level: Option<i64> = sqlx::query_scalar(REPOSITORY_PERMISSION).bind(agent.0).bind(repo.0).fetch_one(&self.pool).await?;
        Ok(level.map(|value| if value == 2 { AgentAccess::Write } else { AgentAccess::Read }))
    }

    pub async fn organization_policy(&self, actor: AccountId, org: AccountId) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        organization_owner(&mut tx, actor, org).await?;
        Ok(sqlx::query_scalar("SELECT COALESCE((SELECT allow_agents FROM org_agent_policies WHERE org_id = ?), 1)")
            .bind(org.0)
            .fetch_one(&mut *tx)
            .await?)
    }

    pub async fn set_organization_policy(&self, actor: AccountId, org: AccountId, allow: bool) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        organization_owner(&mut tx, actor, org).await?;
        sqlx::query("INSERT INTO org_agent_policies(org_id, allow_agents) VALUES (?, ?) ON CONFLICT(org_id) DO UPDATE SET allow_agents = excluded.allow_agents")
            .bind(org.0).bind(allow).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}

async fn organization_owner(tx: &mut WriteTransaction, actor: AccountId, org: AccountId) -> Result<()> {
    super::owner(tx, actor).await?;
    let allowed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accounts a JOIN org_members m ON m.org_id = a.id WHERE a.id = ? AND a.kind = 'organization' AND m.user_id = ? AND m.role = 'owner')")
        .bind(org.0).bind(actor.0).fetch_one(&mut **tx).await?;
    if allowed { Ok(()) } else { Err(AgentError::NotFound) }
}

#[cfg(test)]
mod query_tests;
