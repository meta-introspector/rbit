//! Human-owner management of delegated identities and one-time credentials.

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rbit_core::{AccountId, TokenScope, User};
use rbit_identity_models::agents::{Agent, AgentInput};
use rbit_services::webhooks::payloads::timestamp;
use rbit_services::{CreateAccessToken, ServiceError, agents::AgentCredential};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    ApiMethod::{Delete, Get, Patch, Post, Put},
    ApiRoute,
    auth::Caller,
    error::{ApiError, ApiResult},
    json::ApiJson,
    route,
};
use crate::AppState;

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/user/agents", list),
        route(Post, "/user/agents", create),
        route(Get, "/user/agents/{id}", show),
        route(Patch, "/user/agents/{id}", update),
        route(Post, "/user/agents/{id}/disable", disable),
        route(Get, "/user/agents/{id}/credentials", credentials),
        route(Post, "/user/agents/{id}/credentials", create_credential),
        route(Delete, "/user/agents/{id}/credentials/{token}", revoke_credential),
        route(Post, "/user/agents/{id}/credentials/{token}/rotate", rotate_credential),
        route(Get, "/orgs/{org}/agent-policy", organization_policy),
        route(Put, "/orgs/{org}/agent-policy", set_organization_policy),
    ]
}

fn owner(caller: &Caller) -> ApiResult<&User> {
    let user = caller.require(TokenScope::Admin)?;
    caller.require_unlimited()?;
    if user.is_agent {
        return Err(ApiError::forbidden("Agents cannot manage identities or credentials"));
    }
    Ok(user)
}

fn id(value: &str) -> ApiResult<AccountId> {
    identifier(value).map(AccountId)
}

fn identifier(value: &str) -> ApiResult<i64> {
    value.parse::<i64>().ok().filter(|id| *id > 0).ok_or_else(ApiError::not_found)
}

fn revision(value: i64) -> ApiResult<i64> {
    if value > 0 { Ok(value) } else { Err(ApiError::invalid("revision", "revision must be positive")) }
}

fn error(error: ServiceError) -> ApiError {
    match error {
        ServiceError::Invalid(message) if message == rbit_services::agents::REVISION_CONFLICT => {
            ApiError::new(StatusCode::CONFLICT, message).with_code("agent_changed")
        }
        other => ApiError::service(other, "agent"),
    }
}

fn account(agent: &Agent) -> Value {
    json!({"id": agent.account_id.0, "owner_id": agent.owner_id.0, "login": agent.handle, "type": "Bot",
        "slug": agent.slug, "name": agent.name, "enabled": agent.enabled, "revision": agent.revision,
        "capabilities": agent.capabilities, "created_at": timestamp(agent.created_at)})
}

async fn details(state: &AppState, actor: &User, agent: AccountId) -> ApiResult<Value> {
    let record = state.services.agents.find(actor, agent).await.map_err(error)?;
    let scopes = state.services.agents.scopes(actor, agent).await.map_err(error)?;
    let mut value = account(&record);
    value["scopes"] = json!(scopes);
    Ok(value)
}

async fn list(State(state): State<AppState>, caller: Caller) -> ApiResult<Json<Value>> {
    let actor = owner(&caller)?;
    let records = state.services.agents.list(actor).await.map_err(error)?;
    let mut agents = Vec::with_capacity(records.len());
    for record in records {
        let scopes = state.services.agents.scopes(actor, record.account_id).await.map_err(error)?;
        let mut value = account(&record);
        value["scopes"] = json!(scopes);
        agents.push(value);
    }
    Ok(Json(json!(agents)))
}

async fn create(
    State(state): State<AppState>,
    caller: Caller,
    ApiJson(input): ApiJson<AgentInput>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let actor = owner(&caller)?;
    let agent = state.services.agents.create(actor, &input).await.map_err(error)?;
    Ok((StatusCode::CREATED, Json(details(&state, actor, agent.account_id).await?)))
}

async fn show(State(state): State<AppState>, caller: Caller, Path(agent): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(details(&state, owner(&caller)?, id(&agent)?).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    revision: i64,
    enabled: bool,
    agent: AgentInput,
}

async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Path(agent): Path<String>,
    ApiJson(input): ApiJson<Update>,
) -> ApiResult<Json<Value>> {
    let actor = owner(&caller)?;
    let agent = id(&agent)?;
    state.services.agents.update(actor, agent, revision(input.revision)?, &input.agent, input.enabled).await.map_err(error)?;
    Ok(Json(details(&state, actor, agent).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    revision: i64,
}

async fn disable(
    State(state): State<AppState>,
    caller: Caller,
    Path(agent): Path<String>,
    ApiJson(input): ApiJson<Revision>,
) -> ApiResult<StatusCode> {
    state.services.agents.disable(owner(&caller)?, id(&agent)?, revision(input.revision)?).await.map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn credential(credential: &AgentCredential) -> Value {
    let token = &credential.token;
    json!({"id": credential.id, "name": token.name, "hint": token.hint, "scope": token.scope.as_str(),
        "created_at": timestamp(token.created_at), "last_used_at": token.last_used_at.map(timestamp), "expires_at": token.expires_at.map(timestamp)})
}

async fn credentials(State(state): State<AppState>, caller: Caller, Path(agent): Path<String>) -> ApiResult<Json<Value>> {
    let tokens = state.services.agents.managed_credentials(owner(&caller)?, id(&agent)?).await.map_err(error)?;
    Ok(Json(json!(tokens.iter().map(credential).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CredentialInput {
    revision: i64,
    name: String,
    scope: String,
    #[serde(default = "default_expiry")]
    expires_in_days: Option<u32>,
}
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must preserve omitted thirty-day expiry versus explicit null for no expiration"
)]
const fn default_expiry() -> Option<u32> {
    Some(30)
}

async fn issuer_lifetime(state: &AppState, caller: &Caller, actor: &User, days: Option<u32>) -> ApiResult<Option<rbit_core::Timestamp>> {
    let issuer = caller.token_id().ok_or_else(|| ApiError::unauthorized("An owner token is required"))?;
    let current = state
        .services
        .tokens
        .list(actor)
        .await?
        .into_iter()
        .find(|token| token.id == issuer)
        .ok_or_else(|| ApiError::unauthorized("The issuing credential was revoked"))?;
    if let Some(expiry) = current.expires_at {
        let requested = days.map(|days| rbit_core::Timestamp::now().0 + i64::from(days) * 86_400);
        if requested.is_none_or(|requested| requested > expiry.0) {
            return Err(ApiError::invalid("expires_in_days", "an agent credential may not outlive the token that creates it"));
        }
    }
    Ok(current.expires_at)
}

async fn create_credential(
    State(state): State<AppState>,
    caller: Caller,
    Path(agent): Path<String>,
    ApiJson(input): ApiJson<CredentialInput>,
) -> ApiResult<Response> {
    let actor = owner(&caller)?;
    let scope = TokenScope::parse(&input.scope)
        .filter(|scope| *scope != TokenScope::Admin)
        .ok_or_else(|| ApiError::invalid("scope", "agent credential scope must be read or write"))?;
    let ceiling = issuer_lifetime(&state, &caller, actor, input.expires_in_days).await?;
    let (metadata, secret) = state
        .services
        .agents
        .create_managed_credential_bounded(
            actor,
            id(&agent)?,
            revision(input.revision)?,
            CreateAccessToken { name: input.name, scope, expires_in_days: input.expires_in_days },
            ceiling,
        )
        .await
        .map_err(error)?;
    let mut value = credential(&metadata);
    value["token"] = json!(secret);
    Ok(([(header::CACHE_CONTROL, "no-store")], (StatusCode::CREATED, Json(value))).into_response())
}

async fn rotate_credential(
    State(state): State<AppState>,
    caller: Caller,
    Path((agent, old)): Path<(String, String)>,
    ApiJson(input): ApiJson<CredentialInput>,
) -> ApiResult<Response> {
    let actor = owner(&caller)?;
    let scope = TokenScope::parse(&input.scope)
        .filter(|scope| *scope != TokenScope::Admin)
        .ok_or_else(|| ApiError::invalid("scope", "agent credential scope must be read or write"))?;
    let ceiling = issuer_lifetime(&state, &caller, actor, input.expires_in_days).await?;
    let (metadata, secret) = state
        .services
        .agents
        .rotate_credential_bounded(
            actor,
            id(&agent)?,
            revision(input.revision)?,
            identifier(&old)?,
            CreateAccessToken { name: input.name, scope, expires_in_days: input.expires_in_days },
            ceiling,
        )
        .await
        .map_err(error)?;
    let mut value = credential(&metadata);
    value["token"] = json!(secret);
    Ok(([(header::CACHE_CONTROL, "no-store")], (StatusCode::CREATED, Json(value))).into_response())
}

async fn revoke_credential(
    State(state): State<AppState>,
    caller: Caller,
    Path((agent, token)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    state.services.agents.revoke_managed_credential(owner(&caller)?, id(&agent)?, identifier(&token)?).await.map_err(error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Policy {
    allow_agents: bool,
}

async fn organization_policy(State(state): State<AppState>, caller: Caller, Path(org): Path<String>) -> ApiResult<Json<Value>> {
    let actor = owner(&caller)?;
    let org = state.services.orgs.open(Some(actor), &org).await?;
    let allow = state.services.agents.organization_policy(actor, org.org.account.id).await.map_err(error)?;
    Ok(Json(json!({"allow_agents": allow})))
}

async fn set_organization_policy(
    State(state): State<AppState>,
    caller: Caller,
    Path(org): Path<String>,
    ApiJson(input): ApiJson<Policy>,
) -> ApiResult<Json<Value>> {
    let actor = owner(&caller)?;
    let org = state.services.orgs.open(Some(actor), &org).await?;
    state.services.agents.set_organization_policy(actor, org.org.account.id, input.allow_agents).await.map_err(error)?;
    Ok(Json(json!({"allow_agents": input.allow_agents})))
}
