//! Optional site and organization MFA enrollment policies.
//!
//! Organization owners can require MFA for their members' accounts. Password/SSO
//! sign-in remains possible so an unenrolled user can reach Security and enroll;
//! all other authenticated activity, including API tokens and Git, is refused.
use crate::{
    ServiceError,
    audit::{AuditAction, AuditEntry, AuditService},
};
use rbit_core::{AccountId, Organization, User};
use rbit_storage::{Database, sqlite::authentication_policy::AuthenticationPolicyStore};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

#[derive(Debug, Clone, Copy)]
struct PolicyState {
    site: bool,
    any_organization: bool,
}

#[derive(Debug, Clone)]
pub struct AuthenticationPolicyService {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    database: Database,
    state: RwLock<PolicyState>,
    mutations: Mutex<()>,
    audit: AuditService,
}

impl AuthenticationPolicyService {
    /// Load policy switches once; disabled policies add no SQL to request paths.
    pub(crate) async fn load(database: &Database, default_required: bool) -> Result<Self, ServiceError> {
        let store = AuthenticationPolicyStore::new(database);
        let state = PolicyState {
            site: store.site().await?.unwrap_or(default_required),
            any_organization: store.any_organization_required().await?,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                database: database.clone(),
                state: RwLock::new(state),
                mutations: Mutex::new(()),
                audit: AuditService::from_store(database.audit()),
            }),
        })
    }

    pub async fn site_required(&self) -> Result<bool, ServiceError> {
        Ok(self.inner.state.read().await.site)
    }

    pub async fn organization_required(&self, org: AccountId) -> Result<bool, ServiceError> {
        Ok(AuthenticationPolicyStore::new(&self.inner.database).organization(org).await?)
    }

    pub async fn required(&self, user: AccountId) -> Result<bool, ServiceError> {
        let state = *self.inner.state.read().await;
        if state.site {
            return Ok(true);
        }
        if !state.any_organization {
            return Ok(false);
        }
        Ok(AuthenticationPolicyStore::new(&self.inner.database).required_by_membership(user).await?)
    }

    /// Rechecked on each request/credential use, so existing credentials cannot bypass
    /// a new requirement and disabling the requirement takes effect immediately.
    pub async fn needs_enrollment(&self, user: AccountId) -> Result<bool, ServiceError> {
        Ok(self.required(user).await? && !self.inner.database.two_factor().is_enabled(user).await?)
    }

    async fn require_enrolled(&self, actor: &User, required: bool) -> Result<(), ServiceError> {
        if required && !self.inner.database.two_factor().is_enabled(actor.id()).await? {
            return Err(ServiceError::Invalid(
                "Set up an authenticator app or passkey in your Security settings before requiring MFA.".into(),
            ));
        }
        Ok(())
    }

    pub async fn set_site(&self, actor: &User, required: bool) -> Result<(), ServiceError> {
        if !actor.is_admin {
            return Err(ServiceError::Forbidden);
        }
        self.require_enrolled(actor, required).await?;
        let mutation = self.inner.mutations.lock().await;
        // Publish the conservative check before dispatching an enabling write. If the
        // caller is cancelled after SQL commits, the policy cannot stay unenforced.
        if required {
            self.inner.state.write().await.site = true;
        }
        self.inner.database.instance_settings().save(&[("require_two_factor", if required { "true" } else { "false" })]).await?;
        if !required {
            self.inner.state.write().await.site = false;
        }
        drop(mutation);
        self.inner
            .audit
            .record(AuditEntry::new(AuditAction::InstanceSettingsChanged).by(actor).detail(format!("require MFA: {required}")))
            .await;
        Ok(())
    }

    pub async fn set_organization(&self, actor: &User, org: &Organization, required: bool) -> Result<(), ServiceError> {
        let role = self.inner.database.orgs().role_of(org.account.id, actor.id()).await?;
        if !actor.is_admin && !role.is_some_and(rbit_core::OrgRole::is_owner) {
            return Err(ServiceError::NotFound);
        }
        self.require_enrolled(actor, required).await?;
        let mutation = self.inner.mutations.lock().await;
        let store = AuthenticationPolicyStore::new(&self.inner.database);
        if required {
            self.inner.state.write().await.any_organization = true;
        }
        store.set_organization(org.account.id, required).await?;
        let any_organization = store.any_organization_required().await?;
        self.inner.state.write().await.any_organization = any_organization;
        drop(mutation);
        self.inner
            .audit
            .record(
                AuditEntry::new(AuditAction::OrgPermissionChanged)
                    .by(actor)
                    .on(org.name().as_str())
                    .detail(format!("require MFA: {required}")),
            )
            .await;
        Ok(())
    }
}
