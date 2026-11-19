//! Agent identity and organization-policy forms require current ownership and Origin.
use super::support::{PASSWORD, agent_form, cookie, create, form, request};
use crate::agents::fixture::Fixture;
use axum::http::StatusCode;

#[tokio::test]
async fn agent_management_forms_resolve_friendly_scopes_fence_revisions_and_escape_names() {
    let fixture = Fixture::new().await;
    let session = cookie(&fixture, &fixture.owner).await;
    let (status, _, html) = request(&fixture, &session, "/settings/agents/new", None, "http://localhost", "es").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("Agentes"));
    assert!(html.contains("Repositorios con acceso de escritura"));
    let body = agent_form("0", "Helper", "");
    assert_eq!(
        request(&fixture, &session, "/settings/agents", Some(&body), "http://localhost", "en").await.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(fixture.services.agents.list(&fixture.owner).await.unwrap().len(), 0);
    let body = agent_form("0", "Helper", PASSWORD);
    assert_eq!(
        request(&fixture, &session, "/settings/agents", Some(&body), "https://attacker.example", "en").await.0,
        StatusCode::FORBIDDEN
    );
    let agent = create(&fixture, &session).await;
    let path = format!("/settings/agents/{}", agent.account_id.0);
    let (status, _, html) = request(&fixture, &session, &path, None, "http://localhost", "en").await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("agent-team/scoped"));
    assert!(html.contains("value=\"30\" selected"));
    assert!(html.contains("value=\"never\""));
    let revision = agent.revision.to_string();
    let body = agent_form(&revision, "Planner <script>", PASSWORD);
    assert_eq!(request(&fixture, &session, &path, Some(&body), "http://localhost", "en").await.0, StatusCode::SEE_OTHER);
    let (status, _, html) = request(&fixture, &session, &path, Some(&body), "http://localhost", "en").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!html.contains("Planner <script>"));
    let current = fixture.services.agents.find(&fixture.owner, agent.account_id).await.unwrap();
    assert_eq!(current.revision, agent.revision + 1);
    let body = form(&[("revision", &current.revision.to_string())]);
    assert_eq!(
        request(&fixture, &session, &format!("{path}/disable"), Some(&body), "http://localhost", "en").await.0,
        StatusCode::SEE_OTHER
    );
    assert!(!fixture.services.agents.find(&fixture.owner, agent.account_id).await.unwrap().enabled);
}

#[tokio::test]
async fn agent_management_policy_is_current_owner_only_and_origin_checked() {
    let fixture = Fixture::new().await;
    let session = cookie(&fixture, &fixture.owner).await;
    let path = "/agent-team/settings/agents";
    assert_eq!(request(&fixture, &session, path, None, "http://localhost", "en").await.0, StatusCode::OK);
    let body = form(&[("policy", "block"), ("current_password", PASSWORD)]);
    assert_eq!(request(&fixture, &session, path, Some(&body), "https://attacker.example", "en").await.0, StatusCode::FORBIDDEN);
    assert!(fixture.services.agents.organization_policy(&fixture.owner, fixture.org).await.unwrap());
    assert_eq!(request(&fixture, &session, path, Some(&body), "http://localhost", "en").await.0, StatusCode::SEE_OTHER);
    assert!(!fixture.services.agents.organization_policy(&fixture.owner, fixture.org).await.unwrap());
    // Site administrators do not replace the explicit organization-owner requirement.
    let admin = cookie(&fixture, &fixture.admin).await;
    assert_eq!(request(&fixture, &admin, path, None, "http://localhost", "en").await.0, StatusCode::NOT_FOUND);
}
