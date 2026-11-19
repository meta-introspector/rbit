//! Hostile SARIF, stable identity and fail-closed exact-snapshot policy regressions.
use super::*;
use serde_json::{Value, json};
fn report() -> Value {
    json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"scanner","rules":[{"id":"rule","properties":{"security-severity":"9.2"}}]}},"invocations":[{"executionSuccessful":true}],"results":[{"ruleId":"rule","message":{"text":"unsafe input"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/main.rs"},"region":{"startLine":4}}}]}]}]})
}
fn parsed(value: &Value) -> Result<Report, &'static str> {
    parse(&serde_json::to_vec(value).unwrap())
}
#[test]
fn security_sarif_normalizes_severity_paths_and_deduplicates_identical_results() {
    let mut value = report();
    let result = value["runs"][0]["results"][0].clone();
    value["runs"][0]["results"].as_array_mut().unwrap().push(result);
    let parsed = parsed(&value).unwrap();
    assert!(parsed.complete);
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(parsed.findings[0].severity, 4);
    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"] = "src/a%20b.rs".into();
    assert_eq!(super::parse(&serde_json::to_vec(&value).unwrap()).unwrap().findings[0].path, "src/a b.rs");
}
#[test]
fn security_scan_omitted_or_failed_evidence_never_counts_complete() {
    for field in ["invocations", "results"] {
        let mut value = report();
        value["runs"][0].as_object_mut().unwrap().remove(field);
        assert!(parsed(&value).is_err());
    }
    let mut value = report();
    value["runs"][0]["invocations"][0]["executionSuccessful"] = false.into();
    assert!(!parsed(&value).unwrap().complete);
    value["runs"][0]["invocations"][0]["executionSuccessful"] = true.into();
    value["runs"][0]["invocations"][0]["toolExecutionNotifications"] = json!([{"level":"error"}]);
    assert!(!parsed(&value).unwrap().complete);
    value["runs"][0]["invocations"] = json!([{}]);
    assert!(!parsed(&value).unwrap().complete);
}
#[test]
fn security_sarif_refuses_external_traversing_and_overbudget_inputs() {
    for path in ["../secret", "/etc/passwd", "file:///tmp/a", "https://host/x", "a\\b", "%2e%2e/secret", "a/%00b", "C:/a"] {
        let mut value = report();
        value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"] = path.into();
        assert!(parsed(&value).is_err(), "{path}");
    }
    let mut value = report();
    value["runs"][0]["externalPropertyFileReferences"] = json!({"results":[{"location":{"uri":"https://host/report"}}]});
    assert!(parsed(&value).is_err());
    assert!(parse(&vec![b' '; MAX_REPORT_BYTES + 1]).is_err());
    let mut value = report();
    value["runs"][0]["results"] = json!(vec![value["runs"][0]["results"][0].clone(); MAX_RESULTS + 1]);
    assert!(parsed(&value).is_err());
}
#[test]
fn security_distinct_locations_stay_distinct_and_scanner_fingerprints_survive_line_motion() {
    let mut value = report();
    let first = parsed(&value).unwrap().findings[0].identity.clone();
    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"]["startColumn"] = 2.into();
    assert_ne!(parsed(&value).unwrap().findings[0].identity, first, "same-line occurrences retain their distinct column context");
    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"].as_object_mut().unwrap().remove("startColumn");
    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"] = 8.into();
    assert_ne!(parsed(&value).unwrap().findings[0].identity, first);
    value["runs"][0]["results"][0]["partialFingerprints"] = json!({"primaryLocationLineHash":"abc"});
    let stable = parsed(&value).unwrap().findings[0].identity.clone();
    value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"] = 20.into();
    assert_eq!(parsed(&value).unwrap().findings[0].identity, stable);
}
#[test]
fn security_gate_requires_complete_both_sides_and_only_new_undismissed_severity_blocks() {
    let settings = Settings { scanner: "scanner".into(), require_scan: true, minimum_severity: 3, ..Settings::default() };
    assert_eq!(evaluate(&settings, None, None, &[]), Gate::Missing);
    let head = vec![("old".into(), 4), ("new".into(), 3), ("minor".into(), 1)];
    let base = vec!["old".into()];
    assert_eq!(evaluate(&settings, Some((false, &head)), Some((true, &base)), &[]), Gate::Incomplete);
    assert_eq!(evaluate(&settings, Some((true, &head)), Some((true, &base)), &[]), Gate::Findings(1));
    assert_eq!(evaluate(&settings, Some((true, &head)), Some((true, &base)), &["new".into()]), Gate::Passed);
    assert!(!crate::pulls::Requirement::Security(Gate::Missing).is_protection(), "branch bypass cannot bypass security evidence");
}

#[test]
fn security_malformed_notifications_and_ambiguous_rule_metadata_cannot_produce_clean_evidence() {
    for notes in [json!({"level":"error"}), json!([null]), json!([{"level":true}])] {
        let mut value = report();
        value["runs"][0]["invocations"][0]["toolExecutionNotifications"] = notes;
        assert!(parsed(&value).is_err());
    }
    let mut value = report();
    value["runs"][0]["tool"]["driver"]["rules"] = json!({"id":"rule"});
    assert!(parsed(&value).is_err());
    value["runs"][0]["tool"]["driver"]["rules"] = json!([{"id":"rule"}, {"id":"rule"}]);
    assert!(parsed(&value).is_err());
    let mut value = report();
    value["runs"][0]["results"][0]["partialFingerprints"] = json!({"same":"fingerprint"});
    let mut conflicting = value["runs"][0]["results"][0].clone();
    conflicting["locations"][0]["physicalLocation"]["region"]["startLine"] = 9.into();
    value["runs"][0]["results"].as_array_mut().unwrap().push(conflicting);
    assert!(parsed(&value).is_err(), "ambiguous scanner identities cannot silently collapse two occurrences");
}
