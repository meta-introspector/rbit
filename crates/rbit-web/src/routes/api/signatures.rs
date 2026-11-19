//! Signing trust CRUD requires repository administration; verification requires read access.
use super::{
    ApiMethod::{Delete, Get, Post},
    ApiRoute,
    auth::Caller,
    error::ApiResult,
    json::ApiJson,
    route,
};
use crate::{AppState, views::signatures::KeyInput};
use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use rbit_core::TokenScope;

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/repos/{owner}/{repo}/git/signatures/{id}", verify),
        route(Get, "/repos/{owner}/{repo}/signing_keys", list),
        route(Post, "/repos/{owner}/{repo}/signing_keys", create),
        route(Delete, "/repos/{owner}/{repo}/signing_keys/{id}", delete),
    ]
}
async fn verify(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let ctx = caller.open_repo(&state, &owner, &repo).await?;
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(state.services.signatures.verify(&ctx, id).await?)).into_response())
}
async fn list(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let ctx = caller.open_repo(&state, &owner, &repo).await?;
    caller.require(TokenScope::Admin)?;
    let keys = state.services.signatures.list_keys(&ctx).await?;
    Ok(Json(serde_json::json!(
        keys.iter()
            .map(|key| serde_json::json!({"id":key.id,"name":key.name,"kind":key.kind,"public_key":key.public_key}))
            .collect::<Vec<_>>()
    )))
}
async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo)): Path<(String, String)>,
    ApiJson(input): ApiJson<KeyInput>,
) -> ApiResult<StatusCode> {
    let ctx = caller.open_repo(&state, &owner, &repo).await?;
    caller.require(TokenScope::Admin)?;
    state.services.signatures.add_key(caller.require_user()?, &ctx, input.name, input.kind, input.public_key).await?;
    Ok(StatusCode::CREATED)
}
async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let ctx = caller.open_repo(&state, &owner, &repo).await?;
    caller.require(TokenScope::Admin)?;
    state.services.signatures.delete_key(caller.require_user()?, &ctx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::super::test_support::Fixture;
    use rbit_core::{TokenScope, Visibility};
    #[tokio::test]
    async fn signatures_routes_enforce_repository_and_token_authority() {
        let f = Fixture::new(|_| {}).await;
        let owner = f.user("signature-owner").await;
        let outsider = f.user("signature-other").await;
        f.repo(&owner, "private", Visibility::Private).await;
        f.repo(&owner, "public", Visibility::Public).await;
        let admin = f.token(&owner, TokenScope::Admin).await;
        let read = f.token(&owner, TokenScope::Read).await;
        let other = f.token(&outsider, TokenScope::Admin).await;
        let path = "/api/v1/repos/signature-owner/private/signing_keys";
        assert_eq!(f.send("GET", path, Some(&other), None).await.0.status(), 404);
        assert_eq!(f.send("GET", path, Some(&read), None).await.0.status(), 403);
        assert_eq!(f.send("GET", path, Some(&admin), None).await.1, serde_json::json!([]));
        let input = serde_json::json!({"name":"invalid", "kind":"ssh", "public_key":"not a key"});
        assert_eq!(f.send("POST", path, Some(&admin), Some(input.clone())).await.0.status(), 422);
        assert_eq!(f.send("POST", path, Some(&read), Some(input)).await.0.status(), 403);
        assert_eq!(f.send("GET", "/api/v1/repos/signature-owner/private/git/signatures/abc", Some(&other), None).await.0.status(), 404);
        assert_eq!(f.send("GET", "/api/v1/repos/signature-owner/public/git/signatures/abc", None, None).await.0.status(), 404);
        assert_eq!(f.raw("GET", "/signature-owner/public/settings/signing-keys", None, None, None).await.0.status(), 404);
    }
}
