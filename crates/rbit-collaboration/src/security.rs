//! Static-analysis report contracts and fail-closed security merge decisions.
pub mod automation;
mod sarif;
pub use sarif::{MAX_REPORT_BYTES, parse};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_RESULTS: usize = 1000;
pub const MAX_SCANS: i64 = 100;
pub const MAX_FINDINGS: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub identity: String,
    pub rule: String,
    pub message: String,
    pub path: String,
    pub line: u32,
    pub severity: u8,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub scanner: String,
    pub complete: bool,
    pub findings: Vec<Finding>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub scanner: String,
    pub workflow: String,
    pub require_scan: bool,
    pub minimum_severity: u8,
}
impl Settings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.scanner.starts_with("rbit:") || !label(&self.scanner, 100) || !(1..=4).contains(&self.minimum_severity) {
            return Err("Supply a scanner name and a severity from 1 (low) to 4 (critical).");
        }
        if !self.workflow.is_empty()
            && (!local_path(&self.workflow)
                || ![".github/workflows/", ".forgejo/workflows/", ".rbit/workflows/"].iter().any(|p| self.workflow.starts_with(p)))
        {
            return Err("The trusted workflow must be a repository-relative workflow path.");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    Disabled,
    Missing,
    Incomplete,
    Findings(usize),
    Passed,
}
impl Gate {
    pub fn met(self) -> bool {
        matches!(self, Self::Disabled | Self::Passed)
    }
}
/// Both snapshots must be the latest started scan for their exact immutable revision.
pub fn evaluate(settings: &Settings, head: Option<(bool, &[(String, u8)])>, base: Option<(bool, &[String])>, dismissed: &[String]) -> Gate {
    if !settings.require_scan {
        return Gate::Disabled;
    }
    let (Some((head_complete, head)), Some((base_complete, base))) = (head, base) else { return Gate::Missing };
    if !head_complete || !base_complete {
        return Gate::Incomplete;
    }
    let previous: HashSet<_> = base.iter().chain(dismissed).collect();
    let introduced = head.iter().filter(|(key, severity)| *severity >= settings.minimum_severity && !previous.contains(key)).count();
    if introduced == 0 { Gate::Passed } else { Gate::Findings(introduced) }
}
pub fn label(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
pub fn local_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains(['\\', ':', '?', '#'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}
pub fn dismissal(reason: &str, note: &str) -> Result<(), &'static str> {
    if !["false_positive", "accepted_risk", "used_in_tests"].contains(&reason) || !label(note, 1000) {
        return Err("Choose a dismissal reason and supply a note of 1–1000 bytes.");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
