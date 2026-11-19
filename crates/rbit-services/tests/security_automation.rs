//! Native scan consent, durable claims and redaction against real immutable Git snapshots.
use crate::actions_fixture::Fixture;
use rbit_core::Timestamp;
use rbit_services::security::{Uploader, automation::AutomationSettings};
fn settings() -> AutomationSettings {
    AutomationSettings { secrets: true, require_secrets: true, minimum_severity: 3, ..AutomationSettings::default() }
}

#[tokio::test]
async fn security_automation_scans_without_workflow_execution_and_redacts_credentials() {
    let f = Fixture::new().await;
    let token = format!("ghp_{}", "x".repeat(36));
    let sha = f.commit(&[("config.txt", &format!("setting=public\n{token}\n"))], "Secret fixture");
    f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.alice).await;
    f.services.security.configure_automation(&f.alice, &repo, settings()).await.unwrap();
    let ids = f.services.security.request_automatic_scan(Uploader::User(&f.alice), &repo, &sha).await.unwrap();
    assert_eq!(
        ids,
        f.services.security.request_automatic_scan(Uploader::User(&f.alice), &repo, &sha).await.unwrap(),
        "duplicate admission shares pending work"
    );
    assert!(f.services.security.run_one_automatic_scan().await.unwrap());
    let (scan, findings) = f.services.security.findings(&repo, ids[0], 0).await.unwrap();
    assert_eq!(scan.channel, "secrets");
    assert_eq!(scan.state, "complete");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 2);
    assert_eq!(findings[0].fingerprint.len(), 64);
    assert!(!serde_json::to_string(&findings).unwrap().contains(&token));
    assert!(!f.services.security.run_one_automatic_scan().await.unwrap());
    let config = f.services.security.automation_configuration(&repo).await.unwrap().unwrap();
    assert!(
        f.database.security().latest(repo.repo.repo.id, &scan.scanner, &sha, config.generation).await.unwrap().is_none(),
        "external channel cannot read native evidence"
    );
    f.services.shutdown().await;
}
#[tokio::test]
async fn security_automation_requires_explicit_network_consent_and_recovers_expired_claims() {
    let f = Fixture::new().await;
    let sha = f.commit(&[("README.md", "safe")], "Base");
    f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.alice).await;
    let mut config = settings();
    config.dependencies = true;
    assert!(f.services.security.configure_automation(&f.alice, &repo, config).await.is_err());
    assert!(f.services.security.automation_configuration(&repo).await.unwrap().is_none());
    f.services.security.configure_automation(&f.alice, &repo, settings()).await.unwrap();
    let id = f.services.security.request_automatic_scan(Uploader::User(&f.alice), &repo, &sha).await.unwrap()[0];
    let store = f.database.security();
    let first = store.claim_scan(Timestamp(10)).await.unwrap().unwrap();
    assert_eq!(first.scan_id, id);
    assert_eq!(first.attempts, 1);
    assert!(store.claim_scan(Timestamp(11)).await.unwrap().is_none());
    let recovered = store.claim_scan(Timestamp(310)).await.unwrap().unwrap();
    assert_eq!(recovered.scan_id, id);
    assert_eq!(recovered.attempts, 2);
    store.abandon_scan(id, Timestamp(311)).await.unwrap();
    assert_eq!(store.scan(repo.repo.repo.id, id).await.unwrap().unwrap().state, "incomplete");
    assert!(store.claim_scan(Timestamp(1000)).await.unwrap().is_none());
    f.services.shutdown().await;
}
#[tokio::test]
async fn security_automation_configuration_change_invalidates_queued_work() {
    let f = Fixture::new().await;
    let sha = f.commit(&[("README.md", "safe")], "Base");
    f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.alice).await;
    f.services.security.configure_automation(&f.alice, &repo, settings()).await.unwrap();
    let id = f.services.security.request_automatic_scan(Uploader::User(&f.alice), &repo, &sha).await.unwrap()[0];
    f.services
        .security
        .configure_automation(&f.alice, &repo, AutomationSettings { minimum_severity: 3, ..AutomationSettings::default() })
        .await
        .unwrap();
    assert!(f.services.security.run_one_automatic_scan().await.unwrap());
    assert_eq!(f.database.security().scan(repo.repo.repo.id, id).await.unwrap().unwrap().state, "incomplete");
    f.services.shutdown().await;
}
