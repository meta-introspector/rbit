//! Owner-only REST lifecycle, stable credentials and organization policy controls.
use super::fixture::{Fixture, REPO};
use axum::http::StatusCode;
use rbit_core::{TokenScope, User};
use rbit_services::CreateAccessToken;
use serde_json::{Value, json};

async fn owner_token(fixture: &Fixture, user: &User, scope: TokenScope, days: Option<u32>) -> String {
    fixture.services.tokens.create(user, CreateAccessToken { name: "management".into(), scope, expires_in_days: days }).await.unwrap().1
}
fn input(fixture: &Fixture) -> Value {
    json!({"slug":"api-helper", "name":"API helper", "capabilities":["read", "issues"],
        "scopes":[{"kind":"organization", "id":fixture.org.0, "access":"write"}]})
}

#[tokio::test]
async fn agent_management_api_fences_ownership_revisions_and_organization_policy() {
    let fixture = Fixture::new().await;
    let owner = owner_token(&fixture, &fixture.owner, TokenScope::Admin, None).await;
    let unrelated = owner_token(&fixture, &fixture.admin, TokenScope::Admin, None).await;
    let writer = owner_token(&fixture, &fixture.owner, TokenScope::Write, None).await;
    assert_eq!(fixture.request(&writer, "POST", "/user/agents", input(&fixture)).await.0, StatusCode::FORBIDDEN);
    let (status, created) = fixture.request(&owner, "POST", "/user/agents", input(&fixture)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["type"], "Bot");
    assert_eq!(created["owner_id"], fixture.owner.id().0);
    let path = format!("/user/agents/{}", created["id"].as_i64().unwrap());
    assert_eq!(fixture.request(&unrelated, "GET", &path, json!(null)).await.0, StatusCode::NOT_FOUND);
    let updated = json!({"revision":1, "enabled":true, "agent":input(&fixture)});
    assert_eq!(fixture.request(&owner, "PATCH", &path, updated.clone()).await.0, StatusCode::OK);
    assert_eq!(fixture.request(&owner, "PATCH", &path, updated).await.0, StatusCode::CONFLICT);
    assert_eq!(
        fixture.request(&owner, "POST", &format!("{path}/disable"), json!({"revision":0})).await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let policy = "/orgs/agent-team/agent-policy";
    assert_eq!(fixture.request(&unrelated, "PUT", policy, json!({"allow_agents":false})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(fixture.request(&owner, "PUT", policy, json!({"allow_agents":false})).await.0, StatusCode::OK);
    assert_eq!(fixture.request(&owner, "GET", policy, json!(null)).await.1["allow_agents"], false);
    assert_eq!(fixture.request(&owner, "POST", &format!("{path}/disable"), json!({"revision":2})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(fixture.request(&owner, "GET", &path, json!(null)).await.1["enabled"], false);
}

#[tokio::test]
async fn agent_management_api_rotates_once_and_never_lists_secrets() {
    let fixture = Fixture::new().await;
    let owner = owner_token(&fixture, &fixture.owner, TokenScope::Admin, None).await;
    let (_, created) = fixture.request(&owner, "POST", "/user/agents", input(&fixture)).await;
    let path = format!("/user/agents/{}/credentials", created["id"].as_i64().unwrap());
    let form = json!({"revision":1,"name":"API worker","scope":"write","expires_in_days":null});
    let (status, first) = fixture.request(&owner, "POST", &path, form.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    let secret = first["token"].as_str().unwrap();
    assert_eq!(fixture.request(secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    assert_eq!(fixture.request(secret, "GET", "/user/agents", json!(null)).await.0, StatusCode::FORBIDDEN);
    let (_, identity) = fixture.request(secret, "GET", "/user", json!(null)).await;
    assert_eq!(identity["type"], "Bot");
    assert_eq!(identity["agent_owner_id"], fixture.owner.id().0);
    assert_eq!(identity["site_admin"], false);
    let old = format!("{path}/{}", first["id"].as_i64().unwrap());
    let (status, second) = fixture.request(&owner, "POST", &format!("{old}/rotate"), form.clone()).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_ne!(first["id"], second["id"]);
    assert_eq!(fixture.request(secret, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(fixture.request(&owner, "DELETE", &old, json!(null)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(fixture.request(&owner, "POST", &format!("{old}/rotate"), form).await.0, StatusCode::NOT_FOUND);
    let (_, listed) = fixture.request(&owner, "GET", &path, json!(null)).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(listed[0].get("token").is_none());
    assert_eq!(listed[0]["id"], second["id"]);
    let fresh = second["token"].as_str().unwrap();
    assert_eq!(fixture.request(fresh, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    assert_eq!(
        fixture.request(&owner, "DELETE", &format!("{path}/{}", second["id"].as_i64().unwrap()), json!(null)).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(fixture.request(fresh, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_management_api_enforces_issuer_expiry_and_read_write_scopes() {
    let fixture = Fixture::new().await;
    let owner = owner_token(&fixture, &fixture.owner, TokenScope::Admin, Some(30)).await;
    let (_, created) = fixture.request(&owner, "POST", "/user/agents", input(&fixture)).await;
    let path = format!("/user/agents/{}/credentials", created["id"].as_i64().unwrap());
    for (scope, expiry) in [("admin", json!(7)), ("write", json!(null)), ("write", json!(365)), ("write", json!(8))] {
        assert_eq!(
            fixture.request(&owner, "POST", &path, json!({"revision":1,"name":"bad","scope":scope,"expires_in_days":expiry})).await.0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    assert_eq!(fixture.request(&owner, "GET", &path, json!(null)).await.1.as_array().unwrap().len(), 0);
    let (status, credential) =
        fixture.request(&owner, "POST", &path, json!({"revision":1,"name":"bounded","scope":"read","expires_in_days":7})).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(credential["expires_at"].is_string());
    let secret = credential["token"].as_str().unwrap();
    assert_eq!(fixture.request(secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    assert_eq!(fixture.request(secret, "POST", &format!("{REPO}/issues"), json!({"title":"Denied"})).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn agent_rotation_rechecks_actual_expiration_before_retiring_previous_credential() {
    let fixture = Fixture::new().await;
    let agent = fixture.agent(vec![rbit_identity_models::agents::Capability::Read], false).await;
    let metadata = fixture.services.agents.managed_credentials(&fixture.owner, agent.record.account_id).await.unwrap();
    let old = metadata[0].id;
    let result = fixture
        .services
        .agents
        .rotate_credential_bounded(
            &fixture.owner,
            agent.record.account_id,
            agent.record.revision,
            old,
            CreateAccessToken { name: "too long".into(), scope: TokenScope::Write, expires_in_days: Some(7) },
            Some(rbit_core::Timestamp::now()),
        )
        .await;
    assert!(result.is_err());
    assert_eq!(fixture.request(&agent.secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    let tokens = fixture.services.agents.managed_credentials(&fixture.owner, agent.record.account_id).await.unwrap();
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].id, old);
    assert_eq!(fixture.db.access_tokens().list_for_account(agent.record.account_id).await.unwrap().len(), 1);
}
