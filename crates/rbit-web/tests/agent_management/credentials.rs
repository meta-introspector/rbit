//! Credential issuance/rotation are owner-only, one-time and uncached.
use super::support::{PASSWORD, cookie, create, form, request, secret};
use crate::agents::fixture::Fixture;
use axum::http::{StatusCode, header};

#[tokio::test]
async fn agent_management_credentials_are_issued_once_rotated_and_revoked() {
    let fixture = Fixture::new().await;
    let session = cookie(&fixture, &fixture.owner).await;
    let agent = create(&fixture, &session).await;
    let base = format!("/settings/agents/{}", agent.account_id.0);
    let revision = agent.revision.to_string();
    let body = form(&[("revision", &revision), ("name", "HTTP automation"), ("scope", "write"), ("current_password", PASSWORD)]);
    let (status, headers, html) = request(&fixture, &session, &format!("{base}/credentials"), Some(&body), "http://localhost", "en").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(html.contains("hx-history=\"false\""));
    let issued = secret(&html);
    assert!(issued.starts_with("rbit_"));
    let (status, _, html) = request(&fixture, &session, &base, None, "http://localhost", "en").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains(&issued));
    let tokens = fixture.services.agents.managed_credentials(&fixture.owner, agent.account_id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    let expires = tokens[0].token.expires_at.unwrap().seconds() - tokens[0].token.created_at.seconds();
    assert!(expires.abs_diff(30 * 24 * 60 * 60) <= 2);
    let bad = form(&[("revision", "-1"), ("name", "stale"), ("scope", "write"), ("current_password", PASSWORD)]);
    let (status, _, html) = request(&fixture, &session, &format!("{base}/credentials"), Some(&bad), "http://localhost", "en").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!html.contains(&issued));
    assert!(!html.contains("id=\"agent-secret\""));
    let body = form(&[("revision", &revision), ("current_password", PASSWORD)]);
    let (status, headers, html) =
        request(&fixture, &session, &format!("{base}/credentials/{}/rotate", tokens[0].id), Some(&body), "http://localhost", "en").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    let rotated = secret(&html);
    // A failing assertion must not print plaintext credential material.
    assert_ne!(
        rbit_auth::AccessTokenSecret::parse(&rotated).unwrap().digest(),
        rbit_auth::AccessTokenSecret::parse(&issued).unwrap().digest()
    );
    assert!(fixture.services.tokens.authenticate_limited(&issued).await.unwrap().is_none());
    assert!(fixture.services.tokens.authenticate_limited(&rotated).await.unwrap().is_some());
    let tokens = fixture.services.agents.managed_credentials(&fixture.owner, agent.account_id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        request(&fixture, &session, &format!("{base}/credentials/{}/revoke", tokens[0].id), Some(""), "http://localhost", "en").await.0,
        StatusCode::SEE_OTHER
    );
    assert!(fixture.services.tokens.authenticate_limited(&rotated).await.unwrap().is_none());
}
