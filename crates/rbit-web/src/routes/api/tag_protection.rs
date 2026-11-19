//! Administrator-only tag protection management with scoped repository authorization.
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
use rbit_core::TokenScope;
use rbit_services::RepoContext;
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn routes() -> Vec<ApiRoute> {
    vec![
        route(Get, "/repos/{owner}/{repo}/tag_protections", list),
        route(Post, "/repos/{owner}/{repo}/tag_protections", create),
        route(Delete, "/repos/{owner}/{repo}/tag_protections/{id}", delete),
    ]
}
async fn open(state: &AppState, caller: &Caller, owner: &str, repo: &str) -> ApiResult<RepoContext> {
    let ctx = caller.open_repo(state, owner, repo).await?;
    caller.require(TokenScope::Admin)?;
    if !ctx.access.can_admin() {
        return Err(ApiError::forbidden("You must administer this repository"));
    }
    Ok(ctx)
}
async fn list(State(state): State<AppState>, caller: Caller, Path((owner, repo)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let ctx = open(&state, &caller, &owner, &repo).await?;
    let rows = state.services.protection.tag_rules(&ctx).await?;
    Ok(Json(json!(rows.into_iter().map(|(id, pattern)| json!({"id": id, "pattern": pattern.as_str()})).collect::<Vec<_>>())))
}
#[derive(Deserialize)]
struct Input {
    pattern: String,
}
async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo)): Path<(String, String)>,
    ApiJson(input): ApiJson<Input>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let ctx = open(&state, &caller, &owner, &repo).await?;
    let id = state.services.protection.create_tag_rule(&ctx, caller.require_user()?, &input.pattern).await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "pattern": input.pattern.trim()}))))
}
async fn delete(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> ApiResult<StatusCode> {
    let ctx = open(&state, &caller, &owner, &repo).await?;
    state.services.protection.delete_tag_rule(&ctx, caller.require_user()?, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::api::test_support::Fixture;
    #[tokio::test]
    async fn rules_require_admin_scope_and_repository_access() {
        let fixture = Fixture::new(|_| {}).await;
        let alice = fixture.user("alice").await;
        let bob = fixture.user("bob").await;
        fixture.repo(&alice, "private", rbit_core::Visibility::Private).await;
        let admin = fixture.token(&alice, TokenScope::Admin).await;
        let write = fixture.token(&alice, TokenScope::Write).await;
        let outsider = fixture.token(&bob, TokenScope::Admin).await;
        let path = "/api/v1/repos/alice/private/tag_protections";
        assert_eq!(fixture.send("GET", path, Some(&outsider), None).await.0.status(), StatusCode::NOT_FOUND);
        assert_eq!(fixture.send("POST", path, Some(&write), Some(json!({"pattern":"v*"}))).await.0.status(), StatusCode::FORBIDDEN);
        let (response, rule) = fixture.send("POST", path, Some(&admin), Some(json!({"pattern":"v*"}))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let (response, rules) = fixture.send("GET", path, Some(&admin), None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(rules.as_array().unwrap().len(), 1);
        assert_eq!(
            fixture.send("POST", path, Some(&admin), Some(json!({"pattern":"a b"}))).await.0.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let path = format!("{path}/{}", rule["id"]);
        assert_eq!(fixture.send("DELETE", &path, Some(&admin), None).await.0.status(), StatusCode::NO_CONTENT);
        assert_eq!(fixture.send("DELETE", &path, Some(&admin), None).await.0.status(), StatusCode::NOT_FOUND);
    }
}
