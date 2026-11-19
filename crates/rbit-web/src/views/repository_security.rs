//! Security findings cards with escaped, immutable commit and file links.
use super::{layout::Page, repo::RepoHeader};
use askama::Template;
use askama_web::WebTemplate;
use rbit_localization::RepoBrowseLabel;
use rbit_services::security::{Occurrence, Scan, Settings};
#[derive(Debug, Template, WebTemplate)]
#[template(path = "security/page.html")]
pub struct SecurityPage {
    pub page: Page,
    pub header: RepoHeader,
    pub config: Settings,
    pub automatic: rbit_services::security::automation::AutomationSettings,
    pub tip: Option<String>,
    pub packages: Vec<rbit_services::security::automation::Package>,
    pub updates: Vec<rbit_services::security::automation::Remediation>,
    pub scans: Vec<Scan>,
    pub selected: Option<Scan>,
    pub findings: Vec<Occurrence>,
    pub historical: bool,
    pub stale: bool,
    pub offset: u32,
    pub next: bool,
}
impl SecurityPage {
    pub fn severity(&self, value: impl std::borrow::Borrow<u8>) -> RepoBrowseLabel {
        match *value.borrow() {
            4 => RepoBrowseLabel::SecurityCritical,
            3 => RepoBrowseLabel::SecurityHigh,
            2 => RepoBrowseLabel::SecurityMedium,
            1 => RepoBrowseLabel::SecurityLow,
            _ => RepoBrowseLabel::SecurityInfo,
        }
    }
    pub fn state(&self, value: &str) -> RepoBrowseLabel {
        match value {
            "complete" => RepoBrowseLabel::SecurityComplete,
            "incomplete" => RepoBrowseLabel::SecurityIncomplete,
            _ => RepoBrowseLabel::SecurityPending,
        }
    }
    pub fn reason(&self, value: &str) -> RepoBrowseLabel {
        match value {
            "false_positive" => RepoBrowseLabel::SecurityFalsePositive,
            "used_in_tests" => RepoBrowseLabel::SecurityTestCode,
            _ => RepoBrowseLabel::SecurityAcceptedRisk,
        }
    }
    pub fn file_url(&self, scan: &Scan, finding: &Occurrence) -> String {
        format!("{}/blob/{}/{}#L{}", self.header.home_url, scan.commit_sha, crate::urls::encode_path(&finding.path), finding.line)
    }
    pub fn next_offset(&self) -> u32 {
        self.offset.saturating_add(50)
    }
    pub fn previous_offset(&self) -> u32 {
        self.offset.saturating_sub(50)
    }
}

#[cfg(test)]
mod tests;
