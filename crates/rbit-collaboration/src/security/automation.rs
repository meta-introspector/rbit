//! Server-owned scanner channels and explicit consent for package metadata disclosure.
pub mod dependencies;
pub mod secrets;
use serde::{Deserialize, Serialize};

pub const SECRETS: &str = "rbit:secrets:v1";
pub const DEPENDENCIES: &str = "rbit:npm:v1";
pub const FRESH_SECONDS: i64 = 86_400;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(clippy::struct_excessive_bools, reason = "independent scanner, consent and publication configuration switches")]
pub struct AutomationSettings {
    pub secrets: bool,
    pub dependencies: bool,
    /// Explicit permission to send public npm package names and versions to OSV/npm.
    pub package_network_consent: bool,
    pub require_secrets: bool,
    pub require_dependencies: bool,
    pub minimum_severity: u8,
    pub workflow: String,
    pub automatic_pull_requests: bool,
}
impl AutomationSettings {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=4).contains(&self.minimum_severity) {
            return Err("Choose a severity from 1 to 4.");
        }
        if self.dependencies && !self.package_network_consent {
            return Err("Dependency scans require consent to send public package names and versions to OSV and npm.");
        }
        if (self.require_secrets && !self.secrets) || (self.require_dependencies && !self.dependencies) {
            return Err("A required scanner must be enabled.");
        }
        if self.automatic_pull_requests && !self.dependencies {
            return Err("Automatic update pull requests require dependency scanning.");
        }
        if !self.workflow.is_empty() {
            super::Settings { scanner: "automation".into(), workflow: self.workflow.clone(), require_scan: false, minimum_severity: 1 }
                .validate()?;
        }
        Ok(())
    }
    pub fn channels(&self) -> impl Iterator<Item = &'static str> {
        [(self.secrets, SECRETS), (self.dependencies, DEPENDENCIES)].into_iter().filter_map(|(enabled, name)| enabled.then_some(name))
    }
    pub fn requires(&self, channel: &str) -> bool {
        match channel {
            SECRETS => self.require_secrets,
            DEPENDENCIES => self.require_dependencies,
            _ => false,
        }
    }
}
pub fn builtin(scanner: &str) -> bool {
    matches!(scanner, SECRETS | DEPENDENCIES)
}
