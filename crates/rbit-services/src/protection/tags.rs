//! Tag rule administration and the shared tag/release publication policy adapter.
use super::ProtectionService;
use crate::{
    RepoContext, ServiceError,
    audit::{AuditAction, AuditEntry},
};
use rbit_access::{protection::BranchPattern, tag_protection};
use rbit_core::{User, protection::PushActor};

impl ProtectionService {
    pub async fn tag_rules(&self, repo: &RepoContext) -> Result<Vec<(i64, BranchPattern)>, ServiceError> {
        if !repo.access.can_admin() {
            return Err(ServiceError::Forbidden);
        }
        Ok(self.ctx.database.tag_protection().list(repo.repo.repo.id).await?)
    }
    pub async fn create_tag_rule(&self, repo: &RepoContext, actor: &User, pattern: &str) -> Result<i64, ServiceError> {
        if !repo.access.can_admin() {
            return Err(ServiceError::Forbidden);
        }
        repo.ensure_writable()?;
        if self.ctx.database.mirrors().is_mirror(repo.repo.repo.id).await? {
            return Err(ServiceError::Invalid("Tag protection cannot be configured on a pull mirror".to_owned()));
        }
        let pattern = BranchPattern::parse(pattern).map_err(|e| ServiceError::Invalid(e.to_string()))?;
        let id = self
            .ctx
            .database
            .tag_protection()
            .create(repo.repo.repo.id, &pattern)
            .await?
            .ok_or_else(|| ServiceError::Invalid("a repository can have at most 100 tag protection rules".to_owned()))?;
        self.audit
            .record(
                AuditEntry::new(AuditAction::TagProtectionCreated)
                    .by(actor)
                    .on(repo.repo.full_name())
                    .detail(format!("tag protection {pattern}")),
            )
            .await;
        Ok(id)
    }
    pub async fn delete_tag_rule(&self, repo: &RepoContext, actor: &User, id: i64) -> Result<(), ServiceError> {
        if !repo.access.can_admin() {
            return Err(ServiceError::Forbidden);
        }
        repo.ensure_writable()?;
        if !self.ctx.database.tag_protection().delete(repo.repo.repo.id, id).await? {
            return Err(ServiceError::NotFound);
        }
        self.audit
            .record(
                AuditEntry::new(AuditAction::TagProtectionDeleted)
                    .by(actor)
                    .on(repo.repo.full_name())
                    .detail(format!("tag protection rule {id}")),
            )
            .await;
        Ok(())
    }
    pub(crate) async fn tag_patterns(&self, repo: &RepoContext) -> Result<Vec<BranchPattern>, ServiceError> {
        Ok(self.ctx.database.tag_protection().list(repo.repo.repo.id).await?.into_iter().map(|(_, pattern)| pattern).collect())
    }
    pub(crate) async fn authorize_release(&self, repo: &RepoContext, tag: &str, person: bool) -> Result<(), ServiceError> {
        let patterns = self.tag_patterns(repo).await?;
        if !tag_protection::may_publish(&patterns, tag, repo.access, person) {
            return Err(ServiceError::Forbidden);
        }
        Ok(())
    }
    pub(crate) async fn check_tag_updates(
        &self,
        repo: &RepoContext,
        actor: &PushActor<'_>,
        updates: &[rbit_core::RefUpdate],
        verdict: &mut super::PushVerdict,
    ) -> Result<(), ServiceError> {
        let patterns = self.tag_patterns(repo).await?;
        for update in updates {
            if let Some(refusal) = tag_protection::check_push(&patterns, actor.access, actor.user.is_some() && !actor.deploy_key, update)
                && !verdict.refusals.iter().any(|existing| existing.name == refusal.name)
            {
                verdict.refusals.push(refusal);
            }
        }
        Ok(())
    }
}
