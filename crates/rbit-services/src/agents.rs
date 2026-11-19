//! Owner-controlled identities, scopes and one-time credentials for automation.

use rbit_core::{AccessToken, AccessTokenId, AccountId, User};
use rbit_identity::access_tokens::{AccessTokenApplication, AccessTokenError};
use rbit_identity_models::agents::{Agent, AgentInput, AgentScope};
use rbit_storage::{
    AccessTokenStore, AccountStore, StorageError,
    sqlite::agent_accounts::{AgentAccountStore, AgentError},
};

mod authority;
mod management;
pub use rbit_storage::sqlite::agent_accounts::AgentCredential;

pub const REVISION_CONFLICT: &str = "the agent changed; reload before trying again";
pub(crate) use authority::require_capability;

use crate::{
    ServiceContext, ServiceError,
    audit::{AuditAction, AuditEntry, AuditService},
    tokens::CreateAccessToken,
};

#[derive(Debug, Clone)]
pub struct AgentService {
    store: AgentAccountStore,
    accounts: AccountStore,
    tokens: AccessTokenStore,
    audit: AuditService,
}

impl AgentService {
    pub(crate) fn new(ctx: &ServiceContext) -> Self {
        Self {
            store: ctx.database.agent_accounts(),
            accounts: ctx.database.accounts(),
            tokens: ctx.database.access_tokens(),
            audit: AuditService::from_store(ctx.database.audit()),
        }
    }

    pub async fn list(&self, actor: &User) -> Result<Vec<Agent>, ServiceError> {
        self.store.list(actor.id()).await.map_err(map)
    }

    pub async fn find(&self, actor: &User, agent: AccountId) -> Result<Agent, ServiceError> {
        self.store.find(actor.id(), agent).await.map_err(map)
    }

    pub async fn scopes(&self, actor: &User, agent: AccountId) -> Result<Vec<AgentScope>, ServiceError> {
        self.store.scopes(actor.id(), agent).await.map_err(map)
    }

    pub async fn create(&self, actor: &User, input: &AgentInput) -> Result<Agent, ServiceError> {
        let agent = self.store.create(actor.id(), input).await.map_err(map)?;
        self.audit.record(AuditEntry::new(AuditAction::AgentCreated).by(actor).on(&agent.handle)).await;
        Ok(agent)
    }

    pub async fn update(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        input: &AgentInput,
        enabled: bool,
    ) -> Result<Agent, ServiceError> {
        let agent = self.store.update(actor.id(), agent, revision, input, enabled).await.map_err(map)?;
        self.audit.record(AuditEntry::new(AuditAction::AgentChanged).by(actor).on(&agent.handle)).await;
        Ok(agent)
    }

    pub async fn disable(&self, actor: &User, agent: AccountId, revision: i64) -> Result<(), ServiceError> {
        self.store.disable(actor.id(), agent, revision).await.map_err(map)?;
        self.audit.record(AuditEntry::new(AuditAction::AgentDisabled).by(actor).on(agent.0.to_string())).await;
        Ok(())
    }

    pub async fn credentials(&self, actor: &User, agent: AccountId) -> Result<Vec<AccessToken>, ServiceError> {
        self.store.credentials(actor.id(), agent).await.map_err(map)
    }

    /// Only the creating human receives the secret. Until the final revision fence
    /// links it, an agent token cannot authenticate, even after interrupted issuance.
    pub async fn create_credential(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        input: CreateAccessToken,
    ) -> Result<(AccessToken, String), ServiceError> {
        let record = self.store.find(actor.id(), agent).await.map_err(map)?;
        if !record.enabled || record.revision != revision {
            return Err(map(AgentError::Conflict));
        }
        if input.scope == rbit_core::TokenScope::Admin {
            return Err(ServiceError::Invalid("agent credentials cannot have administrator scope".into()));
        }
        let identity = self.accounts.find_user(agent).await?.ok_or(ServiceError::NotFound)?;
        let application = AccessTokenApplication::new(&self.tokens, &self.accounts);
        let (token, secret) = application.create(&identity, &input.name, input.scope, input.expires_in_days).await.map_err(map_token)?;
        if let Err(error) = self.store.link_credential(actor.id(), agent, revision, token.id).await {
            if let Err(cleanup) = application.revoke(&identity, token.id).await {
                let cleanup = map_token(cleanup);
                tracing::warn!(agent = agent.0, token = token.id.0, error = %cleanup, "failed to remove unusable unlinked agent credential");
            }
            return Err(map(error));
        }
        self.audit
            .record(
                AuditEntry::new(AuditAction::TokenCreated).by(actor).on(record.handle).detail(format!("agent credential {}", token.id.0)),
            )
            .await;
        Ok((token, secret))
    }

    pub async fn revoke_credential(&self, actor: &User, agent: AccountId, token: AccessTokenId) -> Result<(), ServiceError> {
        self.store.revoke_credential(actor.id(), agent, token).await.map_err(map)?;
        self.audit
            .record(
                AuditEntry::new(AuditAction::TokenRevoked)
                    .by(actor)
                    .on(agent.0.to_string())
                    .detail(format!("agent credential {}", token.0)),
            )
            .await;
        Ok(())
    }

    pub async fn organization_policy(&self, actor: &User, org: AccountId) -> Result<bool, ServiceError> {
        self.store.organization_policy(actor.id(), org).await.map_err(map)
    }

    pub async fn set_organization_policy(&self, actor: &User, org: AccountId, allow: bool) -> Result<(), ServiceError> {
        self.store.set_organization_policy(actor.id(), org, allow).await.map_err(map)?;
        self.audit
            .record(
                AuditEntry::new(AuditAction::OrgAgentPolicyChanged)
                    .by(actor)
                    .on(org.0.to_string())
                    .detail(format!("agents allowed: {allow}")),
            )
            .await;
        Ok(())
    }
}

pub(crate) fn map(error: AgentError) -> ServiceError {
    match error {
        AgentError::NotFound => ServiceError::NotFound,
        AgentError::Forbidden => ServiceError::Forbidden,
        AgentError::Conflict => ServiceError::Invalid(REVISION_CONFLICT.into()),
        AgentError::Invalid(message) => ServiceError::Invalid(message.into()),
        AgentError::Storage(error) => error.into(),
    }
}

fn map_token(error: AccessTokenError<StorageError, StorageError>) -> ServiceError {
    match error {
        AccessTokenError::Token(error) | AccessTokenError::Account(error) => error.into(),
        AccessTokenError::Rejected(rejection) => ServiceError::Invalid(rejection.to_string()),
        AccessTokenError::NotFound => ServiceError::NotFound,
    }
}
