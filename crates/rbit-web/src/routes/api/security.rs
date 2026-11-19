//! Security findings API: writer-only visibility and server-checked upload provenance.
mod automation;
use super::{
    ApiMethod::{Get, Post, Put},
    ApiRoute,
    auth::Caller,
    error::{ApiError, ApiResult},
    json::ApiJson,
    route,
};
use crate::AppState;
use axum::{
    Json,
    body::Bytes,
    extract::{Path, Query, State, rejection::BytesRejection},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rbit_core::TokenScope;
use rbit_services::security::{Settings, Uploader};
use serde::Deserialize;
pub(super) fn routes() -> Vec<ApiRoute> {
    let mut routes = vec![
        route(Get, "/repos/{owner}/{repo}/security/settings", settings),
        route(Put, "/repos/{owner}/{repo}/security/settings", configure),
        route(Get, "/repos/{owner}/{repo}/security/scans", scans),
        route(Post, "/repos/{owner}/{repo}/security/scans", begin),
        route(Post, "/repos/{owner}/{repo}/security/scans/{id}/sarif", finish),
        route(Get, "/repos/{owner}/{repo}/security/scans/{id}/findings", findings),
        route(Post, "/repos/{owner}/{repo}/security/findings/{id}/dismissal", dismiss),
    ];
    routes.extend(automation::routes());
    routes
}
fn response(value: impl serde::Serialize) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
}
fn actor(caller: &Caller) -> ApiResult<Uploader<'_>> {
    match caller.task() {
        Some(task) => Ok(Uploader::Task(task)),
        None => Ok(Uploader::User(caller.require(TokenScope::Admin)?)),
    }
}
#[derive(Debug, Deserialize)]
struct Start {
    commit_sha: String,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Dismissal {
    pub reason: String,
    #[serde(default)]
    pub note: String,
}
#[derive(Debug, Default, Deserialize)]
struct Page {
    #[serde(default)]
    offset: u32,
}
async fn settings(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(state.services.security.configuration(&repo).await?))
}
async fn configure(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(input): ApiJson<Settings>,
) -> ApiResult<StatusCode> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    state.services.security.configure(caller.require(TokenScope::Admin)?, &repo, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn scans(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(state.services.security.scans(&repo).await?))
}
async fn begin(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(input): ApiJson<Start>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    let scan = state.services.security.begin(actor(&caller)?, &repo, &input.commit_sha).await?;
    Ok((StatusCode::CREATED, response(scan)).into_response())
}
async fn finish(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, id)): Path<(String, String, i64)>,
    bytes: Result<Bytes, BytesRejection>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    let actor = actor(&caller)?;
    let bytes = bytes.map_err(|error| ApiError::new(error.status(), "Invalid or oversized SARIF body"))?;
    Ok(response(state.services.security.finish(actor, &repo, id, bytes.to_vec()).await?))
}
async fn findings(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, id)): Path<(String, String, i64)>,
    Query(page): Query<Page>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    let (scan, findings) = state.services.security.findings(&repo, id, page.offset).await?;
    Ok(response(serde_json::json!({"scan": scan, "findings": findings, "offset": page.offset, "per_page":50})))
}
async fn dismiss(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, id)): Path<(String, String, i64)>,
    ApiJson(input): ApiJson<Dismissal>,
) -> ApiResult<StatusCode> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    state.services.security.dismiss(caller.require(TokenScope::Admin)?, &repo, id, &input.reason, &input.note).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
