//! REST adapters retain the caller's scope and fork repository ceiling for review operations.
use super::{
    ApiError, ApiJson, ApiResult, AppState, Caller, Deserialize, FileDiffBody, Json, LineKind, Path, PullContext, RepoContext, Right,
    State, StatusCode, TokenScope, Value, json, open,
};
use rbit_services::pulls::review_workflow::SuggestionSelection;

async fn head(state: &AppState, caller: &Caller, ctx: &PullContext) -> ApiResult<RepoContext> {
    let head = ctx.head_repo.as_ref().ok_or_else(ApiError::not_found)?;
    caller.open_repo(state, head.owner.name.as_str(), head.repo.name.as_str()).await
}

pub(super) async fn progress(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let actor = caller.require_user()?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    let files = state.services.pulls.review_progress(actor, &ctx).await?;
    Ok(Json(json!({"head_sha":ctx.pull.head_sha,"files":files})))
}

#[derive(Deserialize)]
pub(super) struct Mark {
    path: String,
    expected_head: String,
    fingerprint: String,
    viewed: bool,
}
pub(super) async fn mark(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
    ApiJson(input): ApiJson<Mark>,
) -> ApiResult<StatusCode> {
    let actor = caller.require(TokenScope::Write)?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    state.services.pulls.mark_reviewed_file(actor, &ctx, &input.path, &input.expected_head, &input.fingerprint, input.viewed).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(super) struct Apply {
    expected_head: String,
    suggestions: Vec<SuggestionSelection>,
}
pub(super) async fn apply(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
    ApiJson(input): ApiJson<Apply>,
) -> ApiResult<Json<Value>> {
    let actor = caller.require(TokenScope::Write)?;
    caller.require_right(Right::Push)?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    let head = head(&state, &caller, &ctx).await?;
    let commit = state.services.pulls.apply_suggestions(actor, &ctx, &head, &input.expected_head, &input.suggestions).await?;
    Ok(Json(json!({"commit_sha":commit})))
}

pub(super) async fn changes(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let actor = caller.require_user()?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    let head = head(&state, &caller, &ctx).await?;
    let Some((baseline, changes)) = state.services.pulls.changes_since_review(actor, &ctx, &head).await? else {
        return Ok(Json(json!({"first_review":true,"head_sha":ctx.pull.head_sha,"baseline":null,"files":[],"truncated":false})));
    };
    let files:Vec<_> = changes.files.iter().map(|file| {
        let patch = match &file.body { FileDiffBody::Text(hunks) => Some(hunks.iter().map(|h| {
            let lines = h.lines.iter().map(|line| { let sign=match line.kind { LineKind::Added=>'+',LineKind::Removed=>'-',LineKind::Context=>' ' };format!("{sign}{}",line.text) });
            std::iter::once(h.header.clone()).chain(lines).collect::<Vec<_>>().join("\n")
        }).collect::<Vec<_>>().join("\n")),_=>None };
        json!({"filename":file.path,"previous_filename":file.old_path,"additions":file.additions,"deletions":file.deletions,"patch":patch})
    }).collect();
    Ok(Json(json!({"first_review":false,"head_sha":ctx.pull.head_sha,"baseline":baseline,"files":files,"truncated":changes.truncated})))
}
