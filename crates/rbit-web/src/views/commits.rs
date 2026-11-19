//! Commit history list and graph page.

use askama::Template;
use askama_web::WebTemplate;

use super::{
    branch_graph::BranchGraph,
    layout::Page,
    repo::{CommitView, RefSwitcher, RepoHeader},
    when::When,
};

/// Commits grouped under a calendar-day heading.
#[derive(Debug, Clone)]
pub struct CommitDay {
    /// The UTC day, for grouping; the heading shows `when`'s date in the page's language.
    pub date: String,
    pub when: When,
    pub commits: Vec<CommitView>,
}

impl CommitDay {
    /// Groups consecutive commits that share a date (input is newest-first).
    pub fn group(commits: Vec<CommitView>) -> Vec<Self> {
        let mut days: Vec<Self> = Vec::new();
        for commit in commits {
            match days.last_mut() {
                Some(day) if day.date == commit.date => day.commits.push(commit),
                _ => days.push(Self { date: commit.date.clone(), when: commit.when, commits: vec![commit] }),
            }
        }
        days
    }
}

#[derive(Debug, Template, WebTemplate)]
#[template(path = "repo/commits.html")]
pub struct CommitsPage {
    pub page: Page,
    pub header: RepoHeader,
    pub switcher: RefSwitcher,
    pub graph_url: String,
    pub list_url: String,
    pub graph: Option<BranchGraph>,
    pub days: Vec<CommitDay>,
    pub newer_url: Option<String>,
    pub older_url: Option<String>,
}
