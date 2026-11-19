//! Security scans bind exact revisions, provenance and pending evidence across merge paths.
use super::*;
use rbit_core::Timestamp;
use rbit_services::security::{Settings, Uploader};
fn report(with_finding: bool, complete: bool) -> Vec<u8> {
    let findings = if with_finding {
        serde_json::json!([{"ruleId":"unsafe","message":{"text":"Unsafe input"},"partialFingerprints":{"stable":"first"},"level":"error","locations":[{"physicalLocation":{"artifactLocation":{"uri":"README.md"},"region":{"startLine":1}}}]}])
    } else {
        serde_json::json!([])
    };
    serde_json::to_vec(&serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"scanner"}},"invocations":[{"executionSuccessful":complete}],"results":findings}]})).unwrap()
}
async fn upload(f: &Fixture, sha: &str, finding: bool, complete: bool) -> i64 {
    let repo = f.repo(&f.alice, "alice").await;
    let scan = f.services.security.begin(Uploader::User(&f.alice), &repo, sha).await.unwrap();
    f.services.security.finish(Uploader::User(&f.alice), &repo, scan.id, report(finding, complete)).await.unwrap();
    scan.id
}
async fn configure(f: &Fixture) {
    let repo = f.repo(&f.alice, "alice").await;
    f.services
        .security
        .configure(
            &f.alice,
            &repo,
            Settings { scanner: "scanner".into(), workflow: String::new(), require_scan: true, minimum_severity: 3 },
        )
        .await
        .unwrap();
}
async fn gate(f: &Fixture, number: u32) -> rbit_services::security::Gate {
    let ctx = f.open_pull(&f.alice, number).await;
    f.services
        .pulls
        .merge_readiness(Some(&f.alice), &ctx, None)
        .await
        .unwrap()
        .evaluation
        .checks
        .into_iter()
        .find_map(|check| match check.requirement {
            rbit_core::pull::Requirement::Security(gate) => Some(gate),
            _ => None,
        })
        .unwrap()
}
#[tokio::test]
async fn security_gate_tracks_exact_head_base_pending_new_findings_and_audited_dismissal() {
    use rbit_services::security::Gate;
    let f = Fixture::new().await;
    configure(&f).await;
    let number = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, number).await;
    let repo = ctx.repo();
    let head = &ctx.pull.head_sha;
    let base = repo.reader.commit_id("main".into()).await.unwrap();
    assert_eq!(gate(&f, number).await, Gate::Missing);
    assert!(f.services.pulls.enqueue(&f.alice, &ctx, MergeStrategy::Merge, None).await.is_err());
    let mut bypass = Fixture::merge_input(MergeStrategy::Merge);
    bypass.bypass = true;
    assert!(f.services.pulls.merge(&f.alice, &ctx, bypass).await.is_err());
    upload(&f, &base, false, true).await;
    let head_scan = upload(&f, head, true, true).await;
    assert_eq!(gate(&f, number).await, Gate::Findings(1));
    let (_, findings) = f.services.security.findings(repo, head_scan, 0).await.unwrap();
    let id = findings[0].id;
    f.services.security.dismiss(&f.alice, repo, id, "accepted_risk", "Reviewed mitigation").await.unwrap();
    assert_eq!(gate(&f, number).await, Gate::Passed);
    f.services.security.dismiss(&f.alice, repo, id, "reopen", "").await.unwrap();
    assert_eq!(gate(&f, number).await, Gate::Findings(1));
    let previous = f.services.security.begin(Uploader::User(&f.alice), repo, head).await.unwrap();
    let newest = f.services.security.begin(Uploader::User(&f.alice), repo, head).await.unwrap();
    f.services.security.finish(Uploader::User(&f.alice), repo, previous.id, report(false, true)).await.unwrap();
    assert_eq!(gate(&f, number).await, Gate::Incomplete, "older clean completion cannot supersede newer pending scan");
    f.services.security.finish(Uploader::User(&f.alice), repo, newest.id, report(false, false)).await.unwrap();
    assert_eq!(gate(&f, number).await, Gate::Incomplete);
    assert!(
        f.services.security.finish(Uploader::User(&f.alice), repo, newest.id, report(false, true)).await.is_err(),
        "completed scans cannot replay"
    );
    upload(&f, head, false, true).await;
    assert_eq!(gate(&f, number).await, Gate::Passed);
    configure(&f).await;
    assert_eq!(gate(&f, number).await, Gate::Missing, "changing scanner policy invalidates old evidence");
    upload(&f, &base, false, true).await;
    upload(&f, head, false, true).await;
    let commit = f.services.pulls.merge(&f.alice, &ctx, Fixture::merge_input(MergeStrategy::Merge)).await.unwrap();
    assert_eq!(
        repo.reader.commit_id("refs/heads/main".into()).await.unwrap(),
        commit,
        "complete exact-revision evidence permits the guarded native publication"
    );
    f.services.shutdown().await;
}
#[tokio::test]
async fn security_findings_are_hidden_from_readers_and_current_authority_is_required() {
    let f = Fixture::new().await;
    configure(&f).await;
    let owner = f.repo(&f.alice, "alice").await;
    let public = f.services.repositories.open(None, "alice", "demo").await.unwrap();
    assert!(matches!(f.services.security.scans(&public).await, Err(ServiceError::NotFound)));
    let number = f.create(&f.alice, "alice", "feature").await.unwrap();
    let public_pull = f.services.pulls.open(None, public, IssueNumber::new(number).unwrap()).await.unwrap();
    let readiness = f.services.pulls.merge_readiness(None, &public_pull, None).await.unwrap();
    assert!(
        !readiness.evaluation.checks.iter().any(|check| matches!(check.requirement, rbit_core::pull::Requirement::Security(_))),
        "public merge boxes cannot disclose private finding counts or evidence state"
    );
    let writer = f.repo(&f.carol, "alice").await;
    assert!(f.services.security.scans(&writer).await.is_ok());
    let sha = owner.reader.commit_id("main".into()).await.unwrap();
    assert!(f.services.security.begin(Uploader::User(&f.carol), &writer, &sha).await.is_err());
    let scan = f.services.security.begin(Uploader::User(&f.alice), &owner, &sha).await.unwrap();
    assert!(f.services.security.finish(Uploader::User(&f.carol), &writer, scan.id, report(false, true)).await.is_err());
    assert!(f.services.security.begin(Uploader::User(&f.alice), &owner, "main").await.is_err());
    f.services.shutdown().await;
}

#[tokio::test]
async fn security_publication_fence_serializes_scan_start_and_configuration_changes() {
    let f = Fixture::new().await;
    configure(&f).await;
    let repo = f.repo(&f.alice, "alice").await;
    let sha = repo.reader.commit_id("main".into()).await.unwrap();
    let store = f.database.security();
    let fence = store.publication_fence().await.unwrap();
    let start = f.services.security.begin(Uploader::User(&f.alice), &repo, &sha);
    tokio::pin!(start);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut start).await.is_err(),
        "a new pending scan cannot invalidate evidence while native publication holds its fence"
    );
    assert!(store.scans(repo.repo.repo.id).await.unwrap().is_empty());
    drop(fence);
    start.await.unwrap();
    let fence = store.publication_fence().await.unwrap();
    let setting = Settings { scanner: "replacement".into(), workflow: String::new(), require_scan: true, minimum_severity: 4 };
    let change = f.services.security.configure(&f.alice, &repo, setting);
    tokio::pin!(change);
    assert!(tokio::time::timeout(std::time::Duration::from_millis(30), &mut change).await.is_err());
    assert_eq!(store.configuration(repo.repo.repo.id).await.unwrap().unwrap().scanner, "scanner");
    drop(fence);
    change.await.unwrap();
    f.services.shutdown().await;
}

#[tokio::test]
async fn security_automation_required_channels_cannot_be_replaced_by_clean_external_reports() {
    use rbit_services::security::{Gate, automation::AutomationSettings};
    let f = Fixture::new().await;
    git(&f.work, &["checkout", "-q", "feature"]);
    commit(&f.work, "credential.txt", &format!("ghp_{}\n", "x".repeat(36)), "Credential fixture");
    f.push(&f.alice, "alice", &["feature"]).await;
    configure(&f).await;
    let number = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, number).await;
    let repo = ctx.repo();
    let base = repo.reader.commit_id("main".into()).await.unwrap();
    f.services
        .security
        .configure_automation(
            &f.alice,
            repo,
            AutomationSettings {
                secrets: true,
                dependencies: true,
                package_network_consent: true,
                require_secrets: true,
                require_dependencies: true,
                minimum_severity: 3,
                ..AutomationSettings::default()
            },
        )
        .await
        .unwrap();
    upload(&f, &base, false, true).await;
    upload(&f, &ctx.pull.head_sha, false, true).await;
    assert_eq!(gate(&f, number).await, Gate::Missing);
    for sha in [&base, &ctx.pull.head_sha] {
        f.services.security.request_automatic_scan(Uploader::User(&f.alice), repo, sha).await.unwrap();
    }
    for _ in 0..4 {
        assert!(f.services.security.run_one_automatic_scan().await.unwrap());
    }
    assert_eq!(gate(&f, number).await, Gate::Findings(1));
    upload(&f, &ctx.pull.head_sha, false, true).await;
    assert_eq!(gate(&f, number).await, Gate::Findings(1), "external clean evidence cannot replace secret evidence");
    let config = f.database.security().automation(repo.repo.repo.id).await.unwrap().unwrap();
    let id = f
        .database
        .security()
        .publication_fence()
        .await
        .unwrap()
        .enqueue_scan(
            repo.repo.repo.id,
            rbit_collaboration::security::automation::SECRETS,
            &ctx.pull.head_sha,
            config.generation,
            Timestamp(1),
        )
        .await
        .unwrap();
    let scan = f.database.security().scan(repo.repo.repo.id, id).await.unwrap().unwrap();
    f.database
        .security()
        .publication_fence()
        .await
        .unwrap()
        .finish_scan(
            repo.repo.repo.id,
            &scan,
            &rbit_collaboration::security::Report { scanner: scan.scanner.clone(), complete: true, findings: Vec::new() },
        )
        .await
        .unwrap();
    assert_eq!(gate(&f, number).await, Gate::Incomplete, "expired native evidence cannot pass even for an unchanged commit");
    f.services.shutdown().await;
}
