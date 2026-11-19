//! Bounded human-owned agent identities and live delegation fences.

use rbit_core::{AccountId, Timestamp};
use rbit_identity_models::agents::{Agent, AgentInput, MAX_AGENTS};

use super::{
    accounts::INSERT_UNRESERVED_ACCOUNT,
    pool::{Pool, WriteTransaction},
};
use crate::StorageError;

mod credentials;
mod management;
pub use management::AgentCredential;
pub(crate) mod queries;
mod scopes;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("agent or scope not found")]
    NotFound,
    #[error("agent owner permission is required")]
    Forbidden,
    #[error("the agent changed; reload before trying again")]
    Conflict,
    #[error("{0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for AgentError {
    fn from(error: sqlx::Error) -> Self {
        Self::Storage(error.into())
    }
}
type Result<T> = std::result::Result<T, AgentError>;

#[derive(Debug, Clone)]
pub struct AgentAccountStore {
    pool: Pool,
}

#[derive(Debug, sqlx::FromRow)]
struct AgentRow {
    account_id: i64,
    owner_id: i64,
    handle: String,
    slug: String,
    name: String,
    enabled: bool,
    revision: i64,
    capabilities_json: String,
    created_at: i64,
}

impl TryFrom<AgentRow> for Agent {
    type Error = AgentError;
    fn try_from(row: AgentRow) -> Result<Self> {
        Ok(Self {
            account_id: AccountId(row.account_id),
            owner_id: AccountId(row.owner_id),
            handle: row.handle,
            slug: row.slug,
            name: row.name,
            enabled: row.enabled,
            revision: row.revision,
            capabilities: serde_json::from_str(&row.capabilities_json).map_err(|e| StorageError::Corrupt(e.to_string()))?,
            created_at: Timestamp(row.created_at),
        })
    }
}

macro_rules! select_agent {
    ($tail:literal) => { concat!("SELECT a.account_id, a.owner_id, u.name AS handle, a.slug, a.name, a.enabled, a.revision, a.capabilities_json, a.created_at FROM agent_accounts a JOIN accounts u ON u.id = a.account_id ", $tail) };
}

async fn owner(tx: &mut WriteTransaction, actor: AccountId) -> Result<()> {
    let live: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM user_logins owner WHERE owner.account_id = ? AND owner.is_agent = 0 AND owner.suspended_at IS NULL AND owner.password_change_required = 0 AND NOT EXISTS(SELECT 1 FROM credential_revocations pending WHERE pending.account_id = owner.account_id AND pending.target_epoch = owner.authentication_epoch AND pending.completed_at IS NULL))")
        .bind(actor.0).fetch_one(&mut **tx).await?;
    if live { Ok(()) } else { Err(AgentError::Forbidden) }
}

async fn owned(tx: &mut WriteTransaction, actor: AccountId, agent: AccountId) -> Result<Agent> {
    owner(tx, actor).await?;
    let row: AgentRow = sqlx::query_as(select_agent!("WHERE a.account_id = ? AND a.owner_id = ?"))
        .bind(agent.0)
        .bind(actor.0)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(AgentError::NotFound)?;
    row.try_into()
}

impl AgentAccountStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn list(&self, actor: AccountId) -> Result<Vec<Agent>> {
        let mut tx = self.pool.begin().await?;
        owner(&mut tx, actor).await?;
        let rows: Vec<AgentRow> = sqlx::query_as(select_agent!("WHERE a.owner_id = ? ORDER BY a.created_at, a.account_id LIMIT 50"))
            .bind(actor.0)
            .fetch_all(&mut *tx)
            .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }

    pub async fn find(&self, actor: AccountId, agent: AccountId) -> Result<Agent> {
        owned(&mut self.pool.begin().await?, actor, agent).await
    }

    /// Authentication loads this record only for a permanently marked agent identity.
    pub async fn active(&self, agent: AccountId) -> Result<Option<Agent>> {
        let row: Option<AgentRow> = sqlx::query_as(select_agent!("JOIN user_logins human ON human.account_id = a.owner_id JOIN user_logins bot ON bot.account_id = a.account_id WHERE a.account_id = ? AND a.enabled = 1 AND human.is_agent = 0 AND human.suspended_at IS NULL AND human.password_change_required = 0 AND bot.is_agent = 1 AND bot.suspended_at IS NULL AND NOT EXISTS(SELECT 1 FROM credential_revocations pending WHERE pending.account_id = human.account_id AND pending.target_epoch = human.authentication_epoch AND pending.completed_at IS NULL)"))
            .bind(agent.0).fetch_optional(&self.pool).await?;
        row.map(TryInto::try_into).transpose()
    }

    pub async fn create(&self, actor: AccountId, input: &AgentInput) -> Result<Agent> {
        input.validate().map_err(AgentError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        owner(&mut tx, actor).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM agent_accounts WHERE owner_id = ?").bind(actor.0).fetch_one(&mut *tx).await?;
        if count >= MAX_AGENTS {
            return Err(AgentError::Invalid("an owner may have at most 50 agent identities"));
        }
        let now = Timestamp::now().0;
        let handle = format!("agent-{}-{}", actor.0, input.slug);
        let display = format!("{} (agent)", input.name);
        let id: i64 = sqlx::query_scalar(INSERT_UNRESERVED_ACCOUNT)
            .bind("user")
            .bind(&handle)
            .bind(&display)
            .bind(now)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| super::accounts::account_insert_error(error, "slug"))?;
        sqlx::query(
            "INSERT INTO user_logins (account_id, email, password_hash, is_admin, is_agent) VALUES (?, ?, '!non-interactive-agent', 0, 1)",
        )
        .bind(id)
        .bind(format!("agent-{id}@agents.invalid"))
        .execute(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO agent_accounts(account_id, owner_id, slug, name, capabilities_json, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(id).bind(actor.0).bind(&input.slug).bind(&input.name)
            .bind(serde_json::to_string(&input.capabilities).map_err(|e| StorageError::Corrupt(e.to_string()))?).bind(now).bind(now)
            .execute(&mut *tx).await?;
        scopes::replace(&mut tx, actor, AccountId(id), &input.scopes).await?;
        let created = owned(&mut tx, actor, AccountId(id)).await?;
        tx.commit().await?;
        Ok(created)
    }

    pub async fn update(&self, actor: AccountId, agent: AccountId, revision: i64, input: &AgentInput, enabled: bool) -> Result<Agent> {
        input.validate().map_err(AgentError::Invalid)?;
        let mut tx = self.pool.begin_immediate().await?;
        let current = owned(&mut tx, actor, agent).await?;
        if current.revision != revision {
            return Err(AgentError::Conflict);
        }
        if current.slug != input.slug {
            return Err(AgentError::Invalid("an agent handle cannot be renamed"));
        }
        // Only this writer sees the temporary enabled state used to validate replacement scopes.
        sqlx::query("UPDATE agent_accounts SET enabled = 1 WHERE account_id = ?").bind(agent.0).execute(&mut *tx).await?;
        scopes::replace(&mut tx, actor, agent, &input.scopes).await?;
        sqlx::query("UPDATE agent_accounts SET name = ?, capabilities_json = ?, enabled = ?, revision = revision + 1, updated_at = ? WHERE account_id = ?")
            .bind(&input.name).bind(serde_json::to_string(&input.capabilities).map_err(|e| StorageError::Corrupt(e.to_string()))?)
            .bind(enabled).bind(Timestamp::now().0).bind(agent.0).execute(&mut *tx).await?;
        sqlx::query("UPDATE accounts SET display_name = ?, updated_at = ? WHERE id = ?")
            .bind(format!("{} (agent)", input.name))
            .bind(Timestamp::now().0)
            .bind(agent.0)
            .execute(&mut *tx)
            .await?;
        if !enabled {
            credentials::revoke_all(&mut tx, agent).await?;
        }
        let updated = owned(&mut tx, actor, agent).await?;
        tx.commit().await?;
        Ok(updated)
    }

    /// Disable is independent of old scope validity and permanently revokes all credentials.
    pub async fn disable(&self, actor: AccountId, agent: AccountId, revision: i64) -> Result<()> {
        let mut tx = self.pool.begin_immediate().await?;
        let current = owned(&mut tx, actor, agent).await?;
        if current.revision != revision {
            return Err(AgentError::Conflict);
        }
        sqlx::query("UPDATE agent_accounts SET enabled = 0, revision = revision + 1, updated_at = ? WHERE account_id = ?")
            .bind(Timestamp::now().0)
            .bind(agent.0)
            .execute(&mut *tx)
            .await?;
        credentials::revoke_all(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(())
    }
}
