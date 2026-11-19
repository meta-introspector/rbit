//! Repository security composition: current authority, immutable Git targets and bounded scans.
mod authority;
pub mod automation;
pub(crate) mod gate;
mod parsing;
use crate::actions::tokens::TaskGrant;
use crate::{RepoContext, ServiceContext, ServiceError};
pub use rbit_collaboration::security::{Gate, MAX_REPORT_BYTES, Settings};
use rbit_core::User;
pub use rbit_storage::sqlite::security::{Configuration, Occurrence, Scan};
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;
#[derive(Debug, Clone, Copy)]
pub enum Uploader<'a> {
    User(&'a User),
    Task(&'a TaskGrant),
}
#[derive(Debug, Clone)]
pub struct SecurityService {
    ctx: ServiceContext,
    pulls: crate::PullService,
    slots: Arc<Semaphore>,
    closed: Arc<Mutex<bool>>,
    tasks: TaskTracker,
    background: Arc<automation::Background>,
}
impl SecurityService {
    pub(crate) fn new(ctx: ServiceContext, pulls: crate::PullService) -> Self {
        Self {
            ctx,
            pulls,
            slots: Arc::new(Semaphore::new(2)),
            closed: Arc::new(Mutex::new(false)),
            tasks: TaskTracker::new(),
            background: Arc::new(automation::Background::default()),
        }
    }
    pub fn require_read(ctx: &RepoContext) -> Result<(), ServiceError> {
        if ctx.access.can_write() { Ok(()) } else { Err(ServiceError::NotFound) }
    }
    pub async fn configuration(&self, ctx: &RepoContext) -> Result<Option<Configuration>, ServiceError> {
        Self::require_read(ctx)?;
        Ok(self.ctx.database.security().configuration(ctx.repo.repo.id).await?)
    }
    pub async fn configure(&self, user: &User, ctx: &RepoContext, settings: Settings) -> Result<(), ServiceError> {
        settings.validate().map_err(|error| ServiceError::Invalid(error.into()))?;
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let fence = self.ctx.database.security().publication_fence().await?;
        let current = self.current_admin(user, ctx).await?;
        fence.configure(current.repo.repo.id, &settings).await?;
        self.audit(
            user,
            &current,
            crate::audit::AuditAction::SecuritySettingsChanged,
            format!("scanner={}; required={}; minimum={}", settings.scanner, settings.require_scan, settings.minimum_severity),
        )
        .await;
        Ok(())
    }
    pub async fn begin(&self, actor: Uploader<'_>, ctx: &RepoContext, sha: &str) -> Result<Scan, ServiceError> {
        Self::require_read(ctx)?;
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ServiceError::Invalid("Use a full immutable commit object id.".into()));
        }
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let canonical = ctx.reader.commit_id(sha.to_owned()).await?;
        let fence = self.ctx.database.security().publication_fence().await?;
        let config = self.configuration(ctx).await?.ok_or_else(|| ServiceError::Invalid("Configure a scanner first.".into()))?;
        let source = self.authorize(actor, ctx, &config).await?;
        let id = fence.begin_scan(ctx.repo.repo.id, &canonical, &source, config.generation).await?;
        self.ctx.database.security().scan(ctx.repo.repo.id, id).await?.ok_or(ServiceError::NotFound)
    }
    pub async fn finish(&self, actor: Uploader<'_>, ctx: &RepoContext, id: i64, bytes: Vec<u8>) -> Result<Scan, ServiceError> {
        Self::require_read(ctx)?;
        let store = self.ctx.database.security();
        let config = self.configuration(ctx).await?.ok_or(ServiceError::NotFound)?;
        let source = self.authorize(actor, ctx, &config).await?;
        let scan = store.scan(ctx.repo.repo.id, id).await?.ok_or(ServiceError::NotFound)?;
        if scan.source != source {
            return Err(ServiceError::NotFound);
        }
        let report = self.parse(bytes).await?;
        if report.scanner != scan.scanner {
            return Err(ServiceError::Invalid("SARIF tool name must match the configured scanner.".into()));
        }
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let fence = store.publication_fence().await?;
        let current = self.configuration(ctx).await?.ok_or(ServiceError::NotFound)?;
        self.authorize(actor, ctx, &current).await?;
        fence.finish_scan(ctx.repo.repo.id, &scan, &report).await?;
        store.scan(ctx.repo.repo.id, id).await?.ok_or(ServiceError::NotFound)
    }
    pub async fn scans(&self, ctx: &RepoContext) -> Result<Vec<Scan>, ServiceError> {
        Self::require_read(ctx)?;
        Ok(self.ctx.database.security().scans(ctx.repo.repo.id).await?)
    }
    pub async fn findings(&self, ctx: &RepoContext, scan: i64, offset: u32) -> Result<(Scan, Vec<Occurrence>), ServiceError> {
        Self::require_read(ctx)?;
        let store = self.ctx.database.security();
        let scan = store.scan(ctx.repo.repo.id, scan).await?.ok_or(ServiceError::NotFound)?;
        let rows = store.occurrences(ctx.repo.repo.id, scan.id, offset, 50).await?;
        Ok((scan, rows))
    }
    pub async fn dismiss(&self, user: &User, ctx: &RepoContext, id: i64, reason: &str, note: &str) -> Result<(), ServiceError> {
        if note.len() > 1000 || note.chars().any(char::is_control) {
            return Err(ServiceError::Invalid("Audit note must be at most 1000 bytes without control characters.".into()));
        }
        if reason != "reopen" {
            rbit_collaboration::security::dismissal(reason, note).map_err(|error| ServiceError::Invalid(error.into()))?;
        }
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let fence = self.ctx.database.security().publication_fence().await?;
        let current = self.current_admin(user, ctx).await?;
        if !fence.dismiss(current.repo.repo.id, id, user.id(), (reason != "reopen").then_some((reason, note))).await? {
            return Err(ServiceError::NotFound);
        }
        self.audit(
            user,
            &current,
            crate::audit::AuditAction::SecurityFindingDismissed,
            format!("finding={id}; reason={reason}; note={note}"),
        )
        .await;
        Ok(())
    }
    async fn audit(&self, user: &User, ctx: &RepoContext, action: crate::audit::AuditAction, detail: String) {
        crate::AuditService::new(self.ctx.clone())
            .record(crate::audit::AuditEntry::new(action).by(user).on(ctx.repo.full_name()).detail(detail))
            .await;
    }
}
