//! Review progress, exact previous-review diffs and suggestion batches behind current access.
mod application;
mod progress;
mod work;
pub(super) use work::Work;

use super::{PullContext, PullService};
use crate::{RepoContext, ServiceError};
use rbit_collaboration::pulls::suggestions::Suggestion;
use rbit_core::{
    User,
    issue::ReviewState,
    pull::{ReviewCommentId, ReviewSide},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SuggestionSelection {
    pub comment_id: i64,
    /// Digest of the exact comment body shown to the applier; edits invalidate selection.
    pub body_digest: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ReviewProgress {
    pub path: String,
    pub fingerprint: String,
    pub viewed: bool,
}

/// Digest only supported suggestion blocks for the browser selection control.
pub fn suggestion_digest(body: &str) -> Option<String> {
    Suggestion::parse(body).ok().flatten().map(|_| body_digest(body))
}

pub fn body_digest(body: &str) -> String {
    Sha256::digest(body.as_bytes()).iter().fold(String::with_capacity(64), |mut out, byte| {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
        out
    })
}

impl PullService {
    /// Caller adapters must independently open a fork head through their credential ceiling.
    pub fn require_review_head(ctx: &PullContext, head: &RepoContext) -> Result<(), ServiceError> {
        if ctx.pull.head_repo_id != Some(head.repo.repo.id) || !head.access.can_read() {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }

    pub(in crate::pulls) async fn fresh_review_context(
        &self,
        actor: &User,
        ctx: &PullContext,
    ) -> Result<(User, PullContext), ServiceError> {
        if ctx.repo().viewer != Some(actor.id()) {
            return Err(ServiceError::Forbidden);
        }
        let user = self.ctx().database.accounts().find_active_user(actor.id()).await?.ok_or(ServiceError::NotFound)?;
        let repos = crate::repositories::RepositoryService::new(self.ctx());
        let repo = repos.open(Some(&user), ctx.repo().repo.owner.name.as_str(), ctx.repo().repo.repo.name.as_str()).await?;
        if repo.repo.repo.id != ctx.repo().repo.repo.id {
            return Err(ServiceError::NotFound);
        }
        let current = self.open(Some(&user), repo, ctx.issue().number).await?;
        Ok((user, current))
    }

    async fn selected_suggestions(
        &self,
        ctx: &PullContext,
        selected: &[SuggestionSelection],
    ) -> Result<Vec<(String, u32, Suggestion)>, ServiceError> {
        use rbit_collaboration::pulls::suggestions::MAX_SUGGESTIONS;
        if selected.is_empty() || selected.len() > MAX_SUGGESTIONS {
            return Err(ServiceError::Invalid("choose 1–20 suggestions".into()));
        }
        let store = self.ctx().database.reviews();
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for selection in selected {
            if !seen.insert(selection.comment_id) {
                return Err(ServiceError::Invalid("duplicate suggestion".into()));
            }
            let c = store.find_comment(ctx.issue().id, ReviewCommentId(selection.comment_id)).await?.ok_or(ServiceError::NotFound)?;
            let review = store.find(ctx.issue().id, c.review_id).await?.ok_or(ServiceError::NotFound)?;
            if review.state == ReviewState::Pending {
                return Err(ServiceError::NotFound);
            }
            if c.commit_sha != ctx.pull.head_sha
                || c.outdated
                || c.side != ReviewSide::Right
                || c.in_reply_to.is_some()
                || c.resolved_at.is_some()
                || body_digest(&c.body) != selection.body_digest
            {
                return Err(ServiceError::Invalid("suggestion changed, is outdated, or cannot be applied".into()));
            }
            let suggestion = Suggestion::parse(&c.body)
                .map_err(|e| ServiceError::Invalid(e.into()))?
                .ok_or_else(|| ServiceError::Invalid("comment has no suggestion".into()))?;
            result.push((c.path, c.line, suggestion));
        }
        Ok(result)
    }
}
