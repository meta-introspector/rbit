//! Refresh both repositories and branch identities before preview and under the publication fence.
use super::{PullContext, PullService, RepoContext, ServiceError, User};

impl PullService {
    pub(super) async fn conflict_context(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        expected: Option<(&str, &str)>,
    ) -> Result<(User, PullContext, String), ServiceError> {
        Self::require_review_head(ctx, head)?;
        if head.viewer != Some(actor.id()) || !head.can_write_content() {
            return Err(ServiceError::Forbidden);
        }
        let (actor, current) = self.fresh_review_context(actor, ctx).await?;
        if current.pull.head_branch != ctx.pull.head_branch || current.pull.base_branch != ctx.pull.base_branch {
            return Err(ServiceError::Conflict("the pull request branches changed; refresh the conflict preview"));
        }
        self.suggestion_authority(&actor, &current, head, expected.map_or(current.pull.head_sha.as_str(), |pair| pair.0)).await?;
        let current_head = current.head_repo.as_ref().ok_or(ServiceError::NotFound)?;
        if rbit_collaboration::pulls::exposes_private_head(
            current.repo().repo.repo.visibility.is_public(),
            current_head.repo.visibility.is_public(),
        ) {
            return Err(ServiceError::Forbidden);
        }
        let base = self.base_tip(&current).await?.ok_or(ServiceError::NotFound)?;
        if expected.is_some_and(|pair| pair.1 != base) {
            return Err(ServiceError::Conflict("the target branch changed; refresh the conflict preview"));
        }
        Ok((actor, current, base))
    }
}
