//! Human-owned agent identities, explicit capabilities and bounded repository/organization scopes.

use rbit_contracts::{AccountId, Timestamp};
use serde::{Deserialize, Serialize};

pub const MAX_AGENTS: i64 = 50;
pub const MAX_SCOPES: usize = 32;
pub const MAX_CREDENTIALS: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Read,
    Push,
    Issues,
    PullRequests,
    Statuses,
    Dispatch,
    ReadLogs,
    Merge,
}

impl Capability {
    pub const ALL: [Self; 8] =
        [Self::Read, Self::Push, Self::Issues, Self::PullRequests, Self::Statuses, Self::Dispatch, Self::ReadLogs, Self::Merge];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Push => "push",
            Self::Issues => "issues",
            Self::PullRequests => "pull_requests",
            Self::Statuses => "statuses",
            Self::Dispatch => "dispatch",
            Self::ReadLogs => "read_logs",
            Self::Merge => "merge",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|capability| capability.as_str() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAccess {
    Read,
    Write,
}

impl AgentAccess {
    pub const fn level(self) -> i64 {
        match self {
            Self::Read => 1,
            Self::Write => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentScope {
    Repository { id: i64, access: AgentAccess },
    Organization { id: i64, access: AgentAccess },
}

impl AgentScope {
    pub const fn target(&self) -> (&'static str, i64) {
        match *self {
            Self::Repository { id, .. } => ("repository", id),
            Self::Organization { id, .. } => ("organization", id),
        }
    }

    pub const fn access(&self) -> AgentAccess {
        match *self {
            Self::Repository { access, .. } | Self::Organization { access, .. } => access,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInput {
    pub slug: String,
    pub name: String,
    pub capabilities: Vec<Capability>,
    pub scopes: Vec<AgentScope>,
}

impl AgentInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.slug.is_empty()
            || self.slug.len() > 12
            || !self.slug.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || self.slug.starts_with('-')
            || self.slug.ends_with('-')
            || self.slug.contains("--")
        {
            return Err("agent slug must be 1 to 12 lowercase letters, numbers or single hyphens");
        }
        if self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.chars().count() > 80
            || self.name.chars().any(char::is_control)
        {
            return Err("agent name must contain 1 to 80 characters without control characters or surrounding whitespace");
        }
        if self.capabilities.is_empty() || self.capabilities.len() > Capability::ALL.len() || !self.capabilities.contains(&Capability::Read)
        {
            return Err("select read access and at most eight agent capabilities");
        }
        if self.capabilities.iter().collect::<std::collections::BTreeSet<_>>().len() != self.capabilities.len() {
            return Err("agent capabilities must be unique");
        }
        if self.scopes.is_empty() || self.scopes.len() > MAX_SCOPES || self.scopes.iter().any(|scope| scope.target().1 <= 0) {
            return Err("select between one and 32 repository or organization scopes");
        }
        if self.scopes.iter().map(AgentScope::target).collect::<std::collections::BTreeSet<_>>().len() != self.scopes.len() {
            return Err("agent scope targets must be unique");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub account_id: AccountId,
    pub owner_id: AccountId,
    pub handle: String,
    pub slug: String,
    pub name: String,
    pub enabled: bool,
    pub revision: i64,
    pub capabilities: Vec<Capability>,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_inputs_are_bounded_explicit_and_non_administrative() {
        let mut input = AgentInput {
            slug: "review-bot".into(),
            name: "Review assistant".into(),
            capabilities: vec![Capability::Read, Capability::PullRequests],
            scopes: vec![AgentScope::Organization { id: 1, access: AgentAccess::Write }],
        };
        assert!(input.validate().is_ok());
        assert_eq!(Capability::parse("admin"), None);
        assert_eq!(Capability::parse("approve"), None);
        input.scopes.push(input.scopes[0].clone());
        assert!(input.validate().is_err());
        input.scopes.pop();
        input.slug = "../escape".into();
        assert!(input.validate().is_err());
        input.slug = "review-bot".into();
        input.capabilities.clear();
        assert!(input.validate().is_err());
    }
}
