//! Audit identifiers and literal search-pattern regressions.
use super::*;

#[test]
fn identifiers_round_trip_and_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for action in AuditAction::ALL {
        assert_eq!(AuditAction::parse(action.as_str()), Some(action));
        assert!(seen.insert(action.as_str()), "{} is duplicated", action.as_str());
    }
    for category in AuditCategory::ALL {
        assert_eq!(AuditCategory::parse(category.as_str()), Some(category));
        assert!(AuditAction::ALL.iter().any(|action| action.category() == category), "{category:?} has no actions");
    }
    assert_eq!(AuditAction::parse("nope"), None);
}

#[test]
fn escapes_like_wildcards() {
    assert_eq!(like_pattern("a_b%c\\"), "%a\\_b\\%c\\\\%");
}
