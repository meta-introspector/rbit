//! Scope expansion, revocation and parent MFA are observed through existing bearer credentials.
use super::fixture::{Fixture, REPO};
use axum::http::StatusCode;
use rbit_core::{User, Visibility};
use rbit_identity_models::agents::Capability;
use rbit_services::CreateRepository;
use serde_json::json;

#[tokio::test]
async fn agent_http_org_scope_follows_future_repositories_and_live_authority() {
    let fixture = Fixture::new().await;
    let credential = fixture.agent(vec![Capability::Read], true).await;
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    assert_eq!(fixture.request(&credential.secret, "GET", "/repos/delegator/outside-public", json!(null)).await.0, StatusCode::NOT_FOUND);
    fixture
        .services
        .repositories
        .create_in(
            &fixture.owner,
            "agent-team",
            CreateRepository { name: "future".into(), description: String::new(), visibility: Visibility::Private },
        )
        .await
        .unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", "/repos/agent-team/future", json!(null)).await.0, StatusCode::OK);
    fixture.db.admin().set_suspended(fixture.owner.id(), true).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
    fixture.db.admin().set_suspended(fixture.owner.id(), false).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    fixture.services.agents.set_organization_policy(&fixture.owner, fixture.org, false).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::NOT_FOUND);
    fixture.services.agents.set_organization_policy(&fixture.owner, fixture.org, true).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
    fixture.services.agents.revoke_credential(&fixture.owner, credential.record.account_id, credential.token.id).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn agent_http_requires_owner_linked_credentials_and_survives_no_orphan_fallback() {
    let fixture = Fixture::new().await;
    let credential = fixture.agent(vec![Capability::Read], false).await;
    let agent = fixture.db.accounts().find_user(credential.record.account_id).await.unwrap().unwrap();
    let tokens = fixture.db.access_tokens();
    let accounts = fixture.db.accounts();
    let application = rbit_identity::access_tokens::AccessTokenApplication::new(&tokens, &accounts);
    let (_, unlinked) = application.create(&agent, "unlinked", rbit_core::TokenScope::Write, None).await.unwrap();
    assert_eq!(fixture.request(&unlinked, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
    fixture.db.admin().delete_user(fixture.owner.id()).await.unwrap();
    let orphan = fixture.db.accounts().find_user(agent.id()).await.unwrap().unwrap();
    assert!(orphan.is_agent);
    assert_eq!(orphan.agent_owner_id, None);
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
}

pub(super) async fn enroll(fixture: &Fixture, user: &User) {
    fixture.services.two_factor.begin_enrolment(user, "Agent policy test").await.unwrap();
    let record = fixture.db.two_factor().find(user.id()).await.unwrap().unwrap();
    let secret = rbit_auth::TotpSecret::from_bytes(&record.secret).unwrap();
    let step = rbit_auth::totp::time_step(rbit_core::Timestamp::now().seconds());
    let code = format!("{:06}", secret.code_at_step(step));
    fixture.services.confirm_two_factor(user, &code, &rbit_identity_models::sessions::ClientInfo::default()).await.unwrap().unwrap();
}

#[tokio::test]
async fn agent_http_enforces_the_parent_mfa_policy_on_existing_credentials() {
    let fixture = Fixture::new().await;
    let credential = fixture.agent(vec![Capability::Read], false).await;
    enroll(&fixture, &fixture.admin).await;
    fixture.services.authentication_policy.set_site(&fixture.admin, true).await.unwrap();
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::UNAUTHORIZED);
    enroll(&fixture, &fixture.owner).await;
    assert_eq!(fixture.request(&credential.secret, "GET", REPO, json!(null)).await.0, StatusCode::OK);
}
