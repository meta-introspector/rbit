//! Deployment requests bind historical source identity to a gated current trusted workflow.
use crate::actions_fixture::Fixture;
use rbit_core::{Timestamp, actions::Status};
use rbit_services::{
    ServiceError,
    actions::{deployments::DeploymentRequest, dispatch::DispatchRequest, environments::EnvironmentInput},
};
mod lineage;
mod trust;

use std::collections::BTreeMap;

const WORKFLOW: &str = "name: Deploy current
on:
  workflow_dispatch:
    inputs:
      deployment_sha: {type: string, required: true}
      deployment_environment: {type: string, required: true}
      deployment_operation: {type: string, required: true}
jobs:
  deploy:
    runs-on: x
    environment: ${{ inputs.deployment_environment }}
    steps: [{run: echo current}]
";

async fn source(f: &Fixture, environment: &str) -> String {
    let sha = f.commit(&[(".forgejo/workflows/deploy.yml", WORKFLOW)], "Source deployment workflow");
    f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.carol).await;
    let inputs = BTreeMap::from([
        ("deployment_sha".into(), sha.clone()),
        ("deployment_environment".into(), environment.into()),
        ("deployment_operation".into(), "promote".into()),
    ]);
    f.actions()
        .dispatch(&f.carol, &repo, &DispatchRequest { workflow: "deploy.yml".into(), reference: "main".into(), inputs })
        .await
        .unwrap();
    let claimed = f.claim(&["x"], 75).await.unwrap();
    f.database.actions_runs().finish_task(claimed.task.id, Status::Success, Timestamp::now()).await.unwrap();
    sha
}

#[tokio::test]
async fn deployment_promotion_uses_current_workflow_exact_source_and_current_review_gate() {
    let f = Fixture::new().await;
    let selected = source(&f, "staging").await;
    let current = f.commit(
        &[(".forgejo/workflows/deploy.yml", &WORKFLOW.replace("echo current", "echo trusted-new-workflow"))],
        "New trusted workflow",
    );
    f.push_ref("refs/heads/main").await;
    let admin = f.repo(&f.alice).await;
    let writer = f.repo(&f.carol).await;
    f.actions()
        .put_environment(
            &f.alice,
            &admin,
            "production",
            &EnvironmentInput { reviewers: vec!["alice".into()], prevent_self_review: true, wait_timer: 1, ..Default::default() },
        )
        .await
        .unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
    let history = f.actions().deployment_dashboard(&writer, None).await.unwrap();
    let request = DeploymentRequest {
        environment: "production".into(),
        operation: "promote".into(),
        source_attempt: history.attempts[0].id,
        configuration_id: config.id,
        configuration_generation: config.generation,
        artifact_id: None,
        artifact_digest: None,
    };
    let run = f.actions().deploy(&f.carol, &writer, request.clone()).await.unwrap();
    assert_eq!(run.sha, current);
    assert!(
        matches!(
            f.actions().rerun(&f.carol, &writer, run.id, rbit_services::actions::runs::Rerun::All).await,
            Err(ServiceError::Invalid(_))
        ),
        "intent-bound deployments must never rerun historical workflows"
    );
    assert_ne!(run.sha, selected, "never execute the historical source workflow");
    let (_, inputs) = f.database.actions_runs().event(run.id).await.unwrap().unwrap();
    let inputs: serde_json::Value = serde_json::from_str(&inputs.unwrap()).unwrap();
    assert_eq!(inputs["deployment_sha"], selected);
    assert_eq!(inputs["deployment_environment"], "production");
    let jobs = f.database.actions_runs().jobs(run.id).await.unwrap();
    assert!(jobs[0].awaiting_review);
    assert!(f.claim(&["x"], 76).await.is_none());
    let reader = f.repo(&f.rita).await;
    assert!(matches!(f.actions().deploy(&f.rita, &reader, request.clone()).await, Err(ServiceError::Forbidden)));
    let stale = DeploymentRequest { configuration_generation: config.generation + 1, ..request.clone() };
    assert!(matches!(f.actions().deploy(&f.carol, &writer, stale).await, Err(ServiceError::Conflict(_))));
    f.services.collaborators.remove(&f.alice, &admin, "carol").await.unwrap();
    assert!(matches!(f.actions().deploy(&f.carol, &writer, request).await, Err(ServiceError::Forbidden)));
}

#[tokio::test]
async fn deployment_rollback_selects_exact_successful_production_history() {
    let f = Fixture::new().await;
    let production = source(&f, "production").await;
    let admin = f.repo(&f.alice).await;
    f.actions().put_environment(&f.alice, &admin, "staging", &EnvironmentInput::default()).await.unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
    let history = f.actions().deployment_dashboard(&admin, None).await.unwrap();
    let request = DeploymentRequest {
        environment: "production".into(),
        operation: "rollback".into(),
        source_attempt: history.attempts[0].id,
        configuration_id: config.id,
        configuration_generation: config.generation,
        artifact_id: None,
        artifact_digest: None,
    };
    let refused = DeploymentRequest { operation: "promote".into(), ..request.clone() };
    assert!(matches!(f.actions().deploy(&f.alice, &admin, refused).await, Err(ServiceError::Invalid(_))));
    let run = f.actions().deploy(&f.alice, &admin, request).await.unwrap();
    let (_, inputs) = f.database.actions_runs().event(run.id).await.unwrap().unwrap();
    let inputs: serde_json::Value = serde_json::from_str(&inputs.unwrap()).unwrap();
    assert_eq!(inputs["deployment_sha"], production);
    assert_eq!(inputs["deployment_operation"], "rollback");
}

#[tokio::test]
async fn deployment_artifacts_bind_exact_successful_source_run_attempt_and_digest() {
    use rbit_storage::sqlite::actions_artifacts::{NewArtifact, Recorded};
    let f = Fixture::new().await;
    source(&f, "staging").await;
    let admin = f.repo(&f.alice).await;
    let history = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap().remove(0);
    let workflow = WORKFLOW.replace("jobs:\n", "      artifact_id: {type: string}\n      artifact_digest: {type: string}\njobs:\n");
    f.commit(&[(".forgejo/workflows/deploy.yml", &workflow)], "Current artifact deployment workflow");
    f.push_ref("refs/heads/main").await;
    f.actions().put_environment(&f.alice, &admin, "production", &EnvironmentInput::default()).await.unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
    let now = Timestamp::now();
    let recorded = f
        .database
        .actions_artifacts()
        .record(
            &NewArtifact {
                run: history.run_id,
                repo: admin.repo.repo.id,
                attempt: history.run_attempt,
                job: history.job_id,
                name: "dist",
                size: 1,
                sha256: [7; 32],
                now,
                expires_at: Timestamp(now.0 + 3600),
            },
            false,
        )
        .await
        .unwrap();
    let Recorded::Stored { artifact, .. } = recorded else { panic!("new artifact expected") };
    let request = DeploymentRequest {
        environment: "production".into(),
        operation: "promote".into(),
        source_attempt: history.id,
        configuration_id: config.id,
        configuration_generation: config.generation,
        artifact_id: Some(artifact.id.0),
        artifact_digest: Some("07".repeat(32)),
    };
    let wrong = DeploymentRequest { artifact_digest: Some("08".repeat(32)), ..request.clone() };
    assert!(matches!(f.actions().deploy(&f.alice, &admin, wrong).await, Err(ServiceError::Invalid(_))));
    let missing = DeploymentRequest { artifact_digest: None, ..request.clone() };
    assert!(matches!(f.actions().deploy(&f.alice, &admin, missing).await, Err(ServiceError::Invalid(_))));
    let run = f.actions().deploy(&f.alice, &admin, request.clone()).await.unwrap();
    let (_, inputs) = f.database.actions_runs().event(run.id).await.unwrap().unwrap();
    let inputs: serde_json::Value = serde_json::from_str(&inputs.unwrap()).unwrap();
    assert_eq!(inputs["artifact_id"], artifact.id.0.to_string());
    assert_eq!(inputs["artifact_digest"], "07".repeat(32));
    f.database.actions_artifacts().delete(artifact.id).await.unwrap();
    assert!(matches!(f.actions().deploy(&f.alice, &admin, request).await, Err(ServiceError::NotFound)));
}

#[tokio::test]
async fn deployment_concurrency_cannot_cancel_runs_before_authorized_publication() {
    for cancel in [true, false] {
        let f = Fixture::new().await;
        let sha = source(&f, "staging").await;
        let workflow = WORKFLOW.replace("jobs:\n", &format!("concurrency:\n  group: production\n  cancel-in-progress: {cancel}\njobs:\n"));
        f.commit(&[(".forgejo/workflows/deploy.yml", &workflow)], "Current serialized workflow");
        f.push_ref("refs/heads/main").await;
        let admin = f.repo(&f.alice).await;
        f.actions().put_environment(&f.alice, &admin, "production", &EnvironmentInput::default()).await.unwrap();
        let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
        let inputs = BTreeMap::from([
            ("deployment_sha".into(), sha),
            ("deployment_environment".into(), "production".into()),
            ("deployment_operation".into(), "promote".into()),
        ]);
        let existing = f
            .actions()
            .dispatch(&f.alice, &admin, &DispatchRequest { workflow: "deploy.yml".into(), reference: "main".into(), inputs })
            .await
            .unwrap();
        let source = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap()[0].id;
        let count = f.runs().await.len();
        let result = f
            .actions()
            .deploy(
                &f.alice,
                &admin,
                DeploymentRequest {
                    environment: "production".into(),
                    operation: "promote".into(),
                    source_attempt: source,
                    configuration_id: config.id,
                    configuration_generation: config.generation,
                    artifact_id: None,
                    artifact_digest: None,
                },
            )
            .await;
        if cancel {
            assert!(matches!(result, Err(ServiceError::Conflict(_))));
            assert_eq!(f.runs().await.len(), count, "refusal records no new run");
        } else {
            assert_eq!(result.unwrap().status, Status::Blocked, "noncanceling deployments queue behind the active group");
        }
        assert_eq!(
            f.database.actions_runs().run(existing.id).await.unwrap().unwrap().status,
            Status::Waiting,
            "the prior run stays untouched"
        );
    }
}
