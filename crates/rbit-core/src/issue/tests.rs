use super::*;
use crate::{Account, AccountKind, AccountName, Email, RepoId, Timestamp};

fn user(id: i64) -> User {
    User {
        is_agent: false,
        agent_owner_id: None,
        account: Account {
            id: AccountId(id),
            kind: AccountKind::User,
            name: format!("user{id}").parse::<AccountName>().unwrap(),
            display_name: None,
            bio: None,
            created_at: Timestamp(0),
        },
        email: "user@example.com".parse::<Email>().unwrap(),
        is_admin: false,
    }
}

fn issue(kind: IssueKind, locked: bool) -> Issue {
    Issue {
        id: IssueId(1),
        repo_id: RepoId(1),
        number: IssueNumber::new(1).unwrap(),
        kind,
        title: IssueTitle::parse("A bug").unwrap(),
        body: IssueText::default(),
        author: Some(AccountId(10)),
        state: IssueState::Open,
        state_reason: None,
        closed_at: None,
        closed_by: None,
        locked,
        comment_count: 0,
        created_at: Timestamp(0),
        updated_at: Timestamp(0),
    }
}

#[test]
fn issue_numbers_parse_only_canonical_positive_integers() {
    assert_eq!(IssueNumber::parse("12").unwrap().get(), 12);
    assert_eq!(IssueNumber::parse("4294967295").unwrap().get(), u32::MAX);
    for bad in ["", "0", "012", "-1", "+1", "1.0", " 1", "1 ", "#1", "abc", "4294967296", "99999999999", "１"] {
        assert!(IssueNumber::parse(bad).is_err(), "{bad:?}");
    }
    assert_eq!(IssueNumber::new(7).unwrap().to_string(), "#7");
    assert_eq!(IssueNumber::new(0), None);
    assert!(IssueNumber::try_from(-3_i64).is_err());
    assert_eq!(IssueNumber::try_from(3_i64).unwrap().get(), 3);
}

#[test]
fn issue_numbers_deserialize_with_validation() {
    assert_eq!(serde_json::from_str::<IssueNumber>("5").unwrap().get(), 5);
    assert!(serde_json::from_str::<IssueNumber>("0").is_err());
    assert_eq!(serde_json::to_string(&IssueNumber::new(5).unwrap()).unwrap(), "5");
}

#[test]
fn text_enums_round_trip() {
    for kind in IssueKind::ALL {
        assert_eq!(IssueKind::parse(kind.as_str()), Some(*kind));
    }
    for state in IssueState::ALL {
        assert_eq!(IssueState::parse(state.as_str()), Some(*state));
    }
    for reason in StateReason::ALL {
        assert_eq!(StateReason::parse(reason.as_str()), Some(*reason));
        assert_eq!(serde_json::to_string(reason).unwrap(), format!("\"{}\"", reason.as_str()));
    }
    for strategy in MergeStrategy::ALL {
        assert_eq!(MergeStrategy::parse(strategy.as_str()), Some(*strategy));
    }
    for state in ReviewState::ALL {
        assert_eq!(ReviewState::parse(state.as_str()), Some(*state));
        assert_eq!(serde_json::to_string(state).unwrap(), format!("\"{}\"", state.as_str()));
    }
    assert_eq!(IssueKind::parse("pr"), None);
    assert_eq!(StateReason::NotPlanned.as_str(), "not_planned");
    assert_eq!(ReviewState::ChangesRequested.as_str(), "changes_requested");
}

#[test]
fn titles_are_trimmed_and_bounded() {
    assert_eq!(IssueTitle::parse("  Crash on start \t").unwrap().as_str(), "Crash on start");
    assert_eq!(IssueTitle::parse("   "), Err(IssueTextError::Empty { kind: "title" }));
    assert_eq!(IssueTitle::parse("a\u{7}b"), Err(IssueTextError::ControlCharacter { kind: "title" }));
    assert_eq!(IssueTitle::parse("line\nbreak"), Err(IssueTextError::ControlCharacter { kind: "title" }));
    assert!(IssueTitle::parse(&"é".repeat(MAX_TITLE_CHARS)).is_ok(), "characters, not bytes");
    assert_eq!(
        IssueTitle::parse(&"x".repeat(MAX_TITLE_CHARS + 1)),
        Err(IssueTextError::TooManyCharacters { kind: "title", max: MAX_TITLE_CHARS })
    );
}

#[test]
fn texts_normalise_line_endings_and_are_bounded() {
    assert_eq!(IssueText::body("a\r\nb\rc\n").unwrap().as_str(), "a\nb\nc\n");
    assert!(IssueText::body("").unwrap().is_empty());
    assert_eq!(IssueText::comment(" \r\n\t"), Err(IssueTextError::Empty { kind: "comment" }));
    assert_eq!(IssueText::comment("LGTM").unwrap().as_str(), "LGTM");
    assert!(IssueText::body(&"x".repeat(MAX_TEXT_BYTES)).is_ok());
    assert_eq!(IssueText::body(&"x".repeat(MAX_TEXT_BYTES + 1)), Err(IssueTextError::TooLong { kind: "description", max: MAX_TEXT_BYTES }));
    // Normalisation happens before the limit: CRLF counts as one byte.
    assert_eq!(IssueText::comment(&"x\r\n".repeat(MAX_TEXT_BYTES / 2)).unwrap().as_str().len(), MAX_TEXT_BYTES);
}

#[test]
fn timeline_events_serialize_as_kind_and_data() {
    let label = LabelSnapshot { id: LabelId(3), name: "bug".to_owned(), color: "#d73a4a".to_owned() };
    let cases = [
        (
            TimelineEvent::Closed { reason: StateReason::Completed, commit: Some("abc".to_owned()), pull: None },
            r#"{"kind":"closed","data":{"reason":"completed","commit":"abc"}}"#,
        ),
        (TimelineEvent::Reopened, r#"{"kind":"reopened"}"#),
        (TimelineEvent::Labeled { label }, r##"{"kind":"labeled","data":{"label":{"id":3,"name":"bug","color":"#d73a4a"}}}"##),
        (TimelineEvent::CrossReferenced { source: IssueId(9) }, r#"{"kind":"cross_referenced","data":{"source":9}}"#),
        (
            TimelineEvent::Merged { commit: "def".to_owned(), strategy: Some(MergeStrategy::Squash) },
            r#"{"kind":"merged","data":{"commit":"def","strategy":"squash"}}"#,
        ),
        (
            TimelineEvent::Merged { commit: "def".to_owned(), strategy: None },
            r#"{"kind":"merged","data":{"commit":"def","strategy":null}}"#,
        ),
        (TimelineEvent::ReadyForReview, r#"{"kind":"ready_for_review"}"#),
    ];
    for (event, json) in cases {
        assert_eq!(serde_json::to_string(&event).unwrap(), json);
        assert_eq!(serde_json::from_str::<TimelineEvent>(json).unwrap(), event);
        assert!(json.contains(&format!("\"kind\":\"{}\"", event.kind())), "{json}");
    }
    assert!(serde_json::from_str::<TimelineEvent>(r#"{"kind":"teleported"}"#).is_err(), "unknown kinds are rejected");
}

#[test]
fn every_timeline_kind_matches_serde() {
    let account = AccountId(1);
    let label = LabelSnapshot { id: LabelId(1), name: "x".to_owned(), color: "#000000".to_owned() };
    let all = [
        TimelineEvent::Closed { reason: StateReason::Merged, commit: None, pull: IssueNumber::new(2) },
        TimelineEvent::Reopened,
        TimelineEvent::Renamed { from: "a".to_owned(), to: "b".to_owned() },
        TimelineEvent::Locked,
        TimelineEvent::Unlocked,
        TimelineEvent::Labeled { label: label.clone() },
        TimelineEvent::Unlabeled { label },
        TimelineEvent::Milestoned { milestone: MilestoneId(1), title: "v1".to_owned() },
        TimelineEvent::Demilestoned { milestone: MilestoneId(1), title: "v1".to_owned() },
        TimelineEvent::Assigned { assignee: account },
        TimelineEvent::Unassigned { assignee: account },
        TimelineEvent::CommitReferenced { commit: "abc".to_owned(), summary: "Fix".to_owned() },
        TimelineEvent::CrossReferenced { source: IssueId(1) },
        TimelineEvent::HeadPushed { before: "a".to_owned(), after: "b".to_owned(), count: 2, forced: true },
        TimelineEvent::BaseChanged { from: "main".to_owned(), to: "dev".to_owned() },
        TimelineEvent::Merged { commit: "c".to_owned(), strategy: Some(MergeStrategy::Rebase) },
        TimelineEvent::ReviewRequested { reviewer: account },
        TimelineEvent::CodeOwnerReviewRequested { reviewer: account },
        TimelineEvent::ReviewRequestRemoved { reviewer: account },
        TimelineEvent::ReviewDismissed { review: ReviewId(1), reviewer: None },
        TimelineEvent::ReviewThreadResolved { comment: crate::pull::ReviewCommentId(1) },
        TimelineEvent::ReviewThreadReopened { comment: crate::pull::ReviewCommentId(1) },
        TimelineEvent::HeadRefDeleted { branch: "feature".to_owned() },
        TimelineEvent::ConvertedToDraft,
        TimelineEvent::ReadyForReview,
    ];
    for event in all {
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["kind"], event.kind());
        assert_eq!(serde_json::from_value::<TimelineEvent>(value).unwrap(), event);
    }
}

/// Who the viewer is relative to the thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Viewer {
    Anonymous,
    Author,
    Other,
}

/// The permission table of `docs/design/milestone-2b.md`, written out independently.
fn expected(viewer: Viewer, access: RepoAccess, locked: bool, kind: IssueKind) -> IssuePermissions {
    let signed_in = viewer != Viewer::Anonymous;
    let read = signed_in && access >= RepoAccess::Read;
    let write = signed_in && access >= RepoAccess::Write;
    let admin = signed_in && access >= RepoAccess::Admin;
    let author = read && viewer == Viewer::Author;
    IssuePermissions {
        can_comment: if locked { write } else { read },
        can_edit: author || write,
        can_set_state: author || write,
        can_lock: write,
        can_triage: write,
        can_delete: admin && kind == IssueKind::Issue,
        can_moderate: write,
    }
}

#[test]
fn issue_permissions_follow_the_table_exhaustively() {
    let accesses = [RepoAccess::None, RepoAccess::Read, RepoAccess::Write, RepoAccess::Admin];
    let author = user(10);
    let other = user(20);
    let mut admin = user(30);
    admin.is_admin = true; // site administrators get their power through `access`, not here
    for viewer in [Viewer::Anonymous, Viewer::Author, Viewer::Other] {
        for access in accesses {
            for locked in [false, true] {
                for kind in IssueKind::ALL.iter().copied() {
                    let thread = issue(kind, locked);
                    let user = match viewer {
                        Viewer::Anonymous => None,
                        Viewer::Author => Some(&author),
                        Viewer::Other => Some(&other),
                    };
                    let actual = issue_permissions(user, access, &thread);
                    assert_eq!(actual, expected(viewer, access, locked, kind), "{viewer:?} {access:?} locked={locked} {kind:?}");
                    if viewer == Viewer::Other {
                        assert_eq!(issue_permissions(Some(&admin), access, &thread), actual, "is_admin alone grants nothing");
                    }
                }
            }
        }
    }
}

#[test]
fn a_thread_without_author_is_nobody_s() {
    let mut thread = issue(IssueKind::Issue, false);
    thread.author = None;
    let permissions = issue_permissions(Some(&user(10)), RepoAccess::Read, &thread);
    assert!(permissions.can_comment && !permissions.can_edit && !permissions.can_set_state);
}

#[test]
fn comment_permissions_combine_ownership_and_moderation() {
    let author = user(10);
    let other = user(20);
    let comment = Comment {
        id: CommentId(1),
        issue_id: IssueId(1),
        author: Some(author.id()),
        body: IssueText::comment("hi").unwrap(),
        created_at: Timestamp(0),
        updated_at: Timestamp(0),
        edited_at: None,
        source: CommentSource::User,
    };
    let open = issue(IssueKind::Issue, false);
    let locked = issue(IssueKind::Issue, true);
    let check = |viewer: Option<&User>, access, thread: &Issue| {
        let permissions = comment_permissions(viewer, issue_permissions(viewer, access, thread), &comment);
        (permissions.can_edit, permissions.can_delete)
    };
    assert_eq!(check(Some(&author), RepoAccess::Read, &open), (true, true));
    assert_eq!(check(Some(&author), RepoAccess::Read, &locked), (false, false), "locked threads freeze readers");
    assert_eq!(check(Some(&author), RepoAccess::Write, &locked), (true, true));
    assert_eq!(check(Some(&other), RepoAccess::Read, &open), (false, false));
    assert_eq!(check(Some(&other), RepoAccess::Write, &open), (false, true), "writers moderate, never edit");
    assert_eq!(check(None, RepoAccess::Admin, &open), (false, false));
    assert_eq!(check(Some(&author), RepoAccess::None, &open), (false, false));
}

#[test]
fn subject_accounts() {
    assert_eq!(TimelineEvent::Assigned { assignee: AccountId(4) }.subject_account(), Some(AccountId(4)));
    assert_eq!(TimelineEvent::ReviewDismissed { review: ReviewId(1), reviewer: None }.subject_account(), None);
    assert_eq!(TimelineEvent::Locked.subject_account(), None);
}
