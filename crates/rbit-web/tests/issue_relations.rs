//! Real HTTP relationship resources, scope limits, safe rendering and origin defenses.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use rbit_core::{Config, IssueKind, IssueText, IssueTitle, Timestamp, TokenScope, Visibility};
use rbit_services::{CreateAccessToken, CreateOrganization, CreateRepository, Services, SignUp};
use rbit_storage::{Database, KvStore, sqlite::issues::NewIssue};
use std::sync::Arc;
use tower::ServiceExt;

struct Fixture {
    _dir: tempfile::TempDir,
    app: Router,
    token: String,
    read: String,
    limited: String,
    cookie: String,
}
async fn fixture() -> Fixture {
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
    for (name, visibility) in [("public", Visibility::Public), ("private", Visibility::Private)] {
        let repo = services
            .repositories
            .create_in(&owner, "acme", CreateRepository { name: name.into(), description: String::new(), visibility })
            .await
            .unwrap();
        for title in ["<script>secret</script>", "Second issue"] {
            db.issues()
                .create(&NewIssue {
                    repo: repo.repo.id,
                    kind: IssueKind::Issue,
                    title: IssueTitle::parse(title).unwrap(),
                    body: IssueText::body("").unwrap(),
                    author: owner.id(),
                    at: Timestamp(1),
                })
                .await
                .unwrap();
        }
    }
    let token_input = |scope| CreateAccessToken { name: "relations".into(), scope, expires_in_days: None };
    let token = services.tokens.create(&owner, token_input(TokenScope::Write)).await.unwrap().1;
    let read = services.tokens.create(&owner, token_input(TokenScope::Read)).await.unwrap().1;
    let limited = services
        .tokens
        .create_limited(
            &owner,
            token_input(TokenScope::Write),
            &rbit_services::tokens::TokenLimit { repo: None, rights: vec![rbit_services::tokens::Right::ReadLogs] },
        )
        .await
        .unwrap()
        .1;
    let cookie = format!("rbit_session={}", services.accounts.start_session(&owner).await.unwrap().encode());
    Fixture { _dir: dir, app: rbit_web::router(rbit_web::AppState::new(config, services)), token, read, limited, cookie }
}
async fn request(f: &Fixture, method: &str, uri: &str, token: Option<&str>, body: &str) -> (StatusCode, String) {
    let mut req = Request::builder().method(method).uri(uri).header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = f.app.clone().oneshot(req.body(Body::from(body.to_owned())).unwrap()).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn relations_api_scopes_privacy_and_exact_edge_removal() {
    let f = fixture().await;
    let uri = "/api/v1/repos/acme/public/issues/1/relations";
    let body = r#"{"kind":"blocked_by","repository":"private","number":1}"#;
    assert_eq!(request(&f, "POST", uri, Some(&f.read), body).await.0, StatusCode::FORBIDDEN);
    assert_eq!(request(&f, "POST", uri, Some(&f.limited), body).await.0, StatusCode::FORBIDDEN);
    let created = request(&f, "POST", uri, Some(&f.token), body).await;
    assert_eq!(created.0, StatusCode::CREATED, "{}", created.1);
    let edge = serde_json::from_str::<serde_json::Value>(&created.1).unwrap()["id"].as_i64().unwrap();
    let visible = request(&f, "GET", uri, None, "").await;
    assert_eq!(visible.0, StatusCode::OK);
    let visible: serde_json::Value = serde_json::from_str(&visible.1).unwrap();
    assert_eq!(visible["items"], serde_json::json!([]));
    assert_eq!(visible["progress"]["blocked"], false);
    let owner = request(&f, "GET", uri, Some(&f.read), "").await;
    let owner: serde_json::Value = serde_json::from_str(&owner.1).unwrap();
    assert_eq!(owner["progress"]["blocked"], true);
    assert_eq!(owner["can_edit"], false, "read token must not advertise write controls");
    assert_eq!(owner["items"][0]["can_remove"], false);
    assert_eq!(request(&f, "GET", uri, Some(&f.limited), "").await.0, StatusCode::FORBIDDEN);
    let backwards = "/api/v1/repos/acme/private/issues/1/relations";
    assert_eq!(
        request(&f, "POST", backwards, Some(&f.token), r#"{"kind":"child","repository":"public","number":1}"#).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(request(&f, "DELETE", &format!("{uri}/{edge}"), Some(&f.token), "").await.0, StatusCode::NO_CONTENT);
    assert_eq!(request(&f, "DELETE", &format!("{uri}/{edge}"), Some(&f.token), "").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn relations_browser_escapes_titles_and_rejects_cross_origin_forms() {
    let f = fixture().await;
    let api = "/api/v1/repos/acme/public/issues/2/relations";
    assert_eq!(
        request(&f, "POST", api, Some(&f.token), r#"{"kind":"child","repository":"private","number":1}"#).await.0,
        StatusCode::CREATED
    );
    let page = "/acme/public/issues/2";
    let response =
        f.app.clone().oneshot(Request::builder().uri(page).header(header::COOKIE, &f.cookie).body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(axum::body::to_bytes(response.into_body(), 1_000_000).await.unwrap().to_vec()).unwrap();
    assert!(html.contains("Relationships") && html.contains("Subissues completed"));
    assert!(html.contains("&#60;script&#62;secret&#60;/script&#62;"));
    assert!(!html.contains("<script>secret</script>"));
    let anonymous = request(&f, "GET", page, None, "").await;
    assert!(!anonymous.1.contains("secret") && !anonymous.1.contains("/acme/private"));
    let response = f
        .app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{page}/relations"))
                .header(header::COOKIE, &f.cookie)
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "https://attacker.example")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("kind=child&number=1"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
