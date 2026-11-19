//! Planning mutations preserve credential ceilings in addition to live store authority.
use super::{
    ApiJson, ApiResult, ApiRoute, AppState, Caller, Delete, Deserialize, Get, Json, Patch, Path, Post, Put, Query, Revision, State,
    StatusCode, TokenScope, org, route,
};
use rbit_services::projects::{Iteration, IterationInput, PlanningInput, SavedView, ViewInput};

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Put, "/orgs/{org}/projects/{id}/cards/{issue}/planning", plan),
        route(Get, "/orgs/{org}/projects/{id}/iterations", iterations),
        route(Post, "/orgs/{org}/projects/{id}/iterations", create_iteration),
        route(Patch, "/orgs/{org}/projects/{id}/iterations/{iteration}", update_iteration),
        route(Delete, "/orgs/{org}/projects/{id}/iterations/{iteration}", delete_iteration),
        route(Get, "/orgs/{org}/projects/{id}/views", views),
        route(Post, "/orgs/{org}/projects/{id}/views", create_view),
        route(Patch, "/orgs/{org}/projects/{id}/views/{view}", update_view),
        route(Delete, "/orgs/{org}/projects/{id}/views/{view}", delete_view),
    ]
}
pub(super) fn cap_views(caller: &Caller, views: &mut [SavedView]) {
    for view in views {
        view.can_edit &= caller.require(if view.shared { TokenScope::Admin } else { TokenScope::Write }).is_ok();
    }
}
#[derive(serde::Serialize)]
struct Created {
    id: i64,
}
async fn plan(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, issue)): Path<(String, i64, i64)>,
    ApiJson(input): ApiJson<PlanningInput>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Write).await?;
    state.services.projects.plan_card(caller.require_user()?, org, id, issue, &input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn iterations(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
) -> ApiResult<Json<Vec<Iteration>>> {
    let org = org(&state, &caller, &name, TokenScope::Read).await?;
    Ok(Json(state.services.projects.iterations(caller.require_user()?, org, id).await?))
}
async fn create_iteration(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    ApiJson(input): ApiJson<IterationInput>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    let id = state.services.projects.save_iteration(caller.require_user()?, org, id, None, &input).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}
#[derive(Deserialize)]
struct IterationUpdate {
    revision: i64,
    #[serde(flatten)]
    input: IterationInput,
}
async fn update_iteration(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, iteration)): Path<(String, i64, i64)>,
    ApiJson(input): ApiJson<IterationUpdate>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    state.services.projects.save_iteration(caller.require_user()?, org, id, Some((iteration, input.revision)), &input.input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_iteration(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, iteration)): Path<(String, i64, i64)>,
    Query(input): Query<Revision>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Admin).await?;
    state.services.projects.delete_iteration(caller.require_user()?, org, id, iteration, input.revision).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn views(State(state): State<AppState>, caller: Caller, Path((name, id)): Path<(String, i64)>) -> ApiResult<Json<Vec<SavedView>>> {
    let org = org(&state, &caller, &name, TokenScope::Read).await?;
    let mut views = state.services.projects.views(caller.require_user()?, org, id).await?;
    cap_views(&caller, &mut views);
    Ok(Json(views))
}
async fn create_view(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id)): Path<(String, i64)>,
    ApiJson(input): ApiJson<ViewInput>,
) -> ApiResult<(StatusCode, Json<Created>)> {
    let scope = if input.shared { TokenScope::Admin } else { TokenScope::Write };
    let org = org(&state, &caller, &name, scope).await?;
    let id = state.services.projects.save_view(caller.require_user()?, org, id, None, &input).await?;
    Ok((StatusCode::CREATED, Json(Created { id })))
}
#[derive(Deserialize)]
struct ViewUpdate {
    revision: i64,
    #[serde(flatten)]
    input: ViewInput,
}
async fn update_view(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, view)): Path<(String, i64, i64)>,
    ApiJson(input): ApiJson<ViewUpdate>,
) -> ApiResult<StatusCode> {
    let scope = if input.input.shared { TokenScope::Admin } else { TokenScope::Write };
    let org = org(&state, &caller, &name, scope).await?;
    state.services.projects.save_view(caller.require_user()?, org, id, Some((view, input.revision)), &input.input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_view(
    State(state): State<AppState>,
    caller: Caller,
    Path((name, id, view)): Path<(String, i64, i64)>,
    Query(input): Query<Revision>,
) -> ApiResult<StatusCode> {
    let org = org(&state, &caller, &name, TokenScope::Write).await?;
    // Sharing scope is immutable and IDs are never reused, so this read cannot race
    // a personal view into a shared mutation under a narrower credential.
    let views = state.services.projects.views(caller.require_user()?, org, id).await?;
    let current = views.iter().find(|v| v.id == view).ok_or(rbit_services::ServiceError::NotFound)?;
    if current.shared {
        caller.require(TokenScope::Admin)?;
    }
    state.services.projects.delete_view(caller.require_user()?, org, id, view, input.revision).await?;
    Ok(StatusCode::NO_CONTENT)
}
