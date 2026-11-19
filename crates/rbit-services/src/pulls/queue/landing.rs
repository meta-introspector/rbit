//! Final queue landing: hold the repository in place and recheck the queuer.

use super::{
    MergeError, MergeRecord, PullContext, PullService, PushSource, QueueEntry, ServiceError, StateReason, TimelineEvent, Timestamp, User,
    strategy_of,
};

impl PullService {
    /// Lands `candidate_sha` on the base by compare-and-swap from `base_tip`, and records the
    /// pull request as merged by it. A base that moved means a rebuild on the next step.
    pub(super) async fn land_entry(
        &self,
        user: &User,
        ctx: &PullContext,
        entry: &QueueEntry,
        candidate_sha: &str,
        base_tip: &str,
    ) -> Result<(), ServiceError> {
        let service = self.clone();
        let user = user.clone();
        let ctx = ctx.clone();
        let entry = entry.clone();
        let candidate = candidate_sha.to_owned();
        let base = base_tip.to_owned();
        self.inner
            .publication_work
            .submit(async move { service.land_entry_after(&user, &ctx, &entry, (&candidate, &base), std::future::ready(())).await })?
            .await
            .map_err(ServiceError::internal)?
    }

    // Private static seam: production passes Ready, while tests suspend the
    // captured principal after the lease and immediately before authorization.
    async fn land_entry_after(
        &self,
        user: &User,
        ctx: &PullContext,
        entry: &QueueEntry,
        commits: (&str, &str),
        after_lease: impl std::future::Future<Output = ()> + Send,
    ) -> Result<(), ServiceError> {
        let (candidate_sha, base_tip) = commits;
        let pending = self.inner.pushes.begin(ctx.repo(), PushSource::User(user.id())).await;
        let lease = self.ctx().repositories.lease(entry.repo).await;
        after_lease.await;
        // The earlier queue evaluation awaited Git and SQL work. Do not let its
        // User snapshot authorize this write after the account was suspended.
        if self.ctx().database.accounts().find_active_user(entry.queued_by).await?.is_none() {
            drop(lease);
            return self.leave(entry, "failed", "whoever queued it is gone or suspended").await;
        }
        let readiness = self.merge_readiness(Some(user), ctx, None).await?;
        if readiness.base_tip.as_deref() != Some(base_tip) {
            self.nudge(entry.repo, entry.base_branch.clone());
            return Ok(());
        }
        if let Some(check) = readiness.evaluation.checks.iter().find(|check| {
            matches!(check.requirement, rbit_core::pull::Requirement::CodeOwners { .. } | rbit_core::pull::Requirement::Security(_))
                && !check.met
        }) {
            return self.leave(entry, "failed", &check.requirement.describe(false)).await;
        }
        let current = self.ctx().database.pulls().find(ctx.issue().id).await?.ok_or(ServiceError::NotFound)?;
        if current.head_sha != entry.head_sha || current.base_branch != entry.base_branch {
            return self.leave(entry, "failed", "pull request changed before landing").await;
        }
        let landed = rbit_git::merge::guarded::land(
            &self.merge_environment(),
            &ctx.repo().disk_path,
            &ctx.pull.base_branch,
            candidate_sha,
            base_tip,
            self.security_publication_fence(user, ctx, base_tip),
        )
        .await;
        let landed = match landed {
            Ok(()) => Ok(()),
            Err(rbit_git::merge::guarded::Error::Git(error)) => Err(error),
            Err(rbit_git::merge::guarded::Error::Guard(error)) => {
                drop(lease);
                return self.leave(entry, "failed", &error.to_string()).await;
            }
        };
        drop(lease);
        match landed {
            Ok(()) => {}
            Err(MergeError::BaseMoved) => {
                self.nudge(entry.repo, entry.base_branch.clone());
                return Ok(());
            }
            Err(error) => return Err(ServiceError::internal(error)),
        }
        let strategy = strategy_of(&entry.strategy);
        let record = MergeRecord {
            merged_by: Some(user.id()),
            strategy: Some(strategy),
            commit: candidate_sha,
            base: base_tip,
            head: &entry.head_sha,
            at: Timestamp::now(),
        };
        if self.ctx().database.pulls().record_merge(ctx.issue().id, &record).await? {
            let event = TimelineEvent::Merged { commit: candidate_sha.to_owned(), strategy: Some(strategy) };
            self.inner.timeline.writer().close_as_system(ctx.issue().id, Some(user.id()), StateReason::Merged, event).await?;
        }
        self.inner.pushes.complete(pending).await;
        self.ctx().database.actions_runs().queue_finish(entry.id, "landed", None, Timestamp::now()).await?;
        self.forget_candidate(ctx.repo(), entry).await;
        tracing::info!(number = entry.number, commit = %candidate_sha, "merge queue candidate landed");
        self.nudge(entry.repo, entry.base_branch.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests;
