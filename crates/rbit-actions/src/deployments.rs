//! Promotion and rollback select immutable successful attempts, while executing current workflows.

/// Why a writer starts the configured deployment workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Promote,
    Rollback,
}

impl Operation {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "promote" => Some(Self::Promote),
            "rollback" => Some(Self::Rollback),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Rollback => "rollback",
        }
    }
}

/// Provenance recorded when the selected job attempt completed.
#[expect(clippy::struct_excessive_bools, reason = "one flag per persisted provenance restriction")]
#[derive(Debug, Clone, Copy)]
pub struct Source<'a> {
    pub successful: bool,
    pub trusted: bool,
    pub fork: bool,
    pub pull_request: bool,
    pub environment: Option<&'a str>,
}

/// Only a successful own-repository trusted attempt may supply deployment identity.
/// Promotion takes the configured staging environment; rollback takes a previous target attempt.
pub fn check_source(operation: Operation, source: Source<'_>, staging: &str, target: &str) -> Result<(), &'static str> {
    if !source.successful || !source.trusted || source.fork || source.pull_request {
        return Err("choose a successful trusted deployment of this repository's own code");
    }
    let expected = match operation {
        Operation::Promote => staging,
        Operation::Rollback => target,
    };
    if !source.environment.is_some_and(|environment| environment.eq_ignore_ascii_case(expected)) {
        return Err("the selected attempt belongs to the wrong source environment");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Operation, Source, check_source};

    #[test]
    fn deployment_sources_require_exact_successful_trusted_environment_history() {
        let staging = Source { successful: true, trusted: true, fork: false, pull_request: false, environment: Some("Staging") };
        assert!(check_source(Operation::Promote, staging, "staging", "production").is_ok());
        assert!(check_source(Operation::Rollback, staging, "staging", "production").is_err());
        let production = Source { environment: Some("Production"), ..staging };
        assert!(check_source(Operation::Rollback, production, "staging", "production").is_ok());
        assert!(check_source(Operation::Promote, production, "staging", "production").is_err());
        for refused in [
            Source { successful: false, ..staging },
            Source { trusted: false, ..staging },
            Source { fork: true, ..staging },
            Source { pull_request: true, ..staging },
            Source { environment: None, ..staging },
        ] {
            assert!(check_source(Operation::Promote, refused, "staging", "production").is_err());
        }
    }
}
