//! Real session planning forms exercise empty options, escaping, revisions and Origin.
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use rbit_core::{Config, IssueKind, IssueText, IssueTitle, Timestamp, Visibility};
use rbit_services::{CreateOrganization, CreateRepository, Services, SignUp};
use rbit_storage::{Database, KvStore, sqlite::issues::NewIssue};
use std::sync::Arc;
use tower::ServiceExt;

async fn request(app: &axum::Router, cookie: &str, path: &str, body: Option<&str>) -> (StatusCode, String) {
    let mut request = Request::builder().uri(path).header(header::COOKIE, cookie).header(header::HOST, "localhost");
    if body.is_some() {
        request = request
            .method("POST")
            .header(header::ORIGIN, "http://localhost")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let response = app.clone().oneshot(request.body(body.map_or_else(Body::empty, |b| Body::from(b.to_owned()))).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 2_000_000).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}
struct Fixture {
    _dir: tempfile::TempDir,
    db: Database,
    app: axum::Router,
    org: i64,
    user: i64,
    board: i64,
    issue: i64,
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
    let user = services
        .accounts
        .create_user(
            SignUp { username: "planner".into(), email: "planner@example.test".into(), password: "correct horse battery".into() },
            Some(false),
        )
        .await
        .unwrap();
    let org = services
        .orgs
        .create(&user, CreateOrganization { name: "planning".into(), display_name: String::new(), description: String::new() })
        .await
        .unwrap();
    let repo = services
        .repositories
        .create_in(
            &user,
            "planning",
            CreateRepository { name: "public".into(), description: String::new(), visibility: Visibility::Public },
        )
        .await
        .unwrap();
    let issue = db
        .issues()
        .create(&NewIssue {
            repo: repo.repo.id,
            kind: IssueKind::Issue,
            title: IssueTitle::parse("<script>visible</script>").unwrap(),
            body: IssueText::body("").unwrap(),
            author: user.id(),
            at: Timestamp(1),
        })
        .await
        .unwrap();
    let board = services
        .projects
        .create(
            &user,
            org.account.id.0,
            &rbit_services::projects::BoardInput { title: "Planning".into(), columns: vec!["Todo".into(), "Done".into()] },
        )
        .await
        .unwrap();
    services.projects.add(&user, org.account.id.0, board.id, issue.id.0).await.unwrap();
    let cookie = format!("rbit_session={}", services.accounts.start_session(&user).await.unwrap().encode());
    let app = rbit_web::router(rbit_web::AppState::new(config, services));
    Fixture { _dir: dir, db, app, org: org.account.id.0, user: user.id().0, board: board.id, issue: issue.id.0, cookie }
}
#[tokio::test]
async fn project_planning_browser_roundtrip_and_stale_write() {
    let Fixture { _dir, db, app, org, user, board, issue, cookie } = fixture().await;
    let base = format!("/organizations/planning/projects/{board}");
    let card = format!("{base}/cards/{issue}/planning");
    let (status, html) = request(&app, &cookie, &card, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("<script>visible</script>"));
    assert!(html.contains("name=\"target_date\""));
    let revision = db.projects().page(org, user, board, &rbit_services::projects::Filter::default()).await.unwrap().cards[0].revision;
    let body = format!("revision={revision}&priority=high&iteration_id=&target_date=2020-01-01");
    assert_eq!(request(&app, &cookie, &card, Some(&body)).await.0, StatusCode::SEE_OTHER);
    let (status, html) = request(&app, &cookie, &card, Some(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(html.contains("project revision"));
    let (status, html) =
        request(&app, &cookie, &format!("{base}?layout=table&priority=high&column=&iteration=&sort=target_date&overdue=true"), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<table"));
    assert!(html.contains("Overdue"));
    assert!(html.contains("2020-01-01"));
    assert!(html.contains("&#60;script&#62;visible&#60;/script&#62;"));
    assert_eq!(
        request(&app, &cookie, &format!("{base}/iterations"), Some("title=Sprint&starts_on=2026-11-01&ends_on=2026-11-14")).await.0,
        StatusCode::SEE_OTHER
    );
    let criteria =
        percent_encoding::utf8_percent_encode(r#"{"layout":"table","priority":"high"}"#, percent_encoding::NON_ALPHANUMERIC).to_string();
    assert_eq!(
        request(&app, &cookie, &format!("{base}/views"), Some(&format!("title=Mine&criteria={criteria}"))).await.0,
        StatusCode::SEE_OTHER
    );
    let views = db.projects().views(org, user, board).await.unwrap();
    assert_eq!(views.len(), 1);
    assert!(!views[0].shared);
    let (status, html) = request(&app, &cookie, &format!("{base}?view={}", views[0].id), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("<table"));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&card)
                .header(header::COOKIE, &cookie)
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "https://attacker.example")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
