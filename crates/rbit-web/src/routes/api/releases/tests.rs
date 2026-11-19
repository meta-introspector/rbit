//! Release API and public-feed integration regressions.

use std::{path::Path as FsPath, process::Command};

use rbit_core::{Visibility, membership::Permission};
use serde_json::json;

use super::{super::test_support::Fixture, *};

fn git(dir: &FsPath, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Pushes `main` with one commit and the tag `v1` into `alice/{name}`.
async fn push_history(fixture: &Fixture, owner: &rbit_core::User, name: &str) {
    let ctx = fixture.services.repositories.open(Some(owner), owner.name().as_str(), name).await.unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("README.md"), "hello\n").unwrap();
    git(work.path(), &["init", "--quiet", "--initial-branch=main"]);
    git(work.path(), &["add", "README.md"]);
    git(work.path(), &["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit", "--quiet", "-m", "initial"]);
    git(work.path(), &["tag", "v1"]);
    git(work.path(), &["push", "--quiet", ctx.disk_path.to_str().unwrap(), "main", "v1"]);
}

#[test]
fn accept_headers_select_the_bytes() {
    let accept = |value: &'static str| {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(value));
        wants_bytes(&headers)
    };
    assert!(accept("application/octet-stream"));
    assert!(accept("application/json, Application/Octet-Stream;q=0.9"));
    assert!(!accept("application/json"));
    assert!(!accept("*/*"));
    assert!(!wants_bytes(&HeaderMap::new()));
}

#[tokio::test]
#[expect(clippy::too_many_lines, reason = "one walk through the release endpoints")]
async fn releases_and_files_through_the_api() {
    let fixture = Fixture::new(|config| config.releases.max_asset_bytes = 16).await;
    let alice = fixture.user("alice").await;
    let bob = fixture.user("bob").await;
    let carol = fixture.user("carol").await;
    fixture.repo(&alice, "demo", Visibility::Public).await;
    fixture.repo(&alice, "vault", Visibility::Private).await;
    push_history(&fixture, &alice, "demo").await;
    let ctx = fixture.services.repositories.open(Some(&alice), "alice", "demo").await.unwrap();
    fixture.services.collaborators.add(&alice, &ctx, "bob", Permission::Read).await.unwrap();
    let write = fixture.token(&alice, TokenScope::Write).await;
    let read = fixture.token(&alice, TokenScope::Read).await;
    let bobs = fixture.token(&bob, TokenScope::Admin).await;
    let carols = fixture.token(&carol, TokenScope::Admin).await;
    let api = "/api/v1/repos/alice/demo/releases";

    // A release of an existing tag, and one whose tag is created from the default branch.
    let (response, v1) = fixture.send("POST", api, Some(&write), Some(json!({ "tag_name": "v1", "name": "One", "body": "Notes" }))).await;
    assert_eq!(response.status(), StatusCode::CREATED, "{v1}");
    let id = v1["id"].as_i64().unwrap();
    let url = format!("https://git.example.com{api}/{id}");
    assert_eq!((response.headers()[header::LOCATION].to_str().unwrap(), v1["url"].as_str()), (url.as_str(), Some(url.as_str())));
    assert_eq!((v1["tag_name"].as_str(), v1["name"].as_str(), v1["body"].as_str()), (Some("v1"), Some("One"), Some("Notes")));
    assert_eq!((v1["draft"].as_bool(), v1["author"]["login"].as_str()), (Some(false), Some("alice")));
    assert_eq!(v1["upload_url"], format!("{url}/assets{{?name,label}}"));
    let (response, v2) =
        fixture.send("POST", api, Some(&write), Some(json!({ "tag_name": "v2", "draft": true, "prerelease": true }))).await;
    assert_eq!(response.status(), StatusCode::CREATED, "{v2}");
    assert_eq!((v2["name"].as_str(), v2["published_at"].clone()), (Some("v2"), Value::Null), "the title defaults to the tag");
    let draft = format!("{api}/{}", v2["id"]);

    for (body, status, field) in [
        (json!({}), StatusCode::UNPROCESSABLE_ENTITY, "tag_name"),
        (json!({ "tag_name": "v3", "target_commitish": "nope" }), StatusCode::UNPROCESSABLE_ENTITY, "target_commitish"),
        (json!({ "tag_name": "bad..tag" }), StatusCode::UNPROCESSABLE_ENTITY, "tag_name"),
        (json!({ "tag_name": "v3", "name": "x".repeat(300) }), StatusCode::UNPROCESSABLE_ENTITY, "name"),
        (json!({ "tag_name": "v1" }), StatusCode::CONFLICT, ""),
    ] {
        let (response, error) = fixture.send("POST", api, Some(&write), Some(body.clone())).await;
        assert_eq!(response.status(), status, "{body}: {error}");
        if !field.is_empty() {
            assert_eq!(error["errors"][0]["field"], field, "{body}: {error}");
        }
    }

    // Reading: drafts for writers only; latest skips drafts; by tag.
    let (response, list) = fixture.send("GET", api, Some(&write), None).await;
    assert_eq!((response.status(), response.headers()["x-total-count"].to_str().unwrap()), (StatusCode::OK, "2"));
    assert_eq!(list[0]["tag_name"], "v2", "drafts first");
    let (response, list) = fixture.send("GET", api, Some(&read), None).await;
    assert_eq!(
        (response.headers()["x-total-count"].to_str().unwrap(), list[0]["tag_name"].as_str()),
        ("1", Some("v1")),
        "a read token sees what readers see"
    );
    assert_eq!(fixture.send("GET", api, None, None).await.0.status(), StatusCode::OK, "public");
    assert_eq!(fixture.send("GET", &draft, Some(&bobs), None).await.0.status(), StatusCode::NOT_FOUND, "drafts are hidden");
    assert_eq!(fixture.send("GET", &draft, Some(&write), None).await.0.status(), StatusCode::OK);
    assert_eq!(fixture.send("GET", &format!("{api}/latest"), None, None).await.1["tag_name"], "v1");
    assert_eq!(fixture.send("GET", &format!("{api}/tags/v1"), None, None).await.1["id"], id);
    assert_eq!(fixture.send("GET", &format!("{api}/tags/v2"), None, None).await.0.status(), StatusCode::NOT_FOUND);
    for path in [format!("{api}/999"), format!("{api}/x"), "/api/v1/repos/alice/vault/releases".to_owned()] {
        assert_eq!(fixture.send("GET", &path, Some(&carols), None).await.0.status(), StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(
        fixture.send("GET", "/api/v1/repos/alice/vault/releases/latest", Some(&write), None).await.0.status(),
        StatusCode::NOT_FOUND
    );

    // Changing: absent fields stay; publishing the draft.
    let (response, changed) = fixture.send("PATCH", &draft, Some(&write), Some(json!({ "draft": false, "body": "Now" }))).await;
    assert_eq!(response.status(), StatusCode::OK, "{changed}");
    assert_eq!(
        (changed["draft"].as_bool(), changed["prerelease"].as_bool(), changed["body"].as_str()),
        (Some(false), Some(true), Some("Now"))
    );
    assert!(changed["published_at"].is_string());
    let (response, error) = fixture.send("PATCH", &draft, Some(&write), Some(json!({ "tag_name": "v9" }))).await;
    assert_eq!((response.status(), error["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("tag_name")));
    assert_eq!(fixture.send("GET", &format!("{api}/latest"), None, None).await.1["tag_name"], "v1", "full releases first");

    // Scopes and rights.
    assert_eq!(fixture.send("POST", api, Some(&read), Some(json!({ "tag_name": "v1" }))).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.send("PATCH", &draft, Some(&bobs), Some(json!({}))).await.0.status(), StatusCode::FORBIDDEN, "a reader");
    assert_eq!(fixture.send("DELETE", &draft, Some(&bobs), None).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.send("POST", api, Some(&carols), Some(json!({ "tag_name": "v1" }))).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.send("POST", api, None, Some(json!({ "tag_name": "v1" }))).await.0.status(), StatusCode::UNAUTHORIZED);

    // Files: raw bodies, limits, conflicts.
    let upload = |name: &str| format!("{api}/{id}/assets?name={name}&label=ignored");
    let (response, file) =
        fixture.raw("POST", &upload("app.zip"), Some(&write), Some("application/zip"), Some(b"0123456789".to_vec())).await;
    assert_eq!(response.status(), StatusCode::CREATED, "{file:?}");
    let file: Value = serde_json::from_slice(&file).unwrap();
    assert_eq!(
        (file["name"].as_str(), file["size"].as_u64(), file["content_type"].as_str()),
        (Some("app.zip"), Some(10), Some("application/zip"))
    );
    assert_eq!((file["uploader"]["login"].as_str(), file["label"].clone()), (Some("alice"), Value::Null));
    let file_url = format!("https://git.example.com{api}/assets/{}", file["id"]);
    assert_eq!(
        (file["url"].as_str(), response.headers()[header::LOCATION].to_str().unwrap()),
        (Some(file_url.as_str()), file_url.as_str())
    );
    let (response, _) = fixture.raw("POST", &upload("big.bin"), Some(&write), None, Some(vec![b'x'; 17])).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "Content-Length over the limit");
    let (response, _) = fixture.raw("POST", &upload("app.zip"), Some(&write), None, Some(b"x".to_vec())).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    for bad in [format!("{api}/{id}/assets"), upload(".env"), upload("a%2Fb")] {
        let (response, error) = fixture.send("POST", &bad, Some(&write), None).await;
        assert_eq!((response.status(), error["errors"][0]["field"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("name")), "{bad}");
    }
    assert_eq!(fixture.raw("POST", &upload("x.bin"), Some(&read), None, Some(b"x".to_vec())).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.raw("POST", &upload("x.bin"), Some(&bobs), None, Some(b"x".to_vec())).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        fixture.raw("POST", &format!("{api}/999/assets?name=x"), Some(&write), None, Some(b"x".to_vec())).await.0.status(),
        StatusCode::NOT_FOUND
    );

    // Listing, metadata and the bytes.
    let (response, files) = fixture.send("GET", &format!("{api}/{id}/assets"), None, None).await;
    assert_eq!((response.status(), files.as_array().map(Vec::len)), (StatusCode::OK, Some(1)));
    let one = format!("{api}/assets/{}", file["id"]);
    let (response, metadata) = fixture.send("GET", &one, None, None).await;
    assert_eq!((response.status(), metadata["name"].as_str()), (StatusCode::OK, Some("app.zip")));
    let (response, bytes) = fixture.request("GET", &one, None, &[("accept", "application/octet-stream")], None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(&bytes[..], b"0123456789");
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/octet-stream");
    assert_eq!(response.headers()[header::CONTENT_DISPOSITION], "attachment; filename*=UTF-8''app%2Ezip");
    assert_eq!(response.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none(), "streamed, not compressed");
    assert_eq!(fixture.send("GET", &one, None, None).await.1["download_count"], 1);
    assert_eq!(fixture.send("GET", &format!("{api}/assets/999"), None, None).await.0.status(), StatusCode::NOT_FOUND);

    // Deleting files and releases.
    assert_eq!(fixture.send("DELETE", &one, Some(&bobs), None).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.send("DELETE", &one, Some(&write), None).await.0.status(), StatusCode::NO_CONTENT);
    assert_eq!(fixture.send("GET", &one, None, None).await.0.status(), StatusCode::NOT_FOUND);
    assert_eq!(fixture.send("DELETE", &draft, Some(&read), None).await.0.status(), StatusCode::FORBIDDEN);
    assert_eq!(fixture.send("DELETE", &draft, Some(&write), None).await.0.status(), StatusCode::NO_CONTENT);
    assert_eq!(fixture.send("GET", &draft, Some(&write), None).await.0.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn streamed_uploads_stop_at_the_limit() {
    let fixture = Fixture::new(|config| config.releases.max_asset_bytes = 1024).await;
    let alice = fixture.user("alice").await;
    fixture.repo(&alice, "demo", Visibility::Public).await;
    push_history(&fixture, &alice, "demo").await;
    let write = fixture.token(&alice, TokenScope::Write).await;
    let (_, release) = fixture.send("POST", "/api/v1/repos/alice/demo/releases", Some(&write), Some(json!({ "tag_name": "v1" }))).await;
    let path = format!("/api/v1/repos/alice/demo/releases/{}/assets?name=big.bin", release["id"]);

    // Chunked (no Content-Length): refused once more than the limit has arrived.
    let chunks = futures_util::stream::iter((0..8).map(|_| Ok::<_, io::Error>(axum::body::Bytes::from(vec![b'x'; 512]))));
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(&path)
        .header(header::AUTHORIZATION, format!("Bearer {write}"))
        .body(Body::from_stream(chunks))
        .unwrap();
    let response = fixture.oneshot(request).await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let (_, files) = fixture.send("GET", &format!("/api/v1/repos/alice/demo/releases/{}/assets", release["id"]), None, None).await;
    assert_eq!(files, json!([]), "nothing was kept");

    // Exactly the limit is fine.
    let (response, _) = fixture.raw("POST", &path, Some(&write), None, Some(vec![b'x'; 1024])).await;
    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn public_release_feed_is_minimal_and_never_uses_browser_authority() {
    use rbit_services::releases::ReleaseInput;
    let fixture = Fixture::new(|config| config.api.enabled = false).await;
    let alice = fixture.user("alice").await;
    let session = fixture.services.accounts.start_session(&alice).await.unwrap();
    let cookie = format!("rbit_session={}", session.encode());
    for (name, visibility) in [("demo", Visibility::Public), ("vault", Visibility::Private)] {
        fixture.repo(&alice, name, visibility).await;
        push_history(&fixture, &alice, name).await;
        let ctx = fixture.services.repositories.open(Some(&alice), "alice", name).await.unwrap();
        for (tag, draft, prerelease) in [("v1.0.0", false, false), ("v2.0.0", true, false), ("v3.0.0-rc.1", false, true)] {
            fixture
                .services
                .releases
                .create(
                    &ctx,
                    &alice,
                    ReleaseInput {
                        tag_name: tag.into(),
                        name: tag.into(),
                        notes: "PRIVATE_NOTES_MARKER".into(),
                        is_draft: draft,
                        is_prerelease: prerelease,
                        target: "main".into(),
                    },
                )
                .await
                .unwrap();
        }
    }
    let (response, value) = fixture.send("GET", "/alice/demo/releases/latest.json", None, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(value, json!({"tag_name":"v1.0.0", "draft":false, "prerelease":false}));
    for headers in [vec![], vec![("cookie", cookie.as_str())]] {
        let (response, _) = fixture.request("GET", "/alice/vault/releases/latest.json", None, &headers, None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let fixture = Fixture::new(|config| config.server.require_sign_in = true).await;
    let (response, _) = fixture.send("GET", "/alice/demo/releases/latest.json", None, None).await;
    assert_ne!(response.status(), StatusCode::OK);
}
