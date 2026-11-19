//! Pull request diff page and personal review workflow state.
use super::{
    AppState, Deserialize, DiffLimits, IntoResponse, Opened, Page, Path, PullCountsView, PullFileView, PullFilesPage, PullHeaderView,
    PullMessage, PullUrls, Query, RepoPage, Response, State, Viewer, WebResult, open_thread, review_form, review_views, review_workflow,
    title, titled,
};

#[derive(Debug, Deserialize)]
pub struct FilesQuery {
    since: Option<String>,
    /// A line anchor: show the inline comment form there.
    comment: Option<String>,
}

/// `GET /{owner}/{repo}/pull/{n}/files[?comment=<anchor>]`
pub(super) async fn show(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Query(query): Query<FilesQuery>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    let ctx = match open_thread(&state, viewer.user(), loaded.ctx.clone(), &number).await? {
        Opened::Pull(ctx) => *ctx,
        Opened::Issue(url) => return Ok(crate::routes::timeline::moved_to_kind(&url)),
    };
    let user = viewer.user();
    let pulls = &state.services.pulls;
    // The diff and the reviews are independent; a failed diff shows an empty tab.
    let (changes, reviews) = tokio::join!(pulls.changes(&ctx, DiffLimits::default()), async {
        let data = pulls.reviews(user, &ctx).await?;
        let (_, comments) = review_views(&state, &ctx, &data, user).await?;
        WebResult::Ok((data, comments))
    });
    let mut changes = changes.ok();
    let (data, mut comments) = reviews?;
    let workflow = review_workflow::load(&state, user, &ctx, query.since.as_deref(), &mut changes, &mut comments).await?;
    let counts = changes.as_ref().map_or_else(|| PullCountsView::known(0, 0), |c| PullCountsView::known(c.ahead, c.files.len()));
    let urls = PullUrls::for_context(&ctx);
    let can_comment = user.is_some() && ctx.thread.permissions.can_comment && workflow.baseline.is_none();
    // The views take the diff's lines instead of copying them.
    let (mut files, additions, deletions, truncated) = match changes {
        Some(changes) => {
            let (additions, deletions, truncated) = (changes.additions(), changes.deletions(), changes.truncated);
            let form_at = query.comment.as_deref();
            (
                PullFileView::list(changes.files, &changes.head.id, &loaded.urls, &comments, form_at, can_comment),
                additions,
                deletions,
                truncated,
            )
        }
        None => (Vec::new(), 0, 0, false),
    };
    for file in files.iter_mut().filter(|_| workflow.baseline.is_none()) {
        file.progress = workflow.progress.iter().find(|p| p.path == file.path).map(|p| (p.fingerprint.clone(), p.viewed));
    }
    let view = PullFilesPage {
        workflow,
        expected_head: ctx.pull.head_sha.clone(),
        page: titled(page, |locale| title(&ctx, Some(PullMessage::FilesChanged), locale)),
        header: loaded.header(&state, "pulls"),
        pull: PullHeaderView::new(&ctx, counts, None, "files"),
        files,
        additions,
        deletions,
        truncated,
        can_comment,
        signed_in: user.is_some(),
        sign_in_url: format!("/login?next={}", urls.files),
        review_form: review_form(&ctx, &data, user),
        outdated_count: comments
            .iter()
            .flat_map(|comment| std::iter::once(comment).chain(&comment.replies))
            .filter(|comment| comment.outdated)
            .count(),
    };
    Ok(view.into_response())
}
