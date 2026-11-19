//! LFS actions retain live credentials and refuse legacy signed agent grants.
use super::fixture::Fixture;
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rbit_core::lfs::{LfsOid, LfsOperation};
use rbit_identity_models::agents::Capability;
use rbit_services::lfs::TokenSubject;
use serde_json::{Value, json};
use tower::ServiceExt;
const OID: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
async fn request(f: &Fixture, auth: &str, method: &str, path: &str, body: &[u8]) -> (StatusCode, Vec<u8>) {
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::HOST, "localhost")
                .header(header::AUTHORIZATION, auth)
                .header(header::CONTENT_TYPE, "application/vnd.git-lfs+json")
                .header(header::CONTENT_LENGTH, body.len())
                .body(Body::from(body.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    (status, axum::body::to_bytes(response.into_body(), 2_000_000).await.unwrap().to_vec())
}
#[tokio::test]
async fn agent_lfs_transfers_recheck_capability_credential_and_parent_mfa() {
    let f = Fixture::new().await;
    let mut c = f.agent(vec![Capability::Read, Capability::Push], false).await;
    let basic = format!("Basic {}", STANDARD.encode(format!("agent:{}", c.secret)));
    let body = serde_json::to_vec(&json!({"operation":"upload","objects":[{"oid":OID,"size":5}]})).unwrap();
    let (status, response) = request(&f, &basic, "POST", "/agent-team/scoped.git/info/lfs/objects/batch", &body).await;
    assert_eq!(status, StatusCode::OK);
    let response: Value = serde_json::from_slice(&response).unwrap();
    let action = &response["objects"][0]["actions"]["upload"];
    let href: axum::http::Uri = action["href"].as_str().unwrap().parse().unwrap();
    let auth = action["header"]["Authorization"].as_str().unwrap();
    assert_eq!(auth, basic);
    f.capabilities(&mut c, vec![Capability::Read]).await;
    assert_eq!(request(&f, auth, "PUT", href.path(), b"hello").await.0, StatusCode::FORBIDDEN);
    f.capabilities(&mut c, vec![Capability::Read, Capability::Push]).await;
    assert_eq!(request(&f, auth, "PUT", href.path(), b"hello").await.0, StatusCode::OK);
    super::revocation::enroll(&f, &f.admin).await;
    f.services.authentication_policy.set_site(&f.admin, true).await.unwrap();
    assert_eq!(request(&f, auth, "PUT", href.path(), b"hello").await.0, StatusCode::UNAUTHORIZED);
    super::revocation::enroll(&f, &f.owner).await;
    assert_eq!(request(&f, auth, "GET", href.path(), b"").await.1, b"hello");
    f.services.agents.revoke_credential(&f.owner, c.record.account_id, c.token.id).await.unwrap();
    assert_eq!(request(&f, auth, "PUT", href.path(), b"hello").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(request(&f, auth, "GET", href.path(), b"").await.0, StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn agent_lfs_legacy_signed_grants_cannot_bypass_live_credentials() {
    let f = Fixture::new().await;
    let c = f.agent(vec![Capability::Read, Capability::Push], false).await;
    let agent = f.db.accounts().find_user(c.record.account_id).await.unwrap().unwrap();
    let repo = f.services.repositories.open(Some(&agent), "agent-team", "scoped").await.unwrap();
    let oid = LfsOid::parse(OID).unwrap();
    let legacy = f.services.lfs.issue(&repo, TokenSubject::User(agent.id()), LfsOperation::Upload, Some((&oid, 5)));
    let path = format!("/agent-team/scoped.git/info/lfs/objects/{OID}");
    assert_eq!(request(&f, &legacy.authorization, "PUT", &path, b"hello").await.0, StatusCode::FORBIDDEN);
}
