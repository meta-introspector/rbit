//! Bounded suggestion preparation and fenced authorization through a single Git CAS.
#[cfg(test)]
mod tests;
use super::{PullContext, PullService, RepoContext, ServiceError, SuggestionSelection};
use rbit_collaboration::pulls::suggestions::{MAX_BATCH_BYTES, apply};
use rbit_core::{PushSource, RefUpdate, RepoId, Timestamp, User, file_edit::FilePath};
use rbit_git::edit::{Change, EditError, EditSpec, EditWorkspace};
use rbit_storage::sqlite::reviews::ReviewWriteFence;

impl PullService {
    pub async fn apply_suggestions(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        expected_head: &str,
        selected: &[SuggestionSelection],
    ) -> Result<String, ServiceError> {
        if selected.is_empty() || selected.len() > 20 || expected_head.len() > 64 || selected.iter().any(|s| s.body_digest.len() != 64) {
            return Err(ServiceError::Invalid("invalid suggestion batch".into()));
        }
        let (service, actor, ctx, head, expected, selected) =
            (self.clone(), actor.clone(), ctx.clone(), head.clone(), expected_head.to_owned(), selected.to_vec());
        self.inner
            .review_work
            .submit(async move { Box::pin(service.apply_suggestions_inner(&actor, &ctx, &head, &expected, &selected)).await })?
            .await
            .map_err(ServiceError::internal)?
    }

    async fn apply_suggestions_inner(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        expected_head: &str,
        selected: &[SuggestionSelection],
    ) -> Result<String, ServiceError> {
        Self::require_review_head(ctx, head)?;
        if head.viewer != Some(actor.id()) || !head.can_write_content() {
            return Err(ServiceError::Forbidden);
        }
        let (actor, current) = self.fresh_review_context(actor, ctx).await?;
        self.suggestion_authority(&actor, &current, head, expected_head).await?;
        let selected_changes = self.selected_suggestions(&current, selected).await?;
        let lease = self.ctx().repositories.lease(head.repo.repo.id).await;
        let workspace =
            EditWorkspace::open(&self.merge_environment(), &head.disk_path).await.map_err(ServiceError::internal)?.holding(lease);
        let changes = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            self.prepare_suggestions(head, expected_head, selected_changes, &workspace),
        )
        .await
        .map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })??;
        let identity = self.identity(&actor);
        let spec = EditSpec {
            branch: &current.pull.head_branch,
            parent: expected_head,
            new_branch: None,
            changes: &changes,
            message: "Apply review suggestions",
            author: &identity,
            at: Timestamp::now(),
        };
        let guard = async {
            let fence = self
                .suggestion_fence(&actor, &current, head, expected_head, selected)
                .await
                .map_err(|e| EditError::Rejected(e.to_string()))?;
            Ok(fence)
        };
        let committed = workspace.commit_guarded(&spec, guard).await.map_err(|error| match error {
            EditError::BranchMoved => ServiceError::Conflict("the reviewed branch moved; refresh before applying suggestions"),
            EditError::Rejected(reason) => ServiceError::Invalid(reason),
            EditError::NothingChanged => ServiceError::Invalid("suggestions make no changes".into()),
            error => ServiceError::internal(error),
        })?;
        let update = RefUpdate {
            name: format!("refs/heads/{}", committed.branch),
            before: Some(expected_head.to_owned()),
            after: Some(committed.commit.clone()),
        };
        self.record_review_push(head.repo.repo.id, PushSource::User(actor.id()), update).await;
        Ok(committed.commit)
    }

    pub(in crate::pulls) async fn record_review_push(&self, repo: RepoId, source: PushSource, update: RefUpdate) {
        // commit_guarded has dropped its writer fence. Project before broadcasting so
        // the redirect sees the new head and the listener observes an already-current pull.
        if let Err(error) = self.handle_push(repo, source, std::slice::from_ref(&update)).await {
            // The commit is accepted: preserve push delivery and do not invite a retry
            // of the mutation merely because a secondary projection failed.
            tracing::warn!(%error, repo = repo.0, "cannot update pull requests after review branch publication");
        }
        self.inner.pushes.record(repo, source, vec![update]).await;
    }

    async fn suggestion_fence(
        &self,
        actor: &User,
        ctx: &PullContext,
        head: &RepoContext,
        expected: &str,
        selected: &[SuggestionSelection],
    ) -> Result<ReviewWriteFence, ServiceError> {
        let fence = self.ctx().database.reviews().publication_fence().await?;
        let (actor, current) = self.fresh_review_context(actor, ctx).await?;
        if current.pull.head_branch != ctx.pull.head_branch {
            return Err(ServiceError::Conflict("the source branch changed"));
        }
        self.suggestion_authority(&actor, &current, head, expected).await?;
        self.selected_suggestions(&current, selected).await?;
        Ok(fence)
    }

    pub(in crate::pulls) async fn suggestion_authority(
        &self,
        actor: &User,
        ctx: &PullContext,
        original: &RepoContext,
        expected: &str,
    ) -> Result<(), ServiceError> {
        if !ctx.issue().is_open() || ctx.pull.head_sha != expected || ctx.pull.head_repo_id != Some(original.repo.repo.id) {
            return Err(ServiceError::Conflict("the pull request changed; refresh before applying suggestions"));
        }
        ctx.repo().ensure_writable()?;
        crate::agents::require_capability(
            self.ctx(),
            actor,
            ctx.repo().repo.repo.id,
            rbit_identity_models::agents::Capability::PullRequests,
        )
        .await?;
        let mut head = crate::repositories::RepositoryService::new(self.ctx())
            .open(Some(actor), original.repo.owner.name.as_str(), original.repo.repo.name.as_str())
            .await?;
        if head.repo.repo.id != original.repo.repo.id || !head.can_write_content() {
            return Err(ServiceError::Forbidden);
        }
        if self.ctx().database.mirrors().is_mirror(head.repo.repo.id).await? {
            return Err(ServiceError::Invalid("mirror repositories are read-only".into()));
        }
        // Applying a suggestion never invokes the administrator protection bypass.
        head.access = rbit_core::RepoAccess::Write;
        if let Some(reason) = self.inner.protection.branch_update_refusal(&head, actor, &ctx.pull.head_branch).await? {
            return Err(ServiceError::Invalid(reason));
        }
        let actual = head.reader.commit_id(format!("refs/heads/{}", ctx.pull.head_branch)).await?;
        if actual != expected {
            return Err(ServiceError::Conflict("the reviewed branch moved; refresh before applying suggestions"));
        }
        Ok(())
    }

    async fn prepare_suggestions(
        &self,
        head: &RepoContext,
        expected: &str,
        selected: Vec<(String, u32, rbit_collaboration::pulls::suggestions::Suggestion)>,
        workspace: &EditWorkspace,
    ) -> Result<Vec<Change>, ServiceError> {
        let mut files = std::collections::BTreeMap::<String, Vec<_>>::new();
        for (path, line, suggestion) in selected {
            files.entry(path).or_default().push((line, suggestion));
        }
        let mut changes = Vec::new();
        let mut total = 0;
        for (path, suggestions) in files {
            let path = FilePath::parse(&path).map_err(|e| ServiceError::Invalid(e.to_string()))?;
            let blob = head.reader.review_blob(expected.to_owned(), path.as_str().to_owned(), MAX_BATCH_BYTES).await?;
            let source = blob.text().ok_or_else(|| ServiceError::Invalid("suggestions require bounded text files".into()))?;
            if rbit_core::lfs::LfsPointer::parse(source.as_bytes()).is_some() {
                return Err(ServiceError::Invalid("cannot apply suggestions to LFS pointers".into()));
            }
            let content = apply(source, &suggestions).map_err(|e| ServiceError::Invalid(e.into()))?;
            total += source.len() + content.len();
            if total > MAX_BATCH_BYTES {
                return Err(ServiceError::Invalid("suggestion batch exceeds 1 MiB".into()));
            }
            let blob = workspace.write_blob(content.as_slice(), MAX_BATCH_BYTES as u64).await.map_err(ServiceError::internal)?;
            changes.push(Change::Update { path, blob });
        }
        Ok(changes)
    }
}
