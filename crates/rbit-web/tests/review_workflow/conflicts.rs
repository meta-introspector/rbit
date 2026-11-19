//! HTTP conflict previews, origin/scope enforcement, retained drafts and immediate redirects.
use super::*;

async fn page(f: &Fixture, path: &str) -> (StatusCode, String) {
    let request = Request::builder().uri(path).header("cookie", &f.cookie).body(Body::empty()).unwrap();
    let response = f.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

async fn apply(f: &Fixture, base: &str, content: &str, origin: &str) -> axum::response::Response {
    let form = format!("expected_head={}&expected_base={base}&count=1&path_0=README&content_0={content}", f.head);
    let request = Request::builder()
        .method("POST")
        .uri("/reviewer/demo/pull/1/conflicts")
        .header("cookie", &f.cookie)
        .header("origin", origin)
        .header("host", "localhost")
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(form))
        .unwrap();
    f.router.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn browser_conflicts_http_scopes_origins_drafts_and_immediate_projection() {
    let f = Fixture::setup(true).await;
    assert_eq!(f.api("GET", "/conflicts", &f.read_token, None).await.0, StatusCode::FORBIDDEN);
    let (status, preview) = f.api("GET", "/conflicts", &f.token, None).await;
    assert_eq!(status, StatusCode::OK, "{preview}");
    assert_eq!(preview["files"][0]["path"], "README");
    let base = preview["base_sha"].as_str().unwrap();
    let (status, html) = page(&f, "/reviewer/demo/pull/1/conflicts").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Commit resolution to source branch"));
    assert!(html.contains("name=\"expected_base\""), "the preview binds the target snapshot: {html}");
    assert!(html.contains("&#60;&#60;&#60;&#60;&#60;&#60;&#60;"), "the conflict marker is HTML escaped: {html}");
    assert!(!html.contains("<<<<<<<"), "the raw conflict marker must not enter the HTML");
    assert_eq!(apply(&f, base, "resolved", "https://hostile.example").await.status(), StatusCode::FORBIDDEN);
    let response = apply(&f, base, "%3C%3C%3C%3C%3C%3C%3C%20draft", "http://localhost").await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let html = String::from_utf8(axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024).await.unwrap().to_vec()).unwrap();
    assert!(html.contains("Your draft is preserved below"));
    assert!(!html.contains("name=\"expected_base\""), "a refused draft must never be rebound to new branch snapshots");
    let response = apply(&f, base, "resolved%0A", "http://localhost").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()["location"], "/reviewer/demo/pull/1/files");
    let (status, html) = page(&f, "/reviewer/demo/pull/1/files").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("resolved"));
    assert_eq!(apply(&f, base, "old%0A", "http://localhost").await.status(), StatusCode::CONFLICT);
}
