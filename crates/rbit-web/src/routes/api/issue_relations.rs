//! Permission-filtered hierarchy/dependency resources; limited and task tokens are refused.
use super::{
    ApiMethod::{Delete, Get, Post},
    ApiRoute,
    auth::Caller,
    error::{ApiError, ApiResult},
    json::ApiJson,
    route,
};
use crate::AppState;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rbit_core::{IssueNumber, TokenScope};
use rbit_services::{
    IssueContext,
    issue_relations::{RelationInput, Relations},
};
use serde::Serialize;

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/repos/{owner}/{repo}/issues/{number}/relations", get),
        route(Post, "/repos/{owner}/{repo}/issues/{number}/relations", add),
        route(Delete, "/repos/{owner}/{repo}/issues/{number}/relations/{edge}", remove),
    ]
}
async fn open(state: &AppState, caller: &Caller, owner: &str, repo: &str, number: &str) -> ApiResult<IssueContext> {
    // A related repository is not covered by a token limited to the source repository.
    caller.require_unlimited()?;
    let repo = caller.open_repo(state, owner, repo).await?;
    let number = IssueNumber::parse(number).map_err(|_| ApiError::not_found())?;
    Ok(state.services.timeline.open(caller.user(), repo, number).await?)
}
pub async fn get(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, number)): Path<(String, String, String)>,
) -> ApiResult<Json<Relations>> {
    let ctx = open(&state, &caller, &owner, &repo, &number).await?;
    let mut data = state.services.issue_relations.get(caller.user(), ctx.issue.id).await?;
    let admitted_write = ctx.repo.can_write_content();
    data.can_edit &= admitted_write;
    for item in &mut data.items {
        item.can_remove &= admitted_write;
    }
    Ok(Json(data))
}
#[derive(Serialize)]
pub struct Created {
    id: i64,
}
pub async fn add(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, number)): Path<(String, String, String)>,
    ApiJson(input): ApiJson<RelationInput>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    let user = caller.require(TokenScope::Write)?;
    let ctx = open(&state, &caller, &owner, &repo, &number).await?;
    let id = state.services.issue_relations.add(user, ctx.issue.id, &input).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}
pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, number, edge)): Path<(String, String, String, i64)>,
) -> ApiResult<StatusCode> {
    let user = caller.require(TokenScope::Write)?;
    let ctx = open(&state, &caller, &owner, &repo, &number).await?;
    state.services.issue_relations.remove(user, ctx.issue.id, edge).await?;
    Ok(StatusCode::NO_CONTENT)
}
