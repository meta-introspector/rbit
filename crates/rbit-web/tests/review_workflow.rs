//! Real router coverage for review-progress privacy, suggestion scopes and browser origins.
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use rbit_core::{Config, TokenScope, Visibility};
use rbit_services::{CreateAccessToken, CreateRepository, Services, pulls::NewPullInput};
use rbit_storage::{Database, KvStore, NewUser};
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use tower::ServiceExt;

mod conflicts;

struct Fixture {
    _dir: tempfile::TempDir,
    router: Router,
    token: String,
    read_token: String,
    cookie: String,
    head: String,
}
fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(["-c", "user.name=Reviewer", "-c", "user.email=reviewer@example.test", "-c", "init.defaultBranch=main"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}
impl Fixture {
    async fn new() -> Self {
        Self::setup(false).await
    }
    async fn setup(conflicted: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = dir.path().join("data");
        config.security.rate_limit = false;
        std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
        let db = Database::open(&config.storage.sqlite_path()).await.unwrap();
        let kv = KvStore::open(&config.storage.kv_path()).unwrap();
        let config = Arc::new(config);
        let services = Services::new(config.clone(), &db, &kv).await.unwrap();
        let user = db
            .accounts()
            .create_user(NewUser {
                name: "reviewer".parse().unwrap(),
                email: "reviewer@example.test".parse().unwrap(),
                display_name: None,
                password_hash: "test".into(),
                is_admin: false,
            })
            .await
            .unwrap();
        services
            .repositories
            .create(&user, CreateRepository { name: "demo".into(), description: String::new(), visibility: Visibility::Public })
            .await
            .unwrap();
        let repo = services.repositories.open(Some(&user), "reviewer", "demo").await.unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir(&work).unwrap();
        git(&work, &["init", "-q"]);
        std::fs::write(work.join("README"), "old\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-qm", "base"]);
        git(&work, &["push", repo.disk_path.to_str().unwrap(), "main"]);
        git(&work, &["checkout", "-qb", "feature"]);
        std::fs::write(work.join("README"), "new\n").unwrap();
        git(&work, &["commit", "-qam", "head"]);
        let head = git(&work, &["rev-parse", "HEAD"]);
        git(&work, &["push", repo.disk_path.to_str().unwrap(), "feature"]);
        if conflicted {
            git(&work, &["checkout", "-q", "main"]);
            std::fs::write(work.join("README"), "target\n").unwrap();
            git(&work, &["commit", "-qam", "target"]);
            git(&work, &["push", repo.disk_path.to_str().unwrap(), "main"]);
        }
        services
            .pulls
            .create(
                &user,
                &repo,
                &repo,
                NewPullInput { head_branch: "feature".into(), base_branch: "main".into(), ..NewPullInput::default() },
            )
            .await
            .unwrap();
        let make = |scope| CreateAccessToken { name: "workflow".into(), scope, expires_in_days: None };
        let token = services.tokens.create(&user, make(TokenScope::Write)).await.unwrap().1;
        let read_token =
            services.tokens.create(&user, CreateAccessToken { name: "read".into(), ..make(TokenScope::Read) }).await.unwrap().1;
        let cookie = format!("rbit_session={}", services.accounts.start_session(&user).await.unwrap().encode());
        let router = rbit_web::router(rbit_web::AppState::new(config, services));
        Self { _dir: dir, router, token, read_token, cookie, head }
    }
    async fn api(&self, method: &str, suffix: &str, token: &str, input: Option<Value>) -> (StatusCode, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/v1/repos/reviewer/demo/pulls/1{suffix}"))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(input.map_or_else(String::new, |v| v.to_string())))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }
}

#[tokio::test]
async fn review_workflow_http_forms_scopes_and_exact_head() {
    let f = Fixture::new().await;
    let (status, progress) = f.api("GET", "/review-progress", &f.token, None).await;
    assert_eq!(status, StatusCode::OK, "{progress}");
    let file = &progress["files"][0];
    let mark = json!({"path":file["path"],"fingerprint":file["fingerprint"],"expected_head":f.head,"viewed":true});
    assert_eq!(f.api("PUT", "/review-progress", &f.read_token, Some(mark.clone())).await.0, StatusCode::FORBIDDEN);
    assert_eq!(f.api("PUT", "/review-progress", &f.token, Some(mark)).await.0, StatusCode::NO_CONTENT);
    let first = f.api("GET", "/review-changes", &f.token, None).await;
    assert_eq!(first.1["first_review"], true);
    let body = "```suggestion\nbetter\n```";
    let (status, comment) = f
        .api("POST", "/comments", &f.token, Some(json!({"path":"README","side":"RIGHT","line":1,"body":body,"expected_head":f.head})))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{comment}");
    let input = json!({"expected_head":f.head,"suggestions":[{"comment_id":comment["id"],"body_digest":comment["body_digest"]}]});
    assert_eq!(f.api("POST", "/suggestions/apply", &f.read_token, Some(input.clone())).await.0, StatusCode::FORBIDDEN);
    let page = Request::builder().uri("/reviewer/demo/pull/1/files").header("cookie", &f.cookie).body(Body::empty()).unwrap();
    let response = f.router.clone().oneshot(page).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap().to_vec()).unwrap();
    assert!(html.contains("Apply selected suggestions") && html.contains("Mark as not viewed") && html.contains("suggestion_"));
    assert_eq!(f.api("POST", "/reviews", &f.token, Some(json!({"event":"COMMENT","body":"Reviewed"}))).await.0, StatusCode::OK);
    let since = Request::builder().uri("/reviewer/demo/pull/1/files?since=review").header("cookie", &f.cookie).body(Body::empty()).unwrap();
    let response = f.router.clone().oneshot(since).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap().to_vec()).unwrap();
    assert!(!html.contains("/review-progress"), "since-review diffs do not attest full-diff viewed state");
    assert!(html.contains("No changes since your last submitted review"));
    let hostile = Request::builder()
        .method("POST")
        .uri("/reviewer/demo/pull/1/suggestions")
        .header("cookie", &f.cookie)
        .header("origin", "https://untrusted.example")
        .header("host", "localhost")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from("expected_head=bad"))
        .unwrap();
    assert_eq!(f.router.clone().oneshot(hostile).await.unwrap().status(), StatusCode::FORBIDDEN);
    let (status, result) = f.api("POST", "/suggestions/apply", &f.token, Some(input.clone())).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_ne!(result["commit_sha"], f.head);
    let page = Request::builder().uri("/reviewer/demo/pull/1/files").header("cookie", &f.cookie).body(Body::empty()).unwrap();
    let response = f.router.clone().oneshot(page).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let html = String::from_utf8(axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap().to_vec()).unwrap();
    assert!(html.contains(result["commit_sha"].as_str().unwrap()));
    assert!(!html.contains("Apply selected suggestions"));
    assert!(!html.contains("Mark as not viewed"), "the applied file needs review again");
    assert_eq!(f.api("POST", "/suggestions/apply", &f.token, Some(input)).await.0, StatusCode::CONFLICT);
}
