//! A bounded SARIF 2.1.0 subset; unsupported external or ambiguous evidence is refused.
use super::{Finding, MAX_RESULTS, Report, label, local_path};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
pub const MAX_REPORT_BYTES: usize = 1024 * 1024;
const INVALID: &str = "Supply bounded SARIF 2.1.0 with one tool run, explicit invocations, results and local locations.";

pub fn parse(bytes: &[u8]) -> Result<Report, &'static str> {
    if bytes.len() > MAX_REPORT_BYTES {
        return Err("SARIF exceeds the 1 MiB limit.");
    }
    let document: Value = serde_json::from_slice(bytes).map_err(|_| INVALID)?;
    if document["version"] != "2.1.0" {
        return Err(INVALID);
    }
    let runs = document["runs"].as_array().filter(|runs| runs.len() == 1).ok_or(INVALID)?;
    let run = &runs[0];
    if run.get("externalPropertyFileReferences").is_some() {
        return Err("External SARIF properties are not supported.");
    }
    let scanner = text(&run["tool"]["driver"]["name"], 100)?;
    let invocations = run["invocations"].as_array().filter(|items| !items.is_empty() && items.len() <= 16).ok_or(INVALID)?;
    let mut complete = true;
    for invocation in invocations {
        if !invocation.is_object() {
            return Err(INVALID);
        }
        complete &= invocation["executionSuccessful"] == true;
        complete &= notifications_complete(invocation, "toolExecutionNotifications")?;
        complete &= notifications_complete(invocation, "toolConfigurationNotifications")?;
    }
    let results = run["results"].as_array().filter(|items| items.len() <= MAX_RESULTS).ok_or(INVALID)?;
    let rules = run["tool"]["driver"].get("rules").map(|rules| rules.as_array().ok_or(INVALID)).transpose()?;
    if rules.is_some_and(|items| items.len() > MAX_RESULTS) {
        return Err(INVALID);
    }
    let mut rule_ids = BTreeSet::new();
    for rule in rules.into_iter().flatten() {
        if !rule_ids.insert(text(&rule["id"], 200)?) {
            return Err(INVALID);
        }
    }
    let mut findings = BTreeMap::new();
    for result in results {
        let finding = result_finding(run, result, rules)?;
        if let Some(previous) = findings.insert(finding.identity.clone(), finding.clone())
            && previous != finding
        {
            return Err("Conflicting results share a fingerprint; supply distinct scanner fingerprints.");
        }
    }
    Ok(Report { scanner, complete, findings: findings.into_values().collect() })
}
fn result_finding(run: &Value, result: &Value, rules: Option<&Vec<Value>>) -> Result<Finding, &'static str> {
    if result.get("kind").is_some_and(|kind| kind != "fail" && kind != "review") {
        return Err("Only fail/review SARIF results are supported.");
    }
    let rule = text(&result["ruleId"], 200)?;
    let metadata = rules.and_then(|rules| rules.iter().find(|entry| entry["id"] == rule));
    let message = result["message"]["text"]
        .as_str()
        .filter(|text| !text.trim().is_empty() && text.len() <= 2000 && !text.chars().any(|ch| ch.is_control() && ch != '\n' && ch != '\t'))
        .ok_or(INVALID)?
        .to_owned();
    let locations = result["locations"].as_array().filter(|items| items.len() == 1).ok_or(INVALID)?;
    let physical = &locations[0]["physicalLocation"];
    let artifact = &physical["artifactLocation"];
    let artifact = if artifact.get("uri").is_some() {
        artifact
    } else {
        let index = artifact["index"].as_u64().and_then(|index| usize::try_from(index).ok()).ok_or(INVALID)?;
        &run["artifacts"].as_array().filter(|items| items.len() <= MAX_RESULTS).and_then(|items| items.get(index)).ok_or(INVALID)?["location"]
    };
    if artifact.get("uriBaseId").is_some_and(|base| base != "%SRCROOT%") {
        return Err("Only repository-root SARIF locations are supported.");
    }
    let path = decode_path(artifact["uri"].as_str().ok_or(INVALID)?)?;
    let line =
        physical["region"]["startLine"].as_u64().and_then(|line| u32::try_from(line).ok()).filter(|line| *line > 0).ok_or(INVALID)?;
    let severity = severity(result, metadata)?;
    let fingerprints = result.get("fingerprints").or_else(|| result.get("partialFingerprints"));
    let context = if let Some(fingerprints) = fingerprints {
        let entries = fingerprints.as_object().filter(|entries| !entries.is_empty() && entries.len() <= 16).ok_or(INVALID)?;
        let mut canonical = BTreeMap::new();
        for (key, value) in entries {
            if !label(key, 100) {
                return Err(INVALID);
            }
            canonical.insert(key, text(value, 200)?);
        }
        serde_json::to_string(&canonical).map_err(|_| INVALID)?
    } else {
        let region = &physical["region"];
        let optional_position = |name| -> Result<u32, &'static str> {
            region
                .get(name)
                .map_or(Ok(0), |value| value.as_u64().and_then(|value| u32::try_from(value).ok()).filter(|value| *value > 0).ok_or(INVALID))
        };
        format!(
            "{line}\0{}\0{}\0{}\0{message}",
            optional_position("startColumn")?,
            optional_position("endLine")?,
            optional_position("endColumn")?
        )
    };
    let identity = format!("{rule}\0{path}\0{context}");
    Ok(Finding { identity, rule, message, path, line, severity })
}
fn severity(result: &Value, rule: Option<&Value>) -> Result<u8, &'static str> {
    if let Some(score) =
        result.pointer("/properties/security-severity").or_else(|| rule.and_then(|rule| rule.pointer("/properties/security-severity")))
    {
        let score = score
            .as_str()
            .and_then(|score| score.parse::<f64>().ok())
            .or_else(|| score.as_f64())
            .filter(|score| score.is_finite() && (0.0..=10.0).contains(score))
            .ok_or(INVALID)?;
        return Ok(if score >= 9.0 {
            4
        } else if score >= 7.0 {
            3
        } else if score >= 4.0 {
            2
        } else {
            1
        });
    }
    let level = result.get("level").or_else(|| rule.and_then(|rule| rule.pointer("/defaultConfiguration/level")));
    let level = level.map_or(Ok("warning"), |level| level.as_str().ok_or(INVALID))?;
    match level {
        "error" => Ok(3),
        "warning" => Ok(2),
        "note" => Ok(1),
        "none" => Ok(0),
        _ => Err(INVALID),
    }
}
fn text(value: &Value, max: usize) -> Result<String, &'static str> {
    value.as_str().filter(|value| label(value, max)).map(str::to_owned).ok_or(INVALID)
}
fn decode_path(uri: &str) -> Result<String, &'static str> {
    if uri.len() > 3072 {
        return Err(INVALID);
    }
    let mut bytes = Vec::new();
    let mut input = uri.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().and_then(|byte| char::from(byte).to_digit(16)).ok_or(INVALID)?;
            let low = input.next().and_then(|byte| char::from(byte).to_digit(16)).ok_or(INVALID)?;
            bytes.push(u8::try_from(high * 16 + low).map_err(|_| INVALID)?);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok().filter(|path| local_path(path)).ok_or("SARIF paths must stay within the repository.")
}

fn notifications_complete(invocation: &Value, key: &str) -> Result<bool, &'static str> {
    let Some(notes) = invocation.get(key) else {
        return Ok(true);
    };
    let notes = notes.as_array().filter(|notes| notes.len() <= MAX_RESULTS).ok_or(INVALID)?;
    let mut complete = true;
    for note in notes {
        if !note.is_object() {
            return Err(INVALID);
        }
        match note.get("level").map_or(Ok("warning"), |level| level.as_str().ok_or(INVALID))? {
            "error" => complete = false,
            "warning" | "note" | "none" => {}
            _ => return Err(INVALID),
        }
    }
    Ok(complete)
}
