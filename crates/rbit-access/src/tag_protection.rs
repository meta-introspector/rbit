//! Protected tags are immutable; repository administrators alone publish matching releases.

use crate::{
    RepoAccess,
    protection::{BranchPattern, RefChange, Refusal},
};

pub const MAX_TAG_RULES: i64 = 100;

/// Rules never grant access: callers must first authorize the repository operation.
pub fn matching<'a>(patterns: &'a [BranchPattern], tag: &str) -> Option<&'a BranchPattern> {
    patterns.iter().find(|pattern| pattern.matches(tag))
}

/// A protected tag cannot move or disappear, even for administrators. Remove its rule
/// deliberately before rewriting it. Namespace aliases cannot bypass rules on case-folding hosts.
pub fn check_push(patterns: &[BranchPattern], access: RepoAccess, person: bool, update: &impl RefChange) -> Option<Refusal> {
    let mut parts = update.name().splitn(3, '/');
    let (Some(refs), Some(tags), Some(tag)) = (parts.next(), parts.next(), parts.next()) else { return None };
    let equal = |left: &str, right: &str| left == right || (cfg!(any(windows, target_os = "macos")) && left.eq_ignore_ascii_case(right));
    if !equal(refs, "refs") || !equal(tags, "tags") {
        return None;
    }
    let pattern = matching(patterns, tag)?;
    let (code, reason) = if !update.is_created() || update.is_deleted() {
        ("protected_tag_immutable", "protected tag: moving or deleting it is not allowed")
    } else if !person || !access.can_admin() {
        ("protected_tag_publisher", "protected tag: only repository administrators may publish it")
    } else if refs != "refs" || tags != "tags" {
        ("ref_not_canonical", "use the canonical refs/tags/ namespace")
    } else {
        return None;
    };
    Some(
        Refusal::new(update.name(), code, reason, "Ask a repository administrator to review the tag protection rule.")
            .with_rule(format!("tag protection {pattern}")),
    )
}

/// Protect release metadata and attachments as well as the Git tag.
pub fn may_publish(patterns: &[BranchPattern], tag: &str, access: RepoAccess, person: bool) -> bool {
    matching(patterns, tag).is_none() || (person && access.can_admin())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Update {
        name: &'static str,
        created: bool,
        deleted: bool,
    }
    impl RefChange for Update {
        fn name(&self) -> &str {
            self.name
        }
        fn is_created(&self) -> bool {
            self.created
        }
        fn is_deleted(&self) -> bool {
            self.deleted
        }
    }
    #[test]
    fn only_administrators_create_and_nobody_rewrites() {
        let patterns = vec![BranchPattern::parse("v*").unwrap()];
        let mut update = Update { name: "refs/tags/v1", created: true, deleted: false };
        assert!(check_push(&patterns, RepoAccess::Admin, true, &update).is_none());
        assert!(check_push(&patterns, RepoAccess::Write, true, &update).is_some());
        assert!(check_push(&patterns, RepoAccess::Admin, false, &update).is_some());
        update.created = false;
        assert_eq!(check_push(&patterns, RepoAccess::Admin, true, &update).unwrap().code, "protected_tag_immutable");
        update.deleted = true;
        assert!(check_push(&patterns, RepoAccess::Admin, true, &update).is_some());
        update.name = "refs/heads/v1";
        assert!(check_push(&patterns, RepoAccess::Write, true, &update).is_none());
        assert!(!may_publish(&patterns, "v1", RepoAccess::Write, true));
        assert!(may_publish(&patterns, "nightly", RepoAccess::Write, false));
    }
}
