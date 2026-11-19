//! Human-owned credential lifecycle with stable public management identifiers.

use rbit_core::{AccountId, Timestamp, TokenScope, User};
use rbit_identity::access_tokens::AccessTokenApplication;
use rbit_identity_models::agents::AgentScope;
use rbit_storage::sqlite::agent_accounts::AgentError;

use super::{AgentCredential, AgentService, map, map_token};
use crate::{
    ServiceError,
    audit::{AuditAction, AuditEntry},
    tokens::CreateAccessToken,
};

impl AgentService {
    pub async fn scope_names(&self, actor: &User, agent: AccountId) -> Result<Vec<(AgentScope, Option<String>)>, ServiceError> {
        self.store.scope_names(actor.id(), agent).await.map_err(map)
    }

    pub async fn managed_credentials(&self, actor: &User, agent: AccountId) -> Result<Vec<AgentCredential>, ServiceError> {
        self.store.managed_credentials(actor.id(), agent).await.map_err(map)
    }

    pub async fn create_managed_credential(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        input: CreateAccessToken,
    ) -> Result<(AgentCredential, String), ServiceError> {
        self.issue_managed_credential(actor, agent, revision, None, input, None).await
    }

    pub async fn rotate_credential(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        old: i64,
        input: CreateAccessToken,
    ) -> Result<(AgentCredential, String), ServiceError> {
        self.issue_managed_credential(actor, agent, revision, Some(old), input, None).await
    }

    /// API callers may constrain the issued credential to their own expiration.
    pub async fn create_managed_credential_bounded(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        input: CreateAccessToken,
        ceiling: Option<Timestamp>,
    ) -> Result<(AgentCredential, String), ServiceError> {
        self.issue_managed_credential(actor, agent, revision, None, input, ceiling).await
    }

    pub async fn rotate_credential_bounded(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        old: i64,
        input: CreateAccessToken,
        ceiling: Option<Timestamp>,
    ) -> Result<(AgentCredential, String), ServiceError> {
        self.issue_managed_credential(actor, agent, revision, Some(old), input, ceiling).await
    }

    async fn issue_managed_credential(
        &self,
        actor: &User,
        agent: AccountId,
        revision: i64,
        replace: Option<i64>,
        input: CreateAccessToken,
        ceiling: Option<Timestamp>,
    ) -> Result<(AgentCredential, String), ServiceError> {
        let record = self.store.find(actor.id(), agent).await.map_err(map)?;
        if !record.enabled || record.revision != revision {
            return Err(map(AgentError::Conflict));
        }
        if input.scope == TokenScope::Admin {
            return Err(ServiceError::Invalid("agent credentials cannot have administrator scope".into()));
        }
        let identity = self.accounts.find_user(agent).await?.ok_or(ServiceError::NotFound)?;
        let application = AccessTokenApplication::new(&self.tokens, &self.accounts);
        let (token, secret) = application.create(&identity, &input.name, input.scope, input.expires_in_days).await.map_err(map_token)?;
        // Check the actual generated expiry, not a preflight estimate: issuance may
        // cross a clock tick or wait behind password/database work. The old token
        // remains valid if a rotation cannot honor the caller's lifetime ceiling.
        if ceiling.is_some_and(|limit| token.expires_at.is_none_or(|expiry| expiry > limit)) {
            application.revoke(&identity, token.id).await.map_err(map_token)?;
            return Err(ServiceError::Invalid("an agent credential may not outlive the token that creates it".into()));
        }
        let credential = match self.store.link_managed_credential(actor.id(), agent, revision, token.id, replace).await {
            Ok(credential) => credential,
            Err(error) => {
                if let Err(cleanup) = application.revoke(&identity, token.id).await {
                    let cleanup = map_token(cleanup);
                    tracing::warn!(agent = agent.0, token = token.id.0, error = %cleanup, "failed to remove unusable unlinked agent credential");
                }
                return Err(map(error));
            }
        };
        if let Some(old) = replace {
            self.audit
                .record(
                    AuditEntry::new(AuditAction::TokenRevoked)
                        .by(actor)
                        .on(&record.handle)
                        .detail(format!("rotated agent credential {old}")),
                )
                .await;
        }
        self.audit
            .record(
                AuditEntry::new(AuditAction::TokenCreated)
                    .by(actor)
                    .on(record.handle)
                    .detail(format!("agent credential {}", credential.id)),
            )
            .await;
        Ok((credential, secret))
    }

    pub async fn revoke_managed_credential(&self, actor: &User, agent: AccountId, id: i64) -> Result<(), ServiceError> {
        self.store.revoke_managed_credential(actor.id(), agent, id).await.map_err(map)?;
        self.audit
            .record(AuditEntry::new(AuditAction::TokenRevoked).by(actor).on(agent.0.to_string()).detail(format!("agent credential {id}")))
            .await;
        Ok(())
    }
}
