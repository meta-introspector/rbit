//! Personal viewed markers and comparisons to a server-selected previous review.
use super::{PullContext, PullService, RepoContext, ReviewProgress, ServiceError};
use rbit_core::User;
use rbit_git::{CommitDetail, DiffLimits, compare::Comparison};

impl PullService {
    pub async fn review_progress(&self, actor: &User, ctx: &PullContext) -> Result<Vec<ReviewProgress>, ServiceError> {
        if ctx.repo().viewer != Some(actor.id()) {
            return Err(ServiceError::Forbidden);
        }
        let comparison = self.changes(ctx, DiffLimits::default()).await?;
        self.review_progress_for_comparison(actor, ctx, &comparison).await
    }

    /// Bind browser markers to the exact comparison already prepared for display by `changes`.
    /// Mutations still recompute current fingerprints before accepting a viewed marker.
    pub async fn review_progress_for_comparison(
        &self,
        actor: &User,
        ctx: &PullContext,
        comparison: &Comparison,
    ) -> Result<Vec<ReviewProgress>, ServiceError> {
        if ctx.repo().viewer != Some(actor.id()) {
            return Err(ServiceError::Forbidden);
        }
        if comparison.head.id != ctx.pull.head_sha {
            return Err(ServiceError::Conflict("the displayed review head changed"));
        }
        let base = comparison.merge_base.as_ref().unwrap_or(&comparison.base.id).clone();
        let fingerprints = ctx.repo().reader.review_fingerprints(base, comparison.head.id.clone(), &comparison.files).await?;
        let saved = self.ctx().database.reviews().progress(ctx.issue().id, actor.id()).await?;
        Ok(fingerprints
            .into_iter()
            .map(|(path, fingerprint)| {
                let viewed = saved.iter().any(|entry| entry.0 == path && entry.1 == fingerprint);
                ReviewProgress { path, fingerprint, viewed }
            })
            .collect())
    }

    pub async fn mark_reviewed_file(
        &self,
        actor: &User,
        ctx: &PullContext,
        path: &str,
        expected_head: &str,
        fingerprint: &str,
        viewed: bool,
    ) -> Result<(), ServiceError> {
        if path.len() > 4096 {
            return Err(ServiceError::Invalid("review file path is too long".into()));
        }
        let (actor, current) = self.fresh_review_context(actor, ctx).await?;
        if current.pull.head_sha != expected_head {
            return Err(ServiceError::Conflict("the reviewed head changed"));
        }
        let progress = self.review_progress(&actor, &current).await?;
        let item = progress
            .iter()
            .find(|p| p.path == path && p.fingerprint == fingerprint)
            .ok_or_else(|| ServiceError::Invalid("the reviewed file changed".into()))?;
        self.ctx()
            .database
            .reviews()
            .set_progress(current.issue().id, actor.id(), path, viewed.then_some(item.fingerprint.as_str()))
            .await?;
        Ok(())
    }

    /// The baseline is always the caller's own submitted review, never an arbitrary commit id.
    pub async fn changes_since_review(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
    ) -> Result<Option<(String, CommitDetail)>, ServiceError> {
        Self::require_review_head(ctx, head)?;
        if head.viewer != Some(actor.id()) || ctx.repo().viewer != Some(actor.id()) {
            return Err(ServiceError::Forbidden);
        }
        let (actor, current) = self.fresh_review_context(actor, ctx).await?;
        let reopened = crate::repositories::RepositoryService::new(self.ctx())
            .open(Some(&actor), head.repo.owner.name.as_str(), head.repo.repo.name.as_str())
            .await?;
        Self::require_review_head(&current, &reopened)?;
        if current.pull.head_sha != ctx.pull.head_sha {
            return Err(ServiceError::Conflict("the pull request changed; refresh the review comparison"));
        }
        if reopened.repo.repo.id != head.repo.repo.id {
            return Err(ServiceError::NotFound);
        }
        let Some(baseline) = self.ctx().database.reviews().last_reviewed_head(current.issue().id, actor.id()).await? else {
            return Ok(None);
        };
        let changes = current.repo().reader.review_changes(baseline.clone(), current.pull.head_sha.clone(), DiffLimits::default()).await?;
        Ok(Some((baseline, changes)))
    }
}
