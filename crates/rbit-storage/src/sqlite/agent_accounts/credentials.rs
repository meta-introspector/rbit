//! Link only owner-issued bounded credentials; revocation removes the underlying token.

use rbit_core::{AccessToken, AccessTokenId};
use rbit_identity_models::agents::MAX_CREDENTIALS;

use super::{AccountId, AgentAccountStore, AgentError, Result, Timestamp, WriteTransaction, owned};
use crate::sqlite::rows::AccessTokenRow;

pub(super) async fn revoke_all(tx: &mut WriteTransaction, agent: AccountId) -> Result<()> {
    sqlx::query("DELETE FROM access_tokens WHERE account_id = ?").bind(agent.0).execute(&mut **tx).await?;
    Ok(())
}

impl AgentAccountStore {
    /// A generated token is unusable as an agent credential until this owner fence succeeds.
    pub async fn link_credential(&self, actor: AccountId, agent: AccountId, revision: i64, token: AccessTokenId) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        let current = owned(&mut tx, actor, agent).await?;
        if !current.enabled || current.revision != revision {
            return Err(AgentError::Conflict);
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_credentials WHERE agent_id = ?").bind(agent.0).fetch_one(&mut *tx).await?;
        if count >= MAX_CREDENTIALS {
            return Err(AgentError::Invalid("an agent may have at most ten credentials"));
        }
        let result = sqlx::query("INSERT INTO agent_credentials(token_id, agent_id, created_at) SELECT id, account_id, ? FROM access_tokens WHERE id = ? AND account_id = ? AND scope IN ('read','write')")
            .bind(Timestamp::now().0).bind(token.0).bind(agent.0).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(AgentError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn credential_linked(&self, agent: AccountId, token: AccessTokenId) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM agent_credentials c JOIN access_tokens t ON t.id = c.token_id WHERE c.agent_id = ? AND c.token_id = ? AND t.account_id = c.agent_id)")
            .bind(agent.0).bind(token.0).fetch_one(&self.pool).await?)
    }

    pub async fn credentials(&self, actor: AccountId, agent: AccountId) -> Result<Vec<AccessToken>> {
        let mut tx = self.pool.begin().await?;
        owned(&mut tx, actor, agent).await?;
        let rows: Vec<AccessTokenRow> = sqlx::query_as("SELECT t.id, t.account_id, t.name, t.hint, t.scope, t.created_at, t.last_used_at, t.expires_at FROM access_tokens t JOIN agent_credentials c ON c.token_id = t.id WHERE c.agent_id = ? AND t.account_id = c.agent_id ORDER BY t.id LIMIT 10")
            .bind(agent.0).fetch_all(&mut *tx).await?;
        rows.into_iter().map(|row| AccessToken::try_from(row).map_err(Into::into)).collect()
    }

    pub async fn revoke_credential(&self, actor: AccountId, agent: AccountId, token: AccessTokenId) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        owned(&mut tx, actor, agent).await?;
        let result = sqlx::query("DELETE FROM access_tokens WHERE id = ? AND account_id = ? AND EXISTS(SELECT 1 FROM agent_credentials WHERE token_id = ? AND agent_id = ?)")
            .bind(token.0).bind(agent.0).bind(token.0).bind(agent.0).execute(&mut *tx).await?;
        if result.rows_affected() != 1 {
            return Err(AgentError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }
}
