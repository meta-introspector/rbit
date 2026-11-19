//! The final SQL writer boundary rejects delegation revoked after captured admission.
use super::{Fixture, prepare};
use rbit_core::{Timestamp, actions::RunEvent};
use rbit_identity_models::agents::Capability;
use rbit_storage::sqlite::{
    actions_deployments::NewIntent,
    actions_runs::{NewJob, NewRun, RunFilter, RunStart},
};
#[tokio::test]
async fn agent_deployment_final_storage_publication_refuses_captured_dispatch_after_revocation() {
    let f = Fixture::new().await;
    let prepared = Box::pin(prepare(&f)).await;
    let mut agent = f.agent(vec![Capability::Read, Capability::Dispatch], false).await;
    let actor = f.db.accounts().find_user(agent.record.account_id).await.unwrap().unwrap();
    let captured = f.services.tokens.authenticate_limited(&agent.secret).await.unwrap().unwrap();
    assert!(captured.2.unwrap().allows(rbit_services::tokens::Right::Dispatch));
    let template = crate::actions_runner_tasks::template("deploy", &["agent-deploy"], &[]);
    let payload = serde_json::to_vec(&template).unwrap();
    let jobs = [NewJob {
        key: "deploy",
        name: "Guarded production",
        matrix: None,
        needs: &[],
        runs_on: &template.runs_on,
        permissions: None,
        payload: &payload,
        condition: false,
        max_parallel: None,
        group: None,
        environment: Some("production"),
    }];
    let new = NewRun {
        repo: f.repo,
        workflow_path: ".forgejo/workflows/deploy.yml",
        workflow_name: "Agent deployment",
        title: "Guarded production",
        event: RunEvent::WorkflowDispatch,
        reference: "refs/heads/main",
        sha: &prepared.sha,
        actor: Some(actor.id()),
        trigger_actor: Some(actor.id()),
        pull_number: None,
        is_fork: false,
        trusted: true,
        concurrency_group: None,
        cancel_in_progress: false,
        event_payload: "{}",
        dispatch_inputs: None,
        start: RunStart::Runnable,
        jobs: &jobs,
        error: None,
        now: Timestamp::now(),
    };
    let intent = NewIntent {
        actor: actor.id(),
        operation: "promote",
        environment: "production",
        source_attempt: prepared.source.id,
        source_sha: &prepared.source.sha,
        artifact: None,
        configuration_id: prepared.configuration.id,
        configuration_generation: prepared.configuration.generation,
        workflow_sha: &prepared.sha,
    };
    let count = f.db.actions_runs().list_runs(f.repo, &RunFilter { limit: 100, ..RunFilter::default() }).await.unwrap().len();
    f.capabilities(&mut agent, vec![Capability::Read]).await;
    let fence = f.db.actions_deployments().publication_fence().await.unwrap();
    assert!(f.db.actions_runs().create_deployment_run(&new, fence, &intent).await.is_err());
    assert_eq!(f.db.actions_runs().list_runs(f.repo, &RunFilter { limit: 100, ..RunFilter::default() }).await.unwrap().len(), count);
    assert!(f.db.actions_deployments().intents(f.repo, 100).await.unwrap().is_empty());
    // Prove the captured candidate itself is valid: restoring delegation admits the identical run+intent.
    f.capabilities(&mut agent, vec![Capability::Read, Capability::Dispatch]).await;
    let fence = f.db.actions_deployments().publication_fence().await.unwrap();
    assert!(f.db.actions_runs().create_deployment_run(&new, fence, &intent).await.is_ok());
    assert_eq!(f.db.actions_runs().list_runs(f.repo, &RunFilter { limit: 100, ..RunFilter::default() }).await.unwrap().len(), count + 1);
}
