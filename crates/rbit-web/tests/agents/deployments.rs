//! Combined agent scopes/capabilities gate deployment history and atomic publication.
mod fixture;
mod publication;
use super::fixture::{Fixture, REPO};
use axum::http::StatusCode;
use fixture::prepare;
use rbit_core::Timestamp;
use rbit_identity_models::agents::Capability;

use serde_json::json;

#[tokio::test]
async fn agent_deployment_history_is_scoped_and_requests_require_dispatch() {
    let f = Fixture::new().await;
    f.db.actions_settings().set_enabled(f.repo, true, Timestamp::now()).await.unwrap();
    let agent = f.agent(vec![Capability::Read], false).await;
    assert_eq!(f.request(&agent.secret, "GET", &format!("{REPO}/deployments"), json!(null)).await.0, StatusCode::OK);
    assert_eq!(f.request(&agent.secret, "GET", "/repos/delegator/outside-public/deployments", json!(null)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(f.request(&agent.secret, "POST", &format!("{REPO}/deployments/requests"), json!({})).await.0, StatusCode::FORBIDDEN);
}
#[tokio::test]
async fn agent_deployment_dispatch_never_grants_workflow_config_or_human_approval() {
    let f = Fixture::new().await;
    let prepared = Box::pin(prepare(&f)).await;
    let agent = f.agent(vec![Capability::Read, Capability::Dispatch], false).await;
    let body = json!({"environment":"production","operation":"promote","source_attempt":prepared.source.id,"configuration_id":prepared.configuration.id,"configuration_generation":prepared.configuration.generation});
    let (status, run) = f.request(&agent.secret, "POST", &format!("{REPO}/deployments/requests"), body).await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    assert_eq!(
        f.request(&agent.secret, "PUT", &format!("{REPO}/deployments/workflows/production"), json!({})).await.0,
        StatusCode::FORBIDDEN
    );
    let run_id = run["id"].as_i64().or_else(|| run["run_id"].as_i64()).expect("created deployment run identity");
    assert_eq!(
        f.request(&agent.secret, "POST", &format!("{REPO}/actions/runs/{run_id}/approve"), json!({})).await.0,
        StatusCode::FORBIDDEN
    );
}
