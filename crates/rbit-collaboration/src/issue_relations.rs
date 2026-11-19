//! Bounded issue hierarchy and advisory dependencies, projected for one viewer.
use serde::{Deserialize, Serialize};
pub const MAX_LINKS: i64 = 100;
pub const MAX_WALK: i64 = 1000;
pub const MAX_BATCH: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Child,
    BlockedBy,
}
impl RelationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Child => "child",
            Self::BlockedBy => "blocked_by",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationInput {
    pub kind: RelationKind,
    /// Repository name under the current issue's owner; blank means the current repository.
    #[serde(default)]
    pub repository: String,
    pub number: i64,
}
impl RelationInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.number <= 0 || self.repository.len() > 100 || self.repository.chars().any(char::is_control) {
            return Err("invalid related issue reference");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Progress {
    pub completed: u32,
    pub total: u32,
    pub blocked: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct RelatedIssue {
    pub id: i64,
    pub issue_id: i64,
    pub owner: String,
    pub repository: String,
    pub number: i64,
    pub title: String,
    pub state: String,
    pub issue_kind: String,
    /// child, parent, `blocked_by` or blocking, relative to the requested issue.
    pub relationship: String,
    pub can_remove: bool,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Relations {
    pub progress: Progress,
    pub items: Vec<RelatedIssue>,
    pub can_edit: bool,
}
