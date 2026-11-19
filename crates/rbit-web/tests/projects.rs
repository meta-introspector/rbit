//! Projects exercise the real router, token scope, session rendering and origin defense.
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use rbit_core::{Config, TokenScope};
use rbit_services::{CreateAccessToken, Services, SignUp, orgs::CreateOrganization};
use rbit_storage::{Database, KvStore};
use std::sync::Arc;
use tower::ServiceExt;

pub(super) async fn issue_token(services: &Services, user: &rbit_core::User, scope: TokenScope) -> String {
    services.tokens.create(user, CreateAccessToken { name: "projects".into(), scope, expires_in_days: None }).await.unwrap().1
}

#[tokio::test]
async fn projects_api_and_browser_enforce_authority_and_escape_titles() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.storage.data_dir = dir.path().join("data");
    config.security.rate_limit = false;
    std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
    let db = Database::open(&config.storage.sqlite_path()).await.unwrap();
    let kv = KvStore::open(&config.storage.kv_path()).unwrap();
    let config = Arc::new(config);
    let services = Services::new(config.clone(), &db, &kv).await.unwrap();
    let owner = services
        .accounts
        .create_user(
            SignUp { username: "owner".into(), email: "owner@example.com".into(), password: "correct horse battery".into() },
            Some(false),
        )
        .await
        .unwrap();
    services
        .orgs
        .create(&owner, CreateOrganization { name: "acme".into(), display_name: String::new(), description: String::new() })
        .await
        .unwrap();
    let token = issue_token(&services, &owner, TokenScope::Admin).await;
    let write = issue_token(&services, &owner, TokenScope::Write).await;
    let read = issue_token(&services, &owner, TokenScope::Read).await;
    let limited = services
        .tokens
        .create_limited(
            &owner,
            CreateAccessToken { name: "limited".into(), scope: TokenScope::Admin, expires_in_days: None },
            &rbit_services::tokens::TokenLimit { repo: None, rights: vec![rbit_services::tokens::Right::ReadLogs] },
        )
        .await
        .unwrap()
        .1;
    let cookie = format!("rbit_session={}", services.accounts.start_session(&owner).await.unwrap().encode());
    let router = rbit_web::router(rbit_web::AppState::new(config, services));
    let body = r#"{"title":"<script>bad</script>","columns":["Todo","Done"]}"#;
    for (credential, expected) in [(&read, StatusCode::FORBIDDEN), (&token, StatusCode::CREATED)] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/orgs/acme/projects")
                    .header(header::AUTHORIZATION, format!("Bearer {credential}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
    }
    planning_api(&router, &token, &write, &read).await;
    let restricted = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/orgs/acme/projects/1")
                .header(header::AUTHORIZATION, format!("Bearer {limited}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(restricted.status(), StatusCode::FORBIDDEN);
    let response = router
        .clone()
        .oneshot(Request::builder().uri("/organizations/acme/projects/1").header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap();
    let html = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(html.contains("&#60;script&#62;bad&#60;/script&#62;"));
    assert!(!html.contains("<script>bad</script>"));
    assert!(html.contains("Add an issue or pull request"));
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/organizations/acme/projects/1/delete")
                .header(header::COOKIE, &cookie)
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "https://attacker.example")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("revision=1"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = router.oneshot(Request::builder().uri("/api/v1/orgs/acme/projects/1").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

async fn project_request(
    router: &axum::Router,
    token: &str,
    method: &str,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(format!("/api/v1/orgs/acme/projects/1{path}"))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}
async fn planning_api(router: &axum::Router, admin: &str, write: &str, read: &str) {
    use serde_json::json;
    let iteration = json!({"title":"Sprint", "starts_on":"2026-11-01", "ends_on":"2026-11-14"});
    assert_eq!(project_request(router, write, "POST", "/iterations", iteration.clone()).await.0, StatusCode::FORBIDDEN);
    let (status, value) = project_request(router, admin, "POST", "/iterations", iteration).await;
    assert_eq!(status, StatusCode::CREATED);
    let id = value["id"].as_i64().unwrap();
    let personal = json!({"title":"Mine", "filter":{"layout":"table","priority":"high"}});
    assert_eq!(project_request(router, read, "POST", "/views", personal.clone()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(project_request(router, write, "POST", "/views", personal).await.0, StatusCode::CREATED);
    let shared = json!({"title":"Team", "shared":true, "filter":{"iteration":id,"sort":"target_date"}});
    assert_eq!(project_request(router, write, "POST", "/views", shared.clone()).await.0, StatusCode::FORBIDDEN);
    let (status, value) = project_request(router, admin, "POST", "/views", shared).await;
    assert_eq!(status, StatusCode::CREATED);
    let view = value["id"].as_i64().unwrap();
    let (_, page) = project_request(router, read, "GET", "", json!(null)).await;
    assert_eq!(page["can_manage"], false);
    assert!(page["views"].as_array().unwrap().iter().all(|v| v["can_edit"] == false));
    let (_, page) = project_request(router, write, "GET", "", json!(null)).await;
    assert_eq!(page["views"][0]["can_edit"], true);
    assert_eq!(page["views"][1]["can_edit"], false);
    let (status, page) = project_request(router, read, "GET", &format!("?view={view}"), json!(null)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(page["filter"]["iteration"], id);
    assert_eq!(project_request(router, write, "DELETE", &format!("/views/{view}?revision=1"), json!(null)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        project_request(router, admin, "PATCH", &format!("/views/{view}"), json!({"revision":1,"title":"Team","shared":false,"filter":{}}))
            .await
            .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(project_request(router, admin, "DELETE", &format!("/views/{view}?revision=2"), json!(null)).await.0, StatusCode::CONFLICT);
    assert_eq!(project_request(router, admin, "DELETE", &format!("/views/{view}?revision=1"), json!(null)).await.0, StatusCode::NO_CONTENT);
    assert_eq!(
        project_request(router, admin, "DELETE", &format!("/iterations/{id}?revision=1"), json!(null)).await.0,
        StatusCode::NO_CONTENT
    );
}
