//! Reusable workflow provenance and deferred-plan refusal keep all executable definitions current.
use super::*;

#[tokio::test]
async fn deployment_reusable_workflows_must_share_the_current_own_commit() {
    let f = Fixture::new().await;
    source(&f, "staging").await;
    let called = "name: Called deploy\non:\n  workflow_call:\n    inputs:\n      environment: {type: string, required: true}\njobs:\n  deploy:\n    runs-on: x\n    environment: ${{ inputs.environment }}\n    steps: [{run: echo current}]\n";
    let prefix = WORKFLOW.split_once("jobs:\n").unwrap().0;
    let local = format!(
        "{prefix}jobs:\n  call:\n    uses: ./.forgejo/workflows/called.yml\n    with:\n      environment: ${{{{ inputs.deployment_environment }}}}\n"
    );
    let trusted = f.commit(
        &[(".forgejo/workflows/deploy.yml", &local), (".forgejo/workflows/called.yml", called)],
        "Current local reusable deployment",
    );
    f.push_ref("refs/heads/main").await;
    let admin = f.repo(&f.alice).await;
    f.actions().put_environment(&f.alice, &admin, "production", &EnvironmentInput::default()).await.unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
    let source = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap()[0].id;
    let request = DeploymentRequest {
        environment: "production".into(),
        operation: "promote".into(),
        source_attempt: source,
        configuration_id: config.id,
        configuration_generation: config.generation,
        artifact_id: None,
        artifact_digest: None,
    };
    assert!(f.actions().deploy(&f.alice, &admin, request.clone()).await.is_ok(), "same current commit local calls are trusted");
    let historic = local.replace("./.forgejo/workflows/called.yml", &format!("alice/rocket/.forgejo/workflows/called.yml@{trusted}"));
    f.commit(&[(".forgejo/workflows/deploy.yml", &historic)], "Reference historical reusable workflow");
    f.push_ref("refs/heads/main").await;
    let Err(ServiceError::Invalid(message)) = f.actions().deploy(&f.alice, &admin, request).await else {
        panic!("historical called workflow must be refused")
    };
    assert!(message.contains("current default-branch commit"), "{message}");
}

#[tokio::test]
async fn deployment_needs_deferred_plans_refuse_before_any_environment_job_runs() {
    let f = Fixture::new().await;
    source(&f, "staging").await;
    let prefix = WORKFLOW.split_once("jobs:\n").unwrap().0;
    let workflow = format!(
        "{prefix}jobs:\n  gate:\n    runs-on: x\n    environment: ${{{{ inputs.deployment_environment }}}}\n    outputs:\n      runner: x\n    steps: [{{run: echo current}}]\n  deferred:\n    needs: gate\n    runs-on: ${{{{ needs.gate.outputs.runner }}}}\n    steps: [{{run: echo later}}]\n"
    );
    f.commit(&[(".forgejo/workflows/deploy.yml", &workflow)], "Needs dependent deployment plan");
    f.push_ref("refs/heads/main").await;
    let admin = f.repo(&f.alice).await;
    f.actions().put_environment(&f.alice, &admin, "production", &EnvironmentInput::default()).await.unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
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
    assert!(matches!(result, Err(ServiceError::Invalid(_))));
    assert_eq!(f.runs().await.len(), count, "no partially published target run");
}
