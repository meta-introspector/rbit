//! Public OIDC discovery/JWKS and task-token-only identity issuance, outside page auth.
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use rbit_services::ThrottleScope;
use serde::Deserialize;
use serde_json::json;

use super::api::ApiError;
use crate::{AppState, client_ip::ClientIp};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/actions/oidc/.well-known/openid-configuration", get(discovery))
        .route("/api/actions/oidc/jwks", get(jwks))
        .route("/api/actions/oidc/token", get(token))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(StatusCode::SERVICE_UNAVAILABLE, std::time::Duration::from_secs(30)))
        .layer(axum::middleware::from_fn(no_store))
}

async fn no_store(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let token = request.uri().path().ends_with("/token");
    let mut response = next.run(request).await;
    let cache = if token || !response.status().is_success() { "no-store" } else { "public, max-age=300" };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn actions(state: &AppState) -> Result<&rbit_services::ActionsService, ApiError> {
    state.services.actions.as_ref().ok_or_else(ApiError::not_found)
}

async fn discovery(State(state): State<AppState>) -> Result<Response, ApiError> {
    Ok(Json(actions(&state)?.oidc_discovery()?).into_response())
}

async fn jwks(State(state): State<AppState>, ClientIp(ip): ClientIp) -> Result<Response, ApiError> {
    state.services.throttle.check_client(ThrottleScope::ApiRequest, &ip.to_canonical().to_string())?;
    Ok(Json(actions(&state)?.oidc_jwks().await?).into_response())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TokenRequest {
    audience: Option<String>,
    #[serde(rename = "api-version")]
    version: Option<String>,
}

async fn token(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    headers: HeaderMap,
    query: Result<Query<TokenRequest>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, ApiError> {
    let address = ip.to_canonical().to_string();
    state.services.throttle.check_client(ThrottleScope::ApiAuth, &address)?;
    let Query(request) = query.map_err(ApiError::from)?;
    if request.version.as_deref().is_some_and(|version| version != "1") {
        return Err(ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "Unsupported OIDC request version"));
    }
    let default_audience = rbit_services::actions::oidc::issuer(state.config.server.base_url())?;
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|header| header.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "A running workflow task token is required"))?;
    let result = actions(&state)?.oidc_token(bearer, request.audience.as_deref().unwrap_or(&default_audience)).await;
    match result {
        Ok(value) => Ok(Json(json!({"value": value})).into_response()),
        Err(error) => {
            state.services.throttle.record_client_failure(ThrottleScope::ApiAuth, &address);
            Err(error.into())
        }
    }
}
