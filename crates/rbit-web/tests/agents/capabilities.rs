//! Shared issue routes use the actual thread kind and current capabilities on every call.
use super::fixture::{Fixture, REPO};
use axum::http::StatusCode;
use rbit_identity_models::agents::Capability;
use serde_json::json;

#[tokio::test]
async fn agent_http_capabilities_follow_actual_issue_and_pull_comment_threads() {
    let fixture = Fixture::new().await;
    let mut credential = fixture.agent(vec![Capability::Read, Capability::Issues], false).await;
    let issue_comments = format!("{REPO}/issues/1/comments");
    let pull_comments = format!("{REPO}/issues/2/comments");
    let (status, issue_comment) = fixture.request(&credential.secret, "POST", &issue_comments, json!({"body":"Issue comment"})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        fixture.request(&credential.secret, "POST", &pull_comments, json!({"body":"Denied pull comment"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        fixture.request(&credential.secret, "POST", &format!("{REPO}/issues"), json!({"title":"Created by agent"})).await.0,
        StatusCode::CREATED
    );
    fixture.capabilities(&mut credential, vec![Capability::Read, Capability::PullRequests]).await;
    assert_eq!(
        fixture.request(&credential.secret, "POST", &issue_comments, json!({"body":"Denied issue comment"})).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, pull_comment) = fixture.request(&credential.secret, "POST", &pull_comments, json!({"body":"Pull comment"})).await;
    assert_eq!(status, StatusCode::CREATED);
    let issue_edit = format!("{REPO}/issues/comments/{}", issue_comment["id"].as_i64().unwrap());
    let pull_edit = format!("{REPO}/issues/comments/{}", pull_comment["id"].as_i64().unwrap());
    assert_eq!(fixture.request(&credential.secret, "PATCH", &issue_edit, json!({"body":"Denied edit"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(fixture.request(&credential.secret, "PATCH", &pull_edit, json!({"body":"Updated pull comment"})).await.0, StatusCode::OK);
    fixture.capabilities(&mut credential, vec![Capability::Read]).await;
    assert_eq!(
        fixture.request(&credential.secret, "POST", &pull_comments, json!({"body":"Capability removed"})).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(fixture.request(&credential.secret, "GET", &pull_comments, json!(null)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn agent_http_sensitive_operations_require_explicit_non_admin_rights() {
    let fixture = Fixture::new().await;
    let credential = fixture.agent(vec![Capability::Read, Capability::PullRequests], false).await;
    for (method, suffix, body) in [
        ("GET", "/actions/runs/1/logs", json!(null)),
        ("PUT", "/pulls/2/merge", json!({})),
        ("POST", "/actions/runs/1/approve", json!({})),
        ("POST", "/pulls/2/reviews", json!({"event":"APPROVE"})),
        ("POST", "/pulls/2/reviews", json!({"event":"REQUEST_CHANGES"})),
        ("PATCH", "", json!({"description":"Cannot administer"})),
        ("GET", "/hooks", json!(null)),
    ] {
        assert_eq!(
            fixture.request(&credential.secret, method, &format!("{REPO}{suffix}"), body).await.0,
            StatusCode::FORBIDDEN,
            "{method} {suffix}"
        );
    }
    for (method, path, body) in [
        ("GET", "/user/repos", json!(null)),
        ("GET", "/user/tokens", json!(null)),
        ("POST", "/user/tokens", json!({"name":"escalate","scope":"admin"})),
    ] {
        assert_eq!(fixture.request(&credential.secret, method, path, body).await.0, StatusCode::FORBIDDEN);
    }
    let agent = fixture.db.accounts().find_user(credential.record.account_id).await.unwrap().unwrap();
    assert!(fixture.services.accounts.start_session(&agent).await.is_err());
    assert!(fixture.services.tokens.create(&agent, rbit_services::CreateAccessToken::default()).await.is_err());
}

#[tokio::test]
async fn agent_read_only_scope_never_inherits_human_reader_comment_privileges() {
    use rbit_identity_models::agents::{AgentAccess, AgentScope};
    let fixture = Fixture::new().await;
    let credential = fixture.agent(vec![Capability::Read, Capability::Issues, Capability::PullRequests], false).await;
    let mut input = credential.input;
    input.scopes = vec![AgentScope::Repository { id: fixture.repo.0, access: AgentAccess::Read }];
    fixture.services.agents.update(&fixture.owner, credential.record.account_id, credential.record.revision, &input, true).await.unwrap();
    for (path, body) in [
        (format!("{REPO}/issues"), json!({"title":"Cannot write"})),
        (format!("{REPO}/issues/1/comments"), json!({"body":"Cannot comment"})),
        (format!("{REPO}/issues/2/comments"), json!({"body":"Cannot comment on PR"})),
    ] {
        assert_eq!(fixture.request(&credential.secret, "POST", &path, body).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(fixture.request(&credential.secret, "GET", &format!("{REPO}/issues/1"), json!(null)).await.0, StatusCode::OK);
}
