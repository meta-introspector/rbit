//! Scanner-controlled text stays escaped and file links stay pinned to immutable snapshots.
use super::*;
#[test]
fn security_cards_escape_content_and_localize_controls() {
    let scan = Scan {
        id: 1,
        channel: "external".into(),
        scanner: "scanner".into(),
        commit_sha: "a".repeat(40),
        generation: 1,
        source: "user:1".into(),
        state: "incomplete".into(),
        created_at: 1,
        finished_at: Some(2),
    };
    let finding = Occurrence {
        id: 1,
        fingerprint: "b".repeat(64),
        rule: "<script>".into(),
        message: "<img src=x onerror=alert(1)>".into(),
        path: "src/a b.rs".into(),
        line: 12,
        severity: 4,
        reason: Some("accepted_risk".into()),
        note: Some("<script>note</script>".into()),
        dismissed_by: Some(1),
        dismissed_at: Some(2),
    };
    let page = SecurityPage {
        page: crate::views::test_support::page("/owner/repo/security").with_locale(rbit_localization::Locale::Spanish),
        header: RepoHeader { home_url: "/owner/repo".into(), can_admin: true, ..RepoHeader::default() },
        config: Settings::default(),
        automatic: rbit_services::security::automation::AutomationSettings::default(),
        tip: None,
        packages: Vec::new(),
        updates: Vec::new(),
        scans: vec![scan.clone()],
        selected: Some(scan),
        findings: vec![finding],
        historical: true,
        stale: true,
        offset: 0,
        next: false,
    };
    let html = page.render().unwrap();
    assert!(!html.contains("<script>note") && !html.contains("<img src=x"));
    assert!(html.contains("&#60;img src=x onerror=alert(1)&#62;"));
    assert!(html.contains(&format!("/owner/repo/blob/{}/src/a%20b.rs#L12", "a".repeat(40))));
    assert!(html.contains("Seguridad") && !html.contains("Reopen finding"));
    assert!(html.contains("name=\"reason\" value=\"reopen\""));
}
