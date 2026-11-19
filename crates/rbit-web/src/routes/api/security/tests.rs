//! Security visibility, token ceilings and byte-limit enforcement through the HTTP router.
use super::super::test_support::Fixture;
use rbit_core::{TokenScope, Visibility};
#[tokio::test]
async fn security_api_hides_findings_and_requires_unrestricted_admin_scope() {
    let f = Fixture::new(|_| {}).await;
    let owner = f.user("security-owner").await;
    let stranger = f.user("security-other").await;
    f.repo(&owner, "private", Visibility::Private).await;
    f.repo(&owner, "public", Visibility::Public).await;
    let admin = f.token(&owner, TokenScope::Admin).await;
    let read = f.token(&owner, TokenScope::Read).await;
    let write = f.token(&owner, TokenScope::Write).await;
    let other = f.token(&stranger, TokenScope::Admin).await;
    let settings = "/api/v1/repos/security-owner/private/security/settings";
    let input = serde_json::json!({"scanner":"scanner","workflow":"","require_scan":true,"minimum_severity":3});
    assert_eq!(f.send("GET", settings, Some(&other), None).await.0.status(), 404);
    assert_eq!(f.send("PUT", settings, Some(&read), Some(input.clone())).await.0.status(), 403);
    assert_eq!(f.send("PUT", settings, Some(&write), Some(input.clone())).await.0.status(), 403);
    assert_eq!(f.send("PUT", settings, Some(&admin), Some(input)).await.0.status(), 204);
    assert_eq!(f.send("GET", settings, Some(&admin), None).await.1["scanner"], "scanner");
    assert_eq!(f.send("GET", "/api/v1/repos/security-owner/public/security/scans", None, None).await.0.status(), 404);
    assert_eq!(f.raw("GET", "/security-owner/public/security", None, None, None).await.0.status(), 404);
    let path = "/api/v1/repos/security-owner/private/security/scans";
    assert_eq!(f.send("POST", path, Some(&admin), Some(serde_json::json!({"commit_sha":"main"}))).await.0.status(), 422);
    let too_large = " ".repeat(rbit_services::security::MAX_REPORT_BYTES + 1);
    assert_eq!(
        f.send_text("POST", "/api/v1/repos/security-owner/private/security/scans/1/sarif", Some(&admin), Some(&too_large)).await.0.status(),
        413
    );
}

#[tokio::test]
async fn security_api_finalizes_exact_scan_once_and_audits_triage() {
    let f = Fixture::new(|_| {}).await;
    let owner = f.user("scan-owner").await;
    f.repo(&owner, "demo", Visibility::Private).await;
    let repo = f.services.repositories.open(Some(&owner), "scan-owner", "demo").await.unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&repo.disk_path)
            .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let tree = git(&["hash-object", "-w", "-t", "tree", "--stdin"]);
    let sha = git(&["commit-tree", &tree, "-m", "fixture"]);
    git(&["update-ref", "refs/heads/main", &sha]);
    let token = f.token(&owner, TokenScope::Admin).await;
    let path = "/api/v1/repos/scan-owner/demo/security";
    let config = serde_json::json!({"scanner":"scanner","workflow":"","require_scan":true,"minimum_severity":3});
    assert_eq!(f.send("PUT", &format!("{path}/settings"), Some(&token), Some(config)).await.0.status(), 204);
    let (response, scan) = f.send("POST", &format!("{path}/scans"), Some(&token), Some(serde_json::json!({"commit_sha":sha}))).await;
    assert_eq!(response.status(), 201);
    let id = scan["id"].as_i64().unwrap();
    let target = format!("{path}/scans/{id}/sarif");
    assert_eq!(f.send("POST", &target, Some(&token), Some(serde_json::json!({"version":"2.1.0"}))).await.0.status(), 422);
    let report = serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"scanner"}},
        "invocations":[{"executionSuccessful":true}],"results":[{"ruleId":"unsafe","level":"error",
        "message":{"text":"<unsafe> input"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"README.md"},"region":{"startLine":1}}}]}]}]});
    let (response, result) = f.send("POST", &target, Some(&token), Some(report.clone())).await;
    assert_eq!(response.status(), 200);
    assert_eq!(result["state"], "complete");
    assert_eq!(result["commit_sha"], sha);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(f.send("POST", &target, Some(&token), Some(report)).await.0.status(), 409);
    let findings_path = format!("{path}/scans/{id}/findings");
    let (_, findings) = f.send("GET", &findings_path, Some(&token), None).await;
    assert_eq!(findings["findings"][0]["message"], "<unsafe> input");
    let finding = findings["findings"][0]["id"].as_i64().unwrap();
    let dismissal = format!("{path}/findings/{finding}/dismissal");
    assert_eq!(
        f.send("POST", &dismissal, Some(&token), Some(serde_json::json!({"reason":"accepted_risk","note":""}))).await.0.status(),
        422
    );
    assert_eq!(
        f.send("POST", &dismissal, Some(&token), Some(serde_json::json!({"reason":"accepted_risk","note":"Reviewed boundary"})))
            .await
            .0
            .status(),
        204
    );
    let (_, findings) = f.send("GET", &findings_path, Some(&token), None).await;
    assert_eq!(findings["findings"][0]["reason"], "accepted_risk");
    assert_eq!(findings["findings"][0]["dismissed_by"], owner.id().0);
    f.services.shutdown().await;
}

#[tokio::test]
async fn security_automation_api_consent_scope_and_private_metadata() {
    let f = Fixture::new(|_| {}).await;
    let owner = f.user("auto-owner").await;
    f.repo(&owner, "demo", Visibility::Public).await;
    let admin = f.token(&owner, TokenScope::Admin).await;
    let write = f.token(&owner, TokenScope::Write).await;
    let path = "/api/v1/repos/auto-owner/demo/security/automation";
    let settings = serde_json::json!({"dependencies":true,"minimum_severity":3});
    assert_eq!(f.send("GET", path, None, None).await.0.status(), 404);
    assert_eq!(f.send("PUT", path, Some(&write), Some(settings.clone())).await.0.status(), 403);
    assert_eq!(f.send("PUT", path, Some(&admin), Some(settings)).await.0.status(), 422);
    let settings = serde_json::json!({"secrets":true,"dependencies":true,"package_network_consent":true,"minimum_severity":3});
    assert_eq!(f.send("PUT", path, Some(&admin), Some(settings)).await.0.status(), 204);
    let (response, config) = f.send("GET", path, Some(&admin), None).await;
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(config["settings"]["package_network_consent"], true);
    assert!(!config.to_string().contains("fingerprint_key"));
    assert_eq!(f.send("GET", &format!("{path}/updates"), None, None).await.0.status(), 404);
    f.services.shutdown().await;
}
