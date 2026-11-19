//! Browser conflict resolution reuses exact-head review authority and owned publication.
mod authority;

use super::{PullContext, PullService};
use crate::{RepoContext, ServiceError};
use rbit_core::{
    PushSource, RefUpdate, Timestamp, User,
    pull::conflicts::{Preview, Resolution, validate_input},
};
use rbit_git::{
    edit::{EditError, conflicts::ConflictWorkspace},
    merge::{MergeError, guarded},
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveConflicts {
    pub expected_head: String,
    pub expected_base: String,
    pub resolutions: Vec<Resolution>,
}

impl PullService {
    /// Preview only after independently opening the source with the caller's credential ceiling.
    pub async fn preview_conflicts(&self, actor: &User, ctx: &PullContext, head: &RepoContext) -> Result<Preview, ServiceError> {
        let (service, actor, ctx, head) = (self.clone(), actor.clone(), ctx.clone(), head.clone());
        self.inner
            .review_work
            .submit(async move {
                let (actor, current, base) = service.conflict_context(&actor, &ctx, &head, None).await?;
                let workspace = service.conflict_workspace(&current, &head, &base).await?;
                service.conflict_context(&actor, &current, &head, Some((&current.pull.head_sha, &base))).await?;
                Ok(workspace.preview)
            })?
            .await
            .map_err(ServiceError::internal)?
    }

    pub async fn resolve_conflicts(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        input: ResolveConflicts,
    ) -> Result<String, ServiceError> {
        validate_input(&input.resolutions).map_err(|message| ServiceError::Invalid(message.into()))?;
        if ![&input.expected_head, &input.expected_base]
            .iter()
            .all(|id| matches!(id.len(), 40 | 64) && id.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(ServiceError::Invalid("expected full head and base commit IDs".into()));
        }
        let (service, actor, ctx, head) = (self.clone(), actor.clone(), ctx.clone(), head.clone());
        self.inner
            .review_work
            .submit(async move { Box::pin(service.resolve_conflicts_inner(&actor, &ctx, &head, input)).await })?
            .await
            .map_err(ServiceError::internal)?
    }

    async fn resolve_conflicts_inner(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        input: ResolveConflicts,
    ) -> Result<String, ServiceError> {
        let expected = Some((input.expected_head.as_str(), input.expected_base.as_str()));
        let (actor, current, base) = self.conflict_context(actor, ctx, head, expected).await?;
        let workspace = self.conflict_workspace(&current, head, &base).await?;
        workspace.validate_resolutions(&input.resolutions).map_err(|error| ServiceError::Invalid(error.to_string()))?;
        let guard = async {
            let fence = self.ctx().database.reviews().publication_fence().await?;
            self.conflict_context(&actor, &current, head, expected).await?;
            Ok::<_, ServiceError>(fence)
        };
        let commit = workspace
            .commit(&current.pull.head_branch, &input.resolutions, &self.identity(&actor), Timestamp::now(), guard)
            .await
            .map_err(|error| match error {
                guarded::Error::Guard(error) => error,
                guarded::Error::Git(MergeError::BaseMoved) => {
                    ServiceError::Conflict("the source branch changed; refresh the conflict preview")
                }
                guarded::Error::Git(error) => ServiceError::internal(error),
            })?;
        let update = RefUpdate {
            name: format!("refs/heads/{}", current.pull.head_branch),
            before: Some(input.expected_head),
            after: Some(commit.clone()),
        };
        self.record_review_push(head.repo.repo.id, PushSource::User(actor.id()), update).await;
        Ok(commit)
    }

    async fn conflict_workspace(&self, ctx: &PullContext, head: &RepoContext, base: &str) -> Result<ConflictWorkspace, ServiceError> {
        let base_lease = self.ctx().repositories.lease(ctx.repo().repo.repo.id).await;
        let head_lease = self.ctx().repositories.lease(head.repo.repo.id).await;
        let workspace =
            ConflictWorkspace::open(&self.merge_environment(), &head.disk_path, &ctx.repo().disk_path, &ctx.pull.head_sha, base)
                .await
                .map_err(|error| match error {
                    EditError::Rejected(message) => ServiceError::Invalid(message),
                    error => ServiceError::internal(error),
                })?;
        Ok(workspace.holding(base_lease).holding(head_lease))
    }
}
