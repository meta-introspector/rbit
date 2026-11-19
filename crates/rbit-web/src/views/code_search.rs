//! Commit-pinned code search result presentation.
use super::{layout::Page, repo::RepoHeader};
use askama::Template;
use askama_web::WebTemplate;
#[derive(Debug)]
pub struct Match {
    pub path: String,
    pub url: String,
    pub line: usize,
    pub snippet: String,
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "repo/code_search.html")]
pub struct SearchPage {
    pub page: Page,
    pub header: RepoHeader,
    pub query: CodeSearchQuery,
    pub matches: Vec<Match>,
    pub truncated: bool,
    pub skipped: usize,
    pub commit: String,
}

/// Bounded at the service boundary before any content search.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
pub struct CodeSearchQuery {
    pub q: String,
    pub path: String,
    pub language: String,
}
