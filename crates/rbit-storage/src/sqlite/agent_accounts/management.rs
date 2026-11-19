//! Stable management identifiers and atomic replacement of linked agent credentials.

use rbit_core::{AccessToken, AccessTokenId};
use rbit_identity_models::agents::{AgentAccess, AgentScope, MAX_CREDENTIALS};

use super::{AccountId, AgentAccountStore, AgentError, Result, Timestamp, WriteTransaction, owned};
use crate::sqlite::{repositories::readable_by, rows::AccessTokenRow};

#[derive(Debug, Clone)]
pub struct AgentCredential {
    pub id: i64,
    pub token: AccessToken,
}

type ScopeNameRow = (Option<i64>, Option<i64>, i64, Option<String>);

#[derive(sqlx::FromRow)]
struct CredentialRow {
    credential_id: i64,
    #[sqlx(flatten)]
    token: AccessTokenRow,
}

impl TryFrom<CredentialRow> for AgentCredential {
    type Error = AgentError;
    fn try_from(row: CredentialRow) -> Result<Self> {
        Ok(Self { id: row.credential_id, token: row.token.try_into()? })
    }
}

macro_rules! select_credential {
    ($tail:literal) => { concat!("SELECT c.id AS credential_id, t.id, t.account_id, t.name, t.hint, t.scope, t.created_at, t.last_used_at, t.expires_at FROM agent_credentials c JOIN access_tokens t ON t.id = c.token_id AND t.account_id = c.agent_id ", $tail) };
}

impl AgentAccountStore {
    pub async fn managed_credentials(&self, actor: AccountId, agent: AccountId) -> Result<Vec<AgentCredential>> {
        let mut tx = self.pool.begin().await?;
        owned(&mut tx, actor, agent).await?;
        let rows: Vec<CredentialRow> =
            sqlx::query_as(select_credential!("WHERE c.agent_id = ? ORDER BY c.id LIMIT 10")).bind(agent.0).fetch_all(&mut *tx).await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    /// A new candidate is unusable until this final owner/revision fence links it.
    /// Replacement removes the old digest and creates a fresh stable management id
    /// atomically, including when all ten credential slots are occupied.
    pub async fn link_managed_credential(
        &self,
        actor: AccountId,
        agent: AccountId,
        revision: i64,
        candidate: AccessTokenId,
        replace: Option<i64>,
    ) -> Result<AgentCredential> {
        let mut tx = self.pool.begin_immediate().await?;
        let current = owned(&mut tx, actor, agent).await?;
        if !current.enabled || current.revision != revision {
            return Err(AgentError::Conflict);
        }
        if let Some(id) = replace {
            let old = token_for(&mut tx, agent, id).await?;
            if old == candidate.0 {
                return Err(AgentError::Invalid("replacement must be a new credential"));
            }
            sqlx::query("DELETE FROM access_tokens WHERE id = ? AND account_id = ?").bind(old).bind(agent.0).execute(&mut *tx).await?;
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_credentials WHERE agent_id = ?").bind(agent.0).fetch_one(&mut *tx).await?;
        if count >= MAX_CREDENTIALS {
            return Err(AgentError::Invalid("an agent may have at most ten credentials"));
        }
        let id: i64 = sqlx::query_scalar("INSERT INTO agent_credentials(token_id, agent_id, created_at) SELECT id, account_id, ? FROM access_tokens WHERE id = ? AND account_id = ? AND scope IN ('read','write') AND NOT EXISTS(SELECT 1 FROM agent_credentials c WHERE c.token_id = access_tokens.id) RETURNING id")
            .bind(Timestamp::now().0).bind(candidate.0).bind(agent.0).fetch_optional(&mut *tx).await?
            .ok_or(AgentError::NotFound)?;
        let row: CredentialRow =
            sqlx::query_as(select_credential!("WHERE c.id = ? AND c.agent_id = ?")).bind(id).bind(agent.0).fetch_one(&mut *tx).await?;
        let credential = row.try_into()?;
        tx.commit().await?;
        Ok(credential)
    }

    pub async fn revoke_managed_credential(&self, actor: AccountId, agent: AccountId, id: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        owned(&mut tx, actor, agent).await?;
        let token = token_for(&mut tx, agent, id).await?;
        sqlx::query("DELETE FROM access_tokens WHERE id = ? AND account_id = ?").bind(token).bind(agent.0).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Read current scope labels with the human's current access. A disabled agent
    /// remains editable; losing access must not reveal renamed private resources.
    pub async fn scope_names(&self, actor: AccountId, agent: AccountId) -> Result<Vec<(AgentScope, Option<String>)>> {
        let mut tx = self.pool.begin().await?;
        owned(&mut tx, actor, agent).await?;
        let rows: Vec<ScopeNameRow> = sqlx::query_as(concat!(
            "SELECT s.repo_id, s.org_id, s.permission, CASE WHEN s.repo_id IS NOT NULL THEN ",
            "(SELECT a.name || '/' || r.name FROM repositories r JOIN accounts a ON a.id = r.owner_id WHERE r.id = s.repo_id AND ",
            readable_by!("?1", "(SELECT is_admin FROM user_logins WHERE account_id = ?1)"),
            ") ELSE (SELECT a.name FROM accounts a JOIN org_members m ON m.org_id = a.id WHERE a.id = s.org_id AND a.kind = 'organization' AND m.user_id = ?1) END ",
            "FROM agent_scopes s WHERE s.agent_id = ?2 ORDER BY s.id LIMIT 32"
        )).bind(actor.0).bind(agent.0).fetch_all(&mut *tx).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(repo, org, level, name)| {
                let access = if level == 2 { AgentAccess::Write } else { AgentAccess::Read };
                let scope =
                    repo.map(|id| AgentScope::Repository { id, access }).or_else(|| org.map(|id| AgentScope::Organization { id, access }));
                scope.map(|scope| (scope, name))
            })
            .collect())
    }
}

async fn token_for(tx: &mut WriteTransaction, agent: AccountId, id: i64) -> Result<i64> {
    sqlx::query_scalar("SELECT c.token_id FROM agent_credentials c JOIN access_tokens t ON t.id = c.token_id AND t.account_id = c.agent_id WHERE c.id = ? AND c.agent_id = ?")
        .bind(id).bind(agent.0).fetch_optional(&mut **tx).await?.ok_or(AgentError::NotFound)
}
