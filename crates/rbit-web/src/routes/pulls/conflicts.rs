//! Session conflict preview and publication with origin protection and retained refused drafts.
use super::{
    AppState, Form, IntoResponse, Page, Path, PullContext, PullCountsView, PullHeaderView, PullUrls, Redirect, RepoPage, Response,
    ServiceError, SignedIn, State, Viewer, WebError, WebResult, open_for_post,
};
use crate::views::pulls::conflicts::ConflictPage;
use axum::http::StatusCode;
use rbit_core::pull::conflicts::{MAX_FILES, Resolution};
use rbit_localization::ReviewMessage;
use rbit_services::pulls::conflicts::ResolveConflicts;
use std::collections::HashMap;

pub(super) async fn show(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    viewer: Viewer,
    page: Page,
    Path((owner, repo, number)): Path<(String, String, String)>,
) -> WebResult<Response> {
    let ctx = open_for_post(&state, &actor, &owner, &repo, &number).await?;
    let head = super::review_workflow::head_context(&state, &actor, &ctx).await?;
    let (preview, notice) = match state.services.pulls.preview_conflicts(&actor, &ctx, &head).await {
        Ok(preview) => (Some(preview), None),
        Err(ServiceError::Invalid(_)) => (None, Some(ReviewMessage::ConflictUnavailable)),
        Err(error) => return Err(error.into()),
    };
    let mut view = page_view(&state, &viewer, page, &ctx).await?;
    view.preview = preview;
    view.notice = notice;
    Ok(view.into_response())
}

pub(super) async fn apply(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    viewer: Viewer,
    page: Page,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Form(form): Form<HashMap<String, String>>,
) -> WebResult<Response> {
    let ctx = open_for_post(&state, &actor, &owner, &repo, &number).await?;
    let head = super::review_workflow::head_context(&state, &actor, &ctx).await?;
    let input = parse(form)?;
    match state.services.pulls.resolve_conflicts(&actor, &ctx, &head, input.clone()).await {
        Ok(_) => Ok(Redirect::to(&PullUrls::for_context(&ctx).files).into_response()),
        Err(error @ (ServiceError::Invalid(_) | ServiceError::Conflict(_))) => {
            let mut view = page_view(&state, &viewer, page, &ctx).await?;
            view.notice = Some(ReviewMessage::ConflictDraftRetained);
            view.draft = input.resolutions;
            let status = if matches!(error, ServiceError::Conflict(_)) { StatusCode::CONFLICT } else { StatusCode::UNPROCESSABLE_ENTITY };
            Ok((status, view).into_response())
        }
        Err(error) => Err(error.into()),
    }
}

fn parse(mut form: HashMap<String, String>) -> WebResult<ResolveConflicts> {
    let mut take = |key: &str| form.remove(key).ok_or_else(|| WebError::BadRequest("incomplete conflict resolution".into()));
    let expected_head = take("expected_head")?;
    let expected_base = take("expected_base")?;
    let count = take("count")?
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0 && *n <= MAX_FILES)
        .ok_or_else(|| WebError::BadRequest("invalid conflict count".into()))?;
    let resolutions = (0..count)
        .map(|i| Ok(Resolution { path: take(&format!("path_{i}"))?, content: take(&format!("content_{i}"))? }))
        .collect::<WebResult<Vec<_>>>()?;
    Ok(ResolveConflicts { expected_head, expected_base, resolutions })
}

async fn page_view(state: &AppState, viewer: &Viewer, page: Page, ctx: &PullContext) -> WebResult<ConflictPage> {
    let repo = &ctx.repo().repo;
    let loaded = RepoPage::load(state, viewer, repo.owner.name.as_str(), repo.repo.name.as_str()).await?;
    let title = ReviewMessage::ResolveConflicts.localized(page.locale).to_string();
    Ok(ConflictPage {
        page: page.titled(title),
        header: loaded.header(state, "pulls"),
        pull: PullHeaderView::new(ctx, PullCountsView::default(), None, "files"),
        preview: None,
        notice: None,
        draft: Vec::new(),
    })
}
