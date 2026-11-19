//! Thin session adapters for personal progress and exact-head suggestion publication.
use super::{
    AppState, Deserialize, Form, Path, PullContext, PullUrls, Redirect, RepoContext, ReviewCommentView, ReviewSide, SignedIn, State, User,
    WebError, WebResult, open_for_post,
};
use rbit_services::pulls::review_workflow::{SuggestionSelection, suggestion_digest};

use crate::views::pulls::ReviewWorkflowView as WorkflowView;

pub(super) async fn load(
    state: &AppState,
    actor: Option<&User>,
    ctx: &PullContext,
    since: Option<&str>,
    changes: &mut Option<rbit_git::compare::Comparison>,
    comments: &mut Vec<ReviewCommentView>,
) -> WebResult<WorkflowView> {
    let mut view = WorkflowView { since_review: since == Some("review"), ..WorkflowView::default() };
    let Some(actor) = actor else {
        return Ok(view);
    };
    let head = match head_context(state, actor, ctx).await {
        Ok(head) => Some(head),
        Err(WebError::NotFound) => None,
        Err(error) => return Err(error),
    };
    view.progress = match changes.as_ref() {
        Some(comparison) => state.services.pulls.review_progress_for_comparison(actor, ctx, comparison).await?,
        None => Vec::new(),
    };
    view.can_apply = ctx.issue().is_open() && head.as_ref().is_some_and(RepoContext::can_write_content);
    if view.since_review {
        let head = head.as_ref().ok_or(WebError::NotFound)?;
        if let Some((baseline, diff)) = state.services.pulls.changes_since_review(actor, ctx, head).await? {
            view.baseline = Some(baseline);
            if let Some(changes) = changes {
                changes.files = diff.files;
                changes.truncated = diff.truncated;
            }
            comments.clear();
            view.can_apply = false;
        } else {
            view.first_review = true;
        }
    }
    if view.can_apply {
        let data = state.services.pulls.reviews(Some(actor), ctx).await?;
        for comment in comments.iter_mut() {
            if let Some((source, _)) = data.visible_comments().find(|(c, _)| c.id.0 == comment.reply_to) {
                let suggestion = suggestion_digest(&source.body);
                if suggestion.is_some()
                    && !comment.pending
                    && !comment.outdated
                    && !comment.is_reply
                    && !comment.resolution.resolved
                    && source.side == ReviewSide::Right
                    && source.commit_sha == ctx.pull.head_sha
                {
                    comment.suggestion_selection = suggestion;
                }
            }
        }
    }
    view.can_apply &= comments.iter().any(|comment| comment.suggestion_selection.is_some());
    Ok(view)
}

pub(super) async fn head_context(state: &AppState, actor: &User, ctx: &PullContext) -> WebResult<RepoContext> {
    let head = ctx.head_repo.as_ref().ok_or(WebError::NotFound)?;
    Ok(state.services.repositories.open(Some(actor), head.owner.name.as_str(), head.repo.name.as_str()).await?)
}

#[derive(Deserialize)]
pub(super) struct ProgressForm {
    path: String,
    expected_head: String,
    fingerprint: String,
    viewed: bool,
}
pub(super) async fn mark_file(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Form(form): Form<ProgressForm>,
) -> WebResult<Redirect> {
    let ctx = open_for_post(&state, &actor, &owner, &repo, &number).await?;
    state.services.pulls.mark_reviewed_file(&actor, &ctx, &form.path, &form.expected_head, &form.fingerprint, form.viewed).await?;
    Ok(Redirect::to(&PullUrls::for_context(&ctx).files))
}

pub(super) async fn apply(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Form(form): Form<std::collections::HashMap<String, String>>,
) -> WebResult<Redirect> {
    let ctx = open_for_post(&state, &actor, &owner, &repo, &number).await?;
    let head = head_context(&state, &actor, &ctx).await?;
    let expected = form.get("expected_head").ok_or_else(|| WebError::BadRequest("expected_head is required".into()))?;
    let selected = form
        .iter()
        .filter_map(|(name, digest)| name.strip_prefix("suggestion_").map(|id| (id, digest)))
        .map(|(id, digest)| {
            id.parse()
                .map(|comment_id| SuggestionSelection { comment_id, body_digest: digest.clone() })
                .map_err(|_| WebError::BadRequest("invalid suggestion id".into()))
        })
        .collect::<WebResult<Vec<_>>>()?;
    state.services.pulls.apply_suggestions(&actor, &ctx, &head, expected, &selected).await?;
    Ok(Redirect::to(&PullUrls::for_context(&ctx).files))
}
