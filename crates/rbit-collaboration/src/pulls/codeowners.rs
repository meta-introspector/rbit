//! Bounded, case-sensitive CODEOWNERS rules. The last matching rule owns a path.

/// Maximum source bytes and rules accepted from a repository.
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_RULES: usize = 256;
pub const MAX_OWNERS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeOwners {
    rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    pattern: String,
    owners: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid CODEOWNERS: {0}")]
pub struct OwnerError(pub &'static str);

impl CodeOwners {
    /// Parse a bounded subset of CODEOWNERS: users/teams, *, **, ?, rooted paths
    /// and directory rules. Unsupported syntax fails closed instead of dropping rules.
    pub fn parse(source: &str) -> Result<Self, OwnerError> {
        if source.len() > MAX_BYTES {
            return Err(OwnerError("file exceeds 64 KiB"));
        }
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        let mut rules = Vec::new();
        for line in source.lines() {
            let line = line.split('#').next().unwrap_or_default().trim();
            if line.is_empty() {
                continue;
            }
            let mut words = line.split_whitespace();
            let pattern = words.next().unwrap_or_default();
            if pattern.chars().any(char::is_control)
                || pattern.trim_matches('/').is_empty()
                || pattern.len() > 256
                || pattern.contains(['!', '[', ']', '\\'])
                || pattern.contains("***")
                || pattern.contains("..")
            {
                return Err(OwnerError("unsupported pattern"));
            }
            let owners: Vec<String> = words.map(str::to_owned).collect();
            if owners.len() > MAX_OWNERS
                || owners.iter().any(|owner| {
                    !owner.starts_with('@')
                        || owner.len() < 2
                        || owner.len() > 201
                        || owner[1..].split('/').count() > 2
                        || owner[1..].split('/').any(|part| {
                            part.is_empty() || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
                        })
                })
            {
                return Err(OwnerError("use @user or @organization/team owners"));
            }
            rules.push(Rule { pattern: pattern.to_owned(), owners });
            if rules.len() > MAX_RULES {
                return Err(OwnerError("too many rules"));
            }
        }
        Ok(Self { rules })
    }

    /// None means unmatched. An empty owner list explicitly clears earlier ownership.
    pub fn owners(&self, path: &str) -> Option<&[String]> {
        self.rules.iter().rev().find(|rule| matches_path(&rule.pattern, path)).map(|rule| rule.owners.as_slice())
    }
}

fn matches_path(pattern: &str, path: &str) -> bool {
    let rooted = pattern.starts_with('/');
    let pattern = pattern.trim_start_matches('/');
    let directory = pattern.ends_with('/');
    let pattern = pattern.trim_end_matches('/');
    let prefix = if rooted || pattern.contains('/') { "" } else { "**/" };
    let pattern = format!("{prefix}{pattern}");
    (!directory && glob(pattern.as_bytes(), path.as_bytes())) || glob(format!("{pattern}/**").as_bytes(), path.as_bytes())
}

// Dynamic programming avoids recursive/exponential wildcard backtracking.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    let mut previous = vec![false; text.len() + 1];
    previous[0] = true;
    let mut index = 0;
    while index < pattern.len() {
        let byte = pattern[index];
        let double = byte == b'*' && pattern.get(index + 1) == Some(&b'*');
        let mut next = vec![false; text.len() + 1];
        if byte == b'*' {
            next[0] = previous[0];
            for offset in 1..=text.len() {
                next[offset] = previous[offset] || (next[offset - 1] && (double || text[offset - 1] != b'/'));
            }
        } else {
            for offset in 1..=text.len() {
                next[offset] = previous[offset - 1] && (byte == text[offset - 1] || (byte == b'?' && text[offset - 1] != b'/'));
            }
        }
        if double && pattern.get(index + 2) == Some(&b'/') {
            let mut slash = previous.clone();
            for offset in 1..=text.len() {
                slash[offset] |= next[offset - 1] && text[offset - 1] == b'/';
            }
            next = slash;
            index += 1;
        }
        previous = next;
        index += if double { 2 } else { 1 };
    }
    previous[text.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn last_match_and_explicit_clear() {
        assert_eq!(CodeOwners::parse("\u{feff}* @alice").unwrap().owners("a.rs").unwrap(), ["@alice"]);
        assert!(CodeOwners::parse("* @john.doe @my.org/core.team").is_ok());
        let rules = CodeOwners::parse("* @all\n/src/ @org/core\n/src/generated/\n").unwrap();
        assert_eq!(rules.owners("README.md").unwrap(), ["@all"]);
        assert_eq!(rules.owners("src/lib.rs").unwrap(), ["@org/core"]);
        assert_eq!(rules.owners("src/generated/a.rs").unwrap(), [] as [String; 0]);
    }
    #[test]
    fn glob_paths_are_case_sensitive_and_support_globstar() {
        for path in ["a.rs", "src/a.rs", "src/nested/a.rs"] {
            assert!(matches_path("**/*.rs", path));
        }
        assert!(matches_path("docs/", "nested/docs/file.md"));
        assert!(!matches_path("/docs/", "nested/docs/file.md"));
        assert!(!matches_path("/src/*.rs", "src/deep/a.rs"));
        assert!(!matches_path("*.RS", "a.rs"));
        assert!(matches_path("/src/?ib.rs", "src/lib.rs"));
    }
    #[test]
    fn invalid_rules_cannot_silently_remove_protection() {
        for source in ["!secret @alice", "[ab] @alice", "* alice", "* @org/", "* @a/b/c", "bad\0name @alice"] {
            assert!(CodeOwners::parse(source).is_err(), "{source}");
        }
        assert!(CodeOwners::parse(&"* @a\n".repeat(MAX_RULES + 1)).is_err());
        assert!(CodeOwners::parse(&"#".repeat(MAX_BYTES + 1)).is_err());
    }
}
