//! Repository deployment evidence and scoped current-workflow promotion/rollback APIs.
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
    extract::{Path, Query, State},
    http::StatusCode,
};
use rbit_core::TokenScope;
use rbit_services::{ActionsService, ServiceError, actions::deployments::DeploymentRequest, tokens::Right};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/repos/{owner}/{repo}/deployments", list),
        route(Post, "/repos/{owner}/{repo}/deployments/requests", request),
        route(Put, "/repos/{owner}/{repo}/deployments/workflows/{environment}", configure),
    ]
}
fn actions(state: &AppState) -> ApiResult<&ActionsService> {
    state.services.actions.as_ref().ok_or_else(ApiError::not_found)
}
fn error(error: ServiceError) -> ApiError {
    match error {
        ServiceError::Conflict(message) => ApiError::new(StatusCode::CONFLICT, message),
        other => ApiError::service(other, "deployment"),
    }
}
#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    before: Option<i64>,
}

async fn list(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
) -> ApiResult<Json<Value>> {
    let repo = caller.open_repo(&state, &owner, &repo).await?;
    let data = actions(&state)?.deployment_dashboard(&repo, query.before).await?;
    let attempts: Vec<_> = data.attempts.iter().map(|attempt| json!({
        "id":attempt.id,"environment":attempt.environment,"sha":attempt.sha,"workflow_sha":attempt.workflow_sha,
        "origin_attempt_id":attempt.origin_attempt_id,"origin_artifact_id":attempt.artifact_id,"origin_artifact_digest":attempt.artifact_digest,"job_id":attempt.job_id.0,
        "job_name":attempt.job_name,"job_attempt":attempt.job_attempt,"run_number":attempt.run_number,"run_attempt":attempt.run_attempt,
        "run_available":attempt.run_available,"workflow_path":attempt.workflow_path,"status":attempt.status,"trusted":attempt.trusted,
        "is_fork":attempt.is_fork,"event":attempt.event,"observed_environment_url":attempt.environment_url,"finished_at":attempt.finished_at,
    })).collect();
    let active: Vec<_> = data.active.iter().map(|attempt| json!({
        "environment":attempt.environment,"sha":attempt.sha,"workflow_sha":attempt.workflow_sha,
        "origin_attempt_id":attempt.origin_attempt_id,"origin_artifact_id":attempt.artifact_id,"origin_artifact_digest":attempt.artifact_digest,"job_id":attempt.job_id,"job_name":attempt.job_name,
        "job_attempt":attempt.job_attempt,"run_number":attempt.run_number,"status":rbit_core::actions::Status::from_code(attempt.status),
        "observed_environment_url":attempt.environment_url,
    })).collect();
    let configurations: Vec<_> = data
        .configurations
        .iter()
        .map(|config| {
            json!({"id":config.id,"generation":config.generation,
        "environment":config.environment,"source_environment":config.source_environment,"workflow_path":config.workflow_path})
        })
        .collect();
    let requests: Vec<_> = data
        .intents
        .iter()
        .map(|intent| {
            json!({"id":intent.id,"actor_id":intent.actor_id,"operation":intent.operation,
        "environment":intent.environment,"source_attempt_id":intent.source_attempt_id,"source_sha":intent.source_sha,
        "origin_attempt_id":intent.origin_attempt_id,"origin_artifact_id":intent.origin_artifact_id,"origin_artifact_digest":intent.origin_artifact_digest,
        "artifact_id":intent.artifact_id,"artifact_digest":intent.artifact_digest,"workflow_sha":intent.workflow_sha,
        "workflow_path":intent.workflow_path,"workflow_ref":intent.workflow_ref,"run_id":intent.run_id,"run_number":intent.run_number,
        "workflow_status":intent.workflow_status.and_then(rbit_core::actions::Status::from_code),"created_at":intent.created_at})
        })
        .collect();
    let next = data.attempts.last().filter(|_| attempts.len() == 50).map(|last| last.id);
    Ok(Json(json!({"attempts":attempts,"active":active,"configurations":configurations,"requests":requests,"next_before":next})))
}

async fn request(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo)): Path<(String, String)>,
    ApiJson(input): ApiJson<DeploymentRequest>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    caller.require_right(Right::Dispatch)?;
    let actor = caller.require(TokenScope::Write)?;
    let repo = caller.open_repo(&state, &owner, &repo).await?;
    let run = actions(&state)?.deploy(actor, &repo, input).await.map_err(error)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"run_id":run.id.0,"run_number":run.number,"workflow_sha":run.sha,"workflow_path":run.workflow_path})),
    ))
}
#[derive(Debug, Deserialize)]
pub struct ConfigurationBody {
    source_environment: String,
    workflow: String,
}
async fn configure(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, environment)): Path<(String, String, String)>,
    ApiJson(input): ApiJson<ConfigurationBody>,
) -> ApiResult<Json<Value>> {
    let actor = caller.require(TokenScope::Admin)?;
    let repo = caller.open_repo(&state, &owner, &repo).await?;
    let configured = actions(&state)?
        .configure_deployment_workflow(actor, &repo, &environment, &input.source_environment, &input.workflow)
        .await
        .map_err(error)?;
    Ok(Json(json!({"id":configured.id,"environment":configured.environment,"source_environment":configured.source_environment,
        "workflow_path":configured.workflow_path,"generation":configured.generation})))
}
