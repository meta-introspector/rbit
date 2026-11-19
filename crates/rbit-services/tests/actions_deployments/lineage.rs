//! Redeployments retain logical source and original artifact provenance independently of workflow revisions.
use super::*;
use rbit_storage::sqlite::actions_artifacts::{NewArtifact, Recorded};

async fn finish(f: &Fixture, seed: u8) {
    let claimed = f.claim(&["x"], seed).await.unwrap();
    f.database.actions_runs().finish_task(claimed.task.id, Status::Success, Timestamp::now()).await.unwrap();
}

#[tokio::test]
async fn rollback_reuses_logical_source_and_origin_artifact_after_redeployment_and_retention() {
    let f = Fixture::new().await;
    let source_sha = source(&f, "staging").await;
    let admin = f.repo(&f.alice).await;
    let origin = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap().remove(0);
    let now = Timestamp::now();
    let recorded = f
        .database
        .actions_artifacts()
        .record(
            &NewArtifact {
                run: origin.run_id,
                repo: admin.repo.repo.id,
                attempt: origin.run_attempt,
                job: origin.job_id,
                name: "dist",
                size: 1,
                sha256: [9; 32],
                now,
                expires_at: Timestamp(now.0 + 3600),
            },
            false,
        )
        .await
        .unwrap();
    let Recorded::Stored { artifact, .. } = recorded else { panic!("artifact expected") };
    let digest = "09".repeat(32);
    let workflow = WORKFLOW.replace("jobs:\n", "      artifact_id: {type: string}\n      artifact_digest: {type: string}\njobs:\n");
    let workflow_sha = f.commit(&[(".forgejo/workflows/deploy.yml", &workflow)], "Advance trusted workflow independently of source");
    assert_ne!(workflow_sha, source_sha);
    f.push_ref("refs/heads/main").await;
    f.actions().put_environment(&f.alice, &admin, "production", &EnvironmentInput::default()).await.unwrap();
    let config = f.actions().configure_deployment_workflow(&f.alice, &admin, "production", "staging", "deploy.yml").await.unwrap();
    let promote = DeploymentRequest {
        environment: "production".into(),
        operation: "promote".into(),
        source_attempt: origin.id,
        configuration_id: config.id,
        configuration_generation: config.generation,
        artifact_id: Some(artifact.id.0),
        artifact_digest: Some(digest.clone()),
    };
    f.actions().deploy(&f.alice, &admin, promote.clone()).await.unwrap();
    finish(&f, 90).await;
    let produced = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap().remove(0);
    assert_eq!((&produced.sha, &produced.workflow_sha), (&source_sha, &workflow_sha));
    assert_eq!(produced.origin_attempt_id, Some(origin.id));
    assert_eq!((produced.artifact_id, produced.artifact_digest.as_deref()), (Some(artifact.id.0), Some(digest.as_str())));
    let rollback = DeploymentRequest { operation: "rollback".into(), source_attempt: produced.id, ..promote };
    let live = f.actions().deploy(&f.alice, &admin, rollback.clone()).await.unwrap();
    let (_, inputs) = f.database.actions_runs().event(live.id).await.unwrap().unwrap();
    let inputs: serde_json::Value = serde_json::from_str(&inputs.unwrap()).unwrap();
    assert_eq!(inputs["deployment_sha"], source_sha);
    assert_eq!(inputs["artifact_id"], artifact.id.0.to_string());
    finish(&f, 91).await;
    f.database.actions_runs().delete_finished_before(Timestamp(Timestamp::now().0 + 2)).await.unwrap();
    let retained = f.database.actions_deployments().attempt(admin.repo.repo.id, produced.id).await.unwrap().unwrap();
    assert!(!retained.run_available);
    assert_eq!((&retained.sha, &retained.workflow_sha), (&source_sha, &workflow_sha));
    assert_eq!(retained.artifact_id, Some(artifact.id.0));
    assert!(
        matches!(f.actions().deploy(&f.alice, &admin, rollback.clone()).await, Err(ServiceError::NotFound)),
        "missing original archive refuses explicit execution"
    );
    let sha_only = DeploymentRequest { artifact_id: None, artifact_digest: None, ..rollback };
    let next = f.actions().deploy(&f.alice, &admin, sha_only).await.unwrap();
    let (_, inputs) = f.database.actions_runs().event(next.id).await.unwrap().unwrap();
    let inputs: serde_json::Value = serde_json::from_str(&inputs.unwrap()).unwrap();
    assert_eq!(inputs["deployment_sha"], source_sha);
    let intents = f.database.actions_deployments().intents(admin.repo.repo.id, 10).await.unwrap();
    assert_eq!(intents[0].origin_attempt_id, origin.id);
    assert_eq!(intents[0].origin_artifact_id, Some(artifact.id.0));
    assert!(intents[0].artifact_id.is_none(), "retained provenance does not claim unavailable bytes were selected");
    finish(&f, 92).await;
    let next = f.database.actions_deployments().attempts(admin.repo.repo.id, None, 100).await.unwrap().remove(0);
    assert_eq!((&next.sha, &next.workflow_sha), (&source_sha, &workflow_sha));
    assert_eq!(next.artifact_id, Some(artifact.id.0));
}
