//! REST project boards, restricted to unrestricted personal tokens and current members.

use super::{
    ApiMethod::{Delete, Get, Patch, Post, Put},
    ApiRoute,
    auth::Caller,
    error::ApiResult,
    json::ApiJson,
    route,
};
use crate::AppState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use rbit_core::TokenScope;
use rbit_services::projects::{Board, BoardInput, BoardPage, Filter};
use serde::Deserialize;

mod planning;

pub(super) fn routes() -> Vec<ApiRoute> {
    let mut routes = vec![
        route(Get, "/orgs/{org}/projects", list),
        route(Post, "/orgs/{org}/projects", create),
        route(Get, "/orgs/{org}/projects/{id}", show),
        route(Patch, "/orgs/{org}/projects/{id}", update),
        route(Delete, "/orgs/{org}/projects/{id}", delete),
        route(Post, "/orgs/{org}/projects/{id}/cards", add),
        route(Patch, "/orgs/{org}/projects/{id}/cards/{issue}", move_card),
        route(Delete, "/orgs/{org}/projects/{id}/cards/{issue}", remove),
    ];
    routes.extend(planning::routes());
    routes
}

async fn org(state: &AppState, caller: &Caller, name: &str, scope: TokenScope) -> ApiResult<i64> {
    caller.require_unlimited()?;
    let user = caller.require(scope)?;
    Ok(state.services.orgs.open(Some(user), name).await?.org.account.id.0)
}

pub async fn list(State(state): State<AppState>, caller: Caller, Path(name): Path<String>) -> ApiResult<Json<Vec<Board>>> {
    let org = org(&state, &caller, &name, TokenScope::Read).await?;
    Ok(Json(state.services.projects.list(caller.require_user()?, org).await?))
}
pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path(name): Path<String>,
    ApiJson(input): ApiJson<BoardInput>,
) -> ApiResult<(StatusCode, Json<Board>)> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    Ok((StatusCode::CREATED, Json(state.services.projects.create(caller.require_user()?, org, &input).await?)))
}
pub async fn show(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    Query(filter): Query<Filter>,
) -> ApiResult<Json<BoardPage>> {
    let org = org(&state, &caller, &name, TokenScope::Read).await?;
    let mut data = state.services.projects.page(caller.require_user()?, org, id, &filter).await?;
    data.can_manage &= caller.require(TokenScope::Admin).is_ok();
    planning::cap_views(&caller, &mut data.views);
    Ok(Json(data))
}
#[derive(Deserialize)]
pub struct Update {
    revision: i64,
    #[serde(flatten)]
    input: BoardInput,
}
pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    ApiJson(input): ApiJson<Update>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    state.services.projects.update(caller.require_user()?, org, id, input.revision, &input.input).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct Revision {
    revision: i64,
}
pub async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    Query(input): Query<Revision>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    state.services.projects.delete(caller.require_user()?, org, id, input.revision).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct Add {
    repository: String,
    number: i64,
}
pub async fn add(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    ApiJson(input): ApiJson<Add>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Write).await?;
    state.services.projects.add_named(caller.require_user()?, org, id, &input.repository, input.number).await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
pub struct Move {
    revision: i64,
    column: i64,
}
pub async fn move_card(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, issue)): Path<(String, i64, i64)>,
    ApiJson(input): ApiJson<Move>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Write).await?;
    state.services.projects.move_card(caller.require_user()?, org, id, issue, input.revision, Some(input.column)).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub async fn remove(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, issue)): Path<(String, i64, i64)>,
    Query(input): Query<Revision>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Write).await?;
    state.services.projects.move_card(caller.require_user()?, org, id, issue, input.revision, None).await?;
    Ok(StatusCode::NO_CONTENT)
}
