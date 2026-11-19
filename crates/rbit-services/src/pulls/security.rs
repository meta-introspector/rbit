//! Security gates are exact-revision requirements, including immediately before landing.
use super::{Blocked, MergeFailure, PullContext, PullService};
use crate::ServiceError;
use rbit_core::pull::{Requirement, RequirementCheck};
impl PullService {
    pub(super) async fn security_requirement(
        &self,
        ctx: &PullContext,
        base: Option<&str>,
    ) -> Result<Option<RequirementCheck>, ServiceError> {
        // Match branch-protection presentation: private scan evidence is never
        // disclosed in a public or read-scoped merge box.
        if !ctx.repo().access.can_write() {
            return Ok(None);
        }
        let gate = crate::security::gate::evaluate(self.ctx(), ctx.repo().repo.repo.id, &ctx.pull.head_sha, base).await?;
        Ok((gate != crate::security::Gate::Disabled)
            .then_some(RequirementCheck { requirement: Requirement::Security(gate), met: gate.met() }))
    }
    pub(super) async fn security_publication_fence(
        &self,
        actor: &rbit_core::User,
        ctx: &PullContext,
        base: &str,
    ) -> Result<rbit_storage::sqlite::security::SecurityWriteFence, MergeFailure> {
        let fence = self.ctx().database.security().publication_fence().await.map_err(ServiceError::from)?;
        let actor =
            self.ctx().database.accounts().find_active_user(actor.id()).await.map_err(ServiceError::from)?.ok_or(ServiceError::NotFound)?;
        let current_repo =
            self.inner.repositories.open(Some(&actor), ctx.repo().repo.owner.name.as_str(), ctx.repo().repo.repo.name.as_str()).await?;
        if current_repo.repo.repo.id != ctx.repo().repo.repo.id || !current_repo.can_write_content() || !ctx.repo().access.can_write() {
            return Err(ServiceError::NotFound.into());
        }
        current_repo.ensure_writable()?;
        crate::agents::require_capability(self.ctx(), &actor, current_repo.repo.repo.id, rbit_identity_models::agents::Capability::Merge)
            .await?;
        let current = self.ctx().database.pulls().find(ctx.issue().id).await.map_err(ServiceError::from)?.ok_or(ServiceError::NotFound)?;
        if current.head_sha != ctx.pull.head_sha {
            return Err(MergeFailure::HeadChanged);
        }
        if current.base_branch != ctx.pull.base_branch {
            return Err(MergeFailure::BaseChanged);
        }
        let issue = self.ctx().database.issues().find(ctx.issue().id).await.map_err(ServiceError::from)?.ok_or(ServiceError::NotFound)?;
        if current.is_merged() || !issue.is_open() {
            return Err(MergeFailure::Blocked(Blocked::pull_closed()));
        }
        if current.is_draft {
            return Err(MergeFailure::Blocked(Blocked::new("merge_draft", "the pull request is a draft", "Mark it ready for review.")));
        }
        self.recheck_security_merge(ctx, base).await?;
        Ok(fence)
    }
    pub(super) async fn recheck_security_merge(&self, ctx: &PullContext, base: &str) -> Result<(), MergeFailure> {
        if let Some(check) = self.security_requirement(ctx, Some(base)).await?
            && !check.met
        {
            return Err(MergeFailure::Blocked(Blocked::new(
                super::merge_codes::SECURITY,
                check.requirement.describe(false),
                "Complete trusted scans for both exact revisions and triage newly introduced findings.",
            )));
        }
        Ok(())
    }
}
