//! Live environment approvals and ownership fences on cloud identity issuance.
use axum::http::StatusCode;
use rbit_core::{
    Permission,
    actions::{ReviewDecision, RunEvent},
};
use rbit_services::actions::environments::EnvironmentInput;
use rbit_storage::NewUser;

use crate::{
    actions_oidc::{claim, get, issued, job, seed, seed_by},
    actions_runner::Fixture,
};

async fn bob(fixture: &Fixture, permission: Permission) -> rbit_core::User {
    let user = fixture
        .database
        .accounts()
        .create_user(NewUser {
            name: "bob".parse().unwrap(),
            email: "bob@example.com".parse().unwrap(),
            display_name: None,
            password_hash: "not-a-password-hash".to_owned(),
            is_admin: false,
        })
        .await
        .unwrap();
    fixture.services.collaborators.add(&fixture.alice, &fixture.repo().await, "bob", permission).await.unwrap();
    user
}

#[tokio::test]
async fn oidc_environment_approval_loses_authority_with_reviewer_access() {
    let fixture = Fixture::new().await;
    let bob = bob(&fixture, Permission::Read).await;
    let repo = fixture.repo().await;
    let settings = EnvironmentInput { reviewers: vec!["bob".to_owned()], prevent_self_review: true, ..EnvironmentInput::default() };
    fixture.actions().put_environment(&fixture.alice, &repo, "production", &settings).await.unwrap();
    let mut job = job(true);
    job.environment = Some("production".to_owned());
    let run = seed(&fixture, &job, RunEvent::Push, true, false).await;
    let pending = fixture.actions().pending_deployments(Some(&bob), &repo, &run).await.unwrap();
    assert_eq!(pending.len(), 1, "no task may mint before deployment approval");
    let reviewer_repo = fixture.services.repositories.open(Some(&bob), "alice", "rocket").await.unwrap();
    fixture
        .actions()
        .review_deployments(&bob, &reviewer_repo, &run, &["production".to_owned()], ReviewDecision::Approved, "ready")
        .await
        .unwrap();
    let spec = claim(&fixture).await;
    issued(&fixture, &spec).await;
    fixture.services.collaborators.remove(&fixture.alice, &repo, "bob").await.unwrap();
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn oidc_self_review_rechecks_original_actor_even_with_another_trigger() {
    let fixture = Fixture::new().await;
    let bob = bob(&fixture, Permission::Write).await;
    let repo = fixture.repo().await;
    let mut settings = EnvironmentInput { reviewers: vec!["alice".to_owned()], ..EnvironmentInput::default() };
    fixture.actions().put_environment(&fixture.alice, &repo, "production", &settings).await.unwrap();
    let mut job = job(true);
    job.environment = Some("production".to_owned());
    let run = seed_by(&fixture, &job, RunEvent::Push, true, false, fixture.alice.id(), bob.id()).await;
    fixture
        .actions()
        .review_deployments(&fixture.alice, &repo, &run, &["production".to_owned()], ReviewDecision::Approved, "ready")
        .await
        .unwrap();
    let spec = claim(&fixture).await;
    issued(&fixture, &spec).await;
    settings.prevent_self_review = true;
    fixture.actions().put_environment(&fixture.alice, &repo, "production", &settings).await.unwrap();
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn oidc_current_trigger_grants_and_ownership_tenure_are_fenced() {
    let fixture = Fixture::new().await;
    let bob = bob(&fixture, Permission::Write).await;
    let repo = fixture.repo().await;
    seed_by(&fixture, &job(true), RunEvent::Push, true, false, fixture.alice.id(), bob.id()).await;
    let spec = claim(&fixture).await;
    issued(&fixture, &spec).await;
    fixture.services.collaborators.remove(&fixture.alice, &repo, "bob").await.unwrap();
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
    fixture.services.collaborators.add(&fixture.alice, &repo, "bob", Permission::Write).await.unwrap();
    issued(&fixture, &spec).await;
    fixture.services.transfers.transfer(&fixture.alice, &repo, "bob", "alice/rocket").await.unwrap();
    let moved = fixture.services.repositories.open(Some(&fixture.alice), "bob", "rocket").await.unwrap();
    fixture.services.transfers.transfer(&fixture.alice, &moved, "alice", "bob/rocket").await.unwrap();
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn oidc_environment_branch_policy_is_rechecked_for_running_tasks() {
    let fixture = Fixture::new().await;
    let repo = fixture.repo().await;
    fixture.actions().put_environment(&fixture.alice, &repo, "production", &EnvironmentInput::default()).await.unwrap();
    let mut job = job(true);
    job.environment = Some("production".to_owned());
    seed(&fixture, &job, RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    issued(&fixture, &spec).await;
    let policy = EnvironmentInput {
        branch_policy: rbit_core::actions::BranchPolicy::Selected(vec!["release/*".to_owned()]),
        ..EnvironmentInput::default()
    };
    fixture.actions().put_environment(&fixture.alice, &repo, "production", &policy).await.unwrap();
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn oidc_successful_issuance_quota_is_atomic_and_resets_each_minute() {
    let fixture = Fixture::new().await;
    seed(&fixture, &job(true), RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    let digest = rbit_auth::TaskTokenSecret::parse(spec.token.expose()).unwrap().digest();
    let now = rbit_core::Timestamp::now().0;
    let store = fixture.database.actions_oidc();
    assert!(store.issue(&digest, now, 0, |_| None::<()>).await.unwrap().is_none(), "failed signing spends no quota");
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..61 {
        let store = store.clone();
        requests.spawn(async move { store.issue(&digest, now, 0, |_| Some(())).await.unwrap().is_some() });
    }
    let mut admitted = 0;
    while let Some(result) = requests.join_next().await {
        admitted += usize::from(result.unwrap());
    }
    assert_eq!(admitted, 60, "concurrent callers share one persisted task budget");
    assert!(store.issue(&digest, now + 60, 0, |_| Some(())).await.unwrap().is_some());
}
