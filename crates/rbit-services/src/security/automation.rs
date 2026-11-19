//! Opt-in durable native scans; workflow tokens request work but cannot attest its result.
mod advisories;
mod remediation;
mod scanners;
mod worker;
use super::{Configuration, SecurityService, Uploader};
use crate::{RepoContext, ServiceError};
pub use rbit_collaboration::security::automation::{AutomationSettings, FRESH_SECONDS, dependencies::Package};
use rbit_collaboration::security::automation::{DEPENDENCIES, SECRETS};
use rbit_core::{RepoId, Timestamp, User};
use rbit_storage::sqlite::security::AutomationConfiguration;
pub use rbit_storage::sqlite::security::Remediation;
use std::sync::atomic::AtomicBool;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Default)]
pub(super) struct Background {
    pub started: AtomicBool,
    pub stop: CancellationToken,
}
impl SecurityService {
    pub async fn automation_configuration(&self, ctx: &RepoContext) -> Result<Option<AutomationConfiguration>, ServiceError> {
        Self::require_read(ctx)?;
        Ok(self.ctx.database.security().automation(ctx.repo.repo.id).await?)
    }
    pub async fn configure_automation(&self, actor: &User, ctx: &RepoContext, settings: AutomationSettings) -> Result<(), ServiceError> {
        settings.validate().map_err(|e| ServiceError::Invalid(e.into()))?;
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let fence = self.ctx.database.security().publication_fence().await?;
        let current = self.current_admin(actor, ctx).await?;
        fence.configure_automation(current.repo.repo.id, actor.id(), &settings).await?;
        self.audit(
            actor,
            &current,
            crate::audit::AuditAction::SecuritySettingsChanged,
            format!(
                "automatic secrets={}; npm={}; network consent={}; automatic PRs={}",
                settings.secrets, settings.dependencies, settings.package_network_consent, settings.automatic_pull_requests
            ),
        )
        .await;
        Ok(())
    }
    pub async fn request_automatic_scan(&self, actor: Uploader<'_>, ctx: &RepoContext, sha: &str) -> Result<Vec<i64>, ServiceError> {
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ServiceError::Invalid("Use an immutable full commit id.".into()));
        }
        let sha = ctx.reader.commit_id(sha.to_owned()).await?;
        let config = self.automation_configuration(ctx).await?.ok_or(ServiceError::NotFound)?;
        let mut ids = Vec::new();
        for channel in config.settings.channels() {
            let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
            let fence = self.ctx.database.security().publication_fence().await?;
            let current = self.automation_configuration(ctx).await?.ok_or(ServiceError::NotFound)?;
            if current.generation != config.generation {
                return Err(ServiceError::Conflict("automatic scan settings changed"));
            }
            self.authorize(
                actor,
                ctx,
                &Configuration {
                    scanner: "automation".into(),
                    workflow: current.settings.workflow,
                    require_scan: false,
                    minimum_severity: 1,
                    generation: current.generation,
                },
            )
            .await?;
            ids.push(fence.enqueue_scan(ctx.repo.repo.id, channel, &sha, current.generation, Timestamp::now()).await?);
        }
        Ok(ids)
    }
    pub(super) async fn automation_actor(
        &self,
        repo: RepoId,
        config: &AutomationConfiguration,
    ) -> Result<(User, RepoContext), ServiceError> {
        let id = config.enabled_by.ok_or(ServiceError::Forbidden)?;
        let actor = self.ctx.database.accounts().find_active_user(id).await?.ok_or(ServiceError::Forbidden)?;
        let owned = self.ctx.database.admin().find_repository(repo).await?.ok_or(ServiceError::NotFound)?;
        let ctx = crate::RepositoryService::new(&self.ctx).open(Some(&actor), owned.owner.name.as_str(), owned.repo.name.as_str()).await?;
        if ctx.repo.repo.id != repo || !ctx.access.can_admin() {
            return Err(ServiceError::Forbidden);
        }
        ctx.ensure_writable()?;
        if self.ctx.database.mirrors().is_mirror(repo).await? {
            return Err(ServiceError::Forbidden);
        }
        Ok((actor, ctx))
    }
    pub(super) async fn enqueue_current(&self, repo: RepoId, now: Timestamp) -> Result<(), ServiceError> {
        let store = self.ctx.database.security();
        let Some(config) = store.automation(repo).await? else { return Ok(()) };
        if !config.settings.secrets && !config.settings.dependencies {
            return Ok(());
        }
        let (_, ctx) = self.automation_actor(repo, &config).await?;
        let mut targets = std::collections::BTreeSet::new();
        if let Ok(tip) = ctx.reader.commit_id(format!("refs/heads/{}", ctx.repo.repo.default_branch)).await {
            targets.insert(tip);
        }
        // Bounded reconciliation covers missed push events and exact PR base/head revisions.
        for (_, pull) in self.ctx.database.pulls().list(repo, &rbit_storage::sqlite::pulls::PullFilter::default(), 100, 0).await? {
            targets.insert(pull.head_sha);
            if let Ok(tip) = ctx.reader.commit_id(format!("refs/heads/{}", pull.base_branch)).await {
                targets.insert(tip);
            }
        }
        for sha in targets {
            for scanner in config.settings.channels() {
                let channel = if scanner == SECRETS { "secrets" } else { "npm" };
                if let Some(scan) = store.latest_automatic(repo, channel, &sha, config.generation).await? {
                    let age = now.seconds().saturating_sub(scan.created_at);
                    if scan.state == "pending" || age < if scan.state == "complete" { FRESH_SECONDS / 2 } else { 600 } {
                        continue;
                    }
                }
                let _lease = self.ctx.repositories.lease(repo).await;
                let fence = store.publication_fence().await?;
                self.automation_actor(repo, &config).await?;
                fence.enqueue_scan(repo, scanner, &sha, config.generation, now).await?;
            }
        }
        Ok(())
    }
}
