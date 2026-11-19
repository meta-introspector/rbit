//! Thin automation adapters; metadata disclosure and publication authority live in the service.
use super::{
    ApiJson, ApiResult, ApiRoute, AppState, Caller, Get, Path, Post, Put, Query, Response, State, StatusCode, TokenScope, actor, response,
    route,
};
use rbit_services::security::automation::AutomationSettings;
use serde::Deserialize;

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/repos/{owner}/{repo}/security/automation", settings),
        route(Put, "/repos/{owner}/{repo}/security/automation", configure),
        route(Post, "/repos/{owner}/{repo}/security/automation/scans", scan),
        route(Get, "/repos/{owner}/{repo}/security/automation/inventory", inventory),
        route(Get, "/repos/{owner}/{repo}/security/automation/updates", updates),
        route(Post, "/repos/{owner}/{repo}/security/automation/updates", propose),
    ]
}
#[derive(Deserialize)]
struct Revision {
    commit_sha: String,
}
#[derive(Deserialize)]
struct Proposal {
    commit_sha: String,
    package: String,
}
async fn settings(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(state.services.security.automation_configuration(&repo).await?))
}
async fn configure(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(input): ApiJson<AutomationSettings>,
) -> ApiResult<StatusCode> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    state.services.security.configure_automation(caller.require(TokenScope::Admin)?, &repo, input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn scan(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(input): ApiJson<Revision>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(
        serde_json::json!({"scan_ids":state.services.security.request_automatic_scan(actor(&caller)?, &repo, &input.commit_sha).await?}),
    ))
}
async fn inventory(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    Query(input): Query<Revision>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(state.services.security.dependency_inventory(&repo, &input.commit_sha).await?))
}
async fn updates(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(state.services.security.remediation_history(&repo).await?))
}
async fn propose(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(input): ApiJson<Proposal>,
) -> ApiResult<Response> {
    let repo = caller.open_repo(&state, &owner, &name).await?;
    Ok(response(
        state.services.security.remediate_dependency(caller.require(TokenScope::Admin)?, &repo, &input.commit_sha, &input.package).await?,
    ))
}
