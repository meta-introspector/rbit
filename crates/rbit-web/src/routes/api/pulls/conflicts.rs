//! Conflict APIs independently retain the caller's source-repository scope.
use super::{
    ApiError, ApiJson, ApiResult, AppState, Caller, Json, Path, PullContext, RepoContext, Right, State, TokenScope, Value, json, open,
};
use rbit_services::pulls::conflicts::ResolveConflicts;

async fn head(state: &AppState, caller: &Caller, ctx: &PullContext) -> ApiResult<RepoContext> {
    let source = ctx.head_repo.as_ref().ok_or_else(ApiError::not_found)?;
    caller.open_repo(state, source.owner.name.as_str(), source.repo.name.as_str()).await
}

pub(super) async fn preview(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
) -> ApiResult<Json<rbit_core::pull::conflicts::Preview>> {
    let actor = caller.require(TokenScope::Write)?;
    caller.require_right(Right::Push)?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    let head = head(&state, &caller, &ctx).await?;
    Ok(Json(state.services.pulls.preview_conflicts(actor, &ctx, &head).await.map_err(conflict_error)?))
}

pub(super) async fn resolve(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, number)): Path<(String, String, String)>,
    ApiJson(input): ApiJson<ResolveConflicts>,
) -> ApiResult<Json<Value>> {
    let actor = caller.require(TokenScope::Write)?;
    caller.require_right(Right::Push)?;
    let ctx = open(&state, &caller, &owner, &name, &number).await?;
    let head = head(&state, &caller, &ctx).await?;
    let commit = state.services.pulls.resolve_conflicts(actor, &ctx, &head, input).await.map_err(conflict_error)?;
    Ok(Json(json!({"commit_sha":commit})))
}

fn conflict_error(error: rbit_services::ServiceError) -> ApiError {
    match error {
        rbit_services::ServiceError::Conflict(message) => ApiError::new(axum::http::StatusCode::CONFLICT, message),
        error => ApiError::service(error, "conflicts"),
    }
}
