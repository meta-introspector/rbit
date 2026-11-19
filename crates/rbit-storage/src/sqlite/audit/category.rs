//! Stable audit categories shared by security and repository events.
/// A group of related actions, used to filter the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuditCategory {
    Authentication,
    TwoFactor,
    Sessions,
    Tokens,
    /// Collaborators, teams, transfers, webhooks and branch protection (milestone 2).
    RepositoryAccess,
    /// Issues and pull requests deleted (milestone 2 wave B), repositories archived and
    /// unarchived.
    RepositoryContent,
    Administration,
}

impl AuditCategory {
    pub const ALL: [Self; 7] = [
        Self::Authentication,
        Self::TwoFactor,
        Self::Sessions,
        Self::Tokens,
        Self::RepositoryAccess,
        Self::RepositoryContent,
        Self::Administration,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::TwoFactor => "two_factor",
            Self::Sessions => "sessions",
            Self::Tokens => "tokens",
            Self::RepositoryAccess => "repository_access",
            Self::RepositoryContent => "repository_content",
            Self::Administration => "administration",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|category| category.as_str() == value)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Authentication => "Sign-in",
            Self::TwoFactor => "Two-factor",
            Self::Sessions => "Sessions",
            Self::Tokens => "Access tokens",
            Self::RepositoryAccess => "Repository access",
            Self::RepositoryContent => "Repository content",
            Self::Administration => "Administration",
        }
    }
}
