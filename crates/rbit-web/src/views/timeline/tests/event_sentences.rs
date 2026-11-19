use super::*;

#[test]
#[expect(clippy::too_many_lines, reason = "one case per event kind")]
fn every_event_kind_has_a_sentence() {
    let ctx = context(IssueKind::Pull, all());
    let alice = account(1, "alice");
    let bob = account(2, "bob");
    let none = EventSubjects::default;
    let bob_subject = || EventSubjects { account: Some(bob.clone()), source: None };
    let label = LabelSnapshot { id: LabelId(1), name: "bug".to_owned(), color: "#d73a4a".to_owned() };
    let branches = TimelineOptions { head_branch: Some("feature"), base_branch: Some("main"), ..TimelineOptions::default() };
    let plain = TimelineOptions::default();
    let a = Some(&alice);
    let cases: Vec<(TimelineEvent, EventSubjects, TimelineOptions<'_>, &str, &str)> = vec![
        (
            TimelineEvent::Closed { reason: StateReason::Completed, commit: None, pull: None },
            none(),
            plain,
            "circle-check",
            "closed this as completed",
        ),
        (
            TimelineEvent::Closed { reason: StateReason::NotPlanned, commit: None, pull: None },
            none(),
            plain,
            "circle-slash",
            "closed this as not planned",
        ),
        (
            TimelineEvent::Closed { reason: StateReason::Completed, commit: Some("abc1234def".to_owned()), pull: None },
            none(),
            plain,
            "circle-check",
            "closed this as completed in abc1234",
        ),
        (
            TimelineEvent::Closed { reason: StateReason::Completed, commit: None, pull: IssueNumber::new(2) },
            none(),
            plain,
            "circle-check",
            "closed this as completed in #2",
        ),
        (
            TimelineEvent::Closed { reason: StateReason::Merged, commit: None, pull: None },
            none(),
            plain,
            "git-merge",
            "closed this as merged",
        ),
        (TimelineEvent::Reopened, none(), plain, "circle-dot", "reopened this"),
        (TimelineEvent::Renamed { from: "Old".to_owned(), to: "New".to_owned() }, none(), plain, "pencil", "changed the title Old New"),
        (TimelineEvent::Locked, none(), plain, "lock", "locked the conversation and limited it to collaborators"),
        (TimelineEvent::Unlocked, none(), plain, "unlock", "unlocked this conversation"),
        (TimelineEvent::Labeled { label: label.clone() }, none(), plain, "tag", "added the bug label"),
        (TimelineEvent::Unlabeled { label }, none(), plain, "tag", "removed the bug label"),
        (
            TimelineEvent::Milestoned { milestone: MilestoneId(1), title: "v1".to_owned() },
            none(),
            plain,
            "milestone",
            "added this to the v1 milestone",
        ),
        (
            TimelineEvent::Demilestoned { milestone: MilestoneId(1), title: "v1".to_owned() },
            none(),
            plain,
            "milestone",
            "removed this from the v1 milestone",
        ),
        (TimelineEvent::Assigned { assignee: bob.id }, bob_subject(), plain, "user-plus", "assigned bob"),
        (TimelineEvent::Assigned { assignee: alice.id }, none(), plain, "user-plus", "self-assigned this"),
        (TimelineEvent::Unassigned { assignee: bob.id }, bob_subject(), plain, "user", "unassigned bob"),
        (TimelineEvent::Unassigned { assignee: alice.id }, none(), plain, "user", "removed their assignment"),
        (TimelineEvent::Assigned { assignee: AccountId(99) }, none(), plain, "user-plus", "assigned ghost"),
        (
            TimelineEvent::CommitReferenced { commit: "abc1234def".to_owned(), summary: "Fix it".to_owned() },
            none(),
            plain,
            "commit",
            "referenced this in commit abc1234",
        ),
        (
            TimelineEvent::CrossReferenced { source: IssueId(8) },
            EventSubjects {
                account: None,
                source: Some(CrossReferenceSource {
                    full_name: "carol/other".to_owned(),
                    number: IssueNumber::new(3).unwrap(),
                    kind: IssueKind::Issue,
                    state: IssueState::Open,
                    title: "Related".to_owned(),
                }),
            },
            plain,
            "link",
            "mentioned this in issue Related carol/other#3",
        ),
        (
            TimelineEvent::HeadPushed { before: "abc1234aaa".to_owned(), after: "def5678bbb".to_owned(), count: 2, forced: true },
            none(),
            branches,
            "git-pull-request",
            "force-pushed the feature branch from abc1234 to def5678",
        ),
        (
            TimelineEvent::HeadPushed { before: "abc1234aaa".to_owned(), after: "def5678bbb".to_owned(), count: 1, forced: false },
            none(),
            branches,
            "commit",
            "added 1 commit to feature, now at def5678",
        ),
        (
            TimelineEvent::BaseChanged { from: "main".to_owned(), to: "dev".to_owned() },
            none(),
            plain,
            "branch",
            "changed the base branch from main to dev",
        ),
        (
            TimelineEvent::Merged { commit: "abc1234fff".to_owned(), strategy: Some(MergeStrategy::Squash) },
            none(),
            branches,
            "git-merge",
            "merged commit abc1234 into main (squash)",
        ),
        (
            TimelineEvent::Merged { commit: "abc1234fff".to_owned(), strategy: None },
            none(),
            plain,
            "git-merge",
            "merged commit abc1234 (outside this instance)",
        ),
        (TimelineEvent::ReviewRequested { reviewer: bob.id }, bob_subject(), plain, "eye", "requested a review from bob"),
        (TimelineEvent::CodeOwnerReviewRequested { reviewer: bob.id }, bob_subject(), plain, "eye", "requested a review from bob"),
        (
            TimelineEvent::ReviewRequestRemoved { reviewer: bob.id },
            bob_subject(),
            plain,
            "eye-off",
            "removed the request for review from bob",
        ),
        (
            TimelineEvent::ReviewDismissed { review: ReviewId(4), reviewer: Some(bob.id) },
            bob_subject(),
            plain,
            "x",
            "dismissed bob’s review",
        ),
        (TimelineEvent::ReviewDismissed { review: ReviewId(4), reviewer: None }, none(), plain, "x", "dismissed a review"),
        (
            TimelineEvent::ReviewThreadResolved { comment: rbit_core::pull::ReviewCommentId(9) },
            none(),
            plain,
            "message-square",
            "resolved a review conversation",
        ),
        (
            TimelineEvent::ReviewThreadReopened { comment: rbit_core::pull::ReviewCommentId(9) },
            none(),
            plain,
            "message-square",
            "reopened a review conversation",
        ),
        (TimelineEvent::HeadRefDeleted { branch: "feature".to_owned() }, none(), plain, "trash", "deleted the feature branch"),
        (TimelineEvent::ConvertedToDraft, none(), plain, "git-pull-request-draft", "marked this pull request as draft"),
        (TimelineEvent::ReadyForReview, none(), plain, "eye", "marked this pull request as ready for review"),
    ];
    let mut kinds = std::collections::HashSet::new();
    for (timeline_event, subjects, options, icon, expected) in cases {
        kinds.insert(timeline_event.kind());
        let (actual_icon, sentence) = sentence_of(&ctx, a, timeline_event, subjects, options);
        assert_eq!((actual_icon.as_str(), sentence.as_str()), (icon, expected));
    }
    assert_eq!(kinds.len(), 25, "every TimelineEvent kind is covered");
}

#[test]
fn automatic_owner_requests_are_localized_without_relabeling_deleted_users() {
    let ctx = context(IssueKind::Pull, all());
    let bob = account(2, "bob");
    for (locale, expected) in [
        (Locale::English, "CODEOWNERS automation requested a review from bob"),
        (Locale::Spanish, "Automatización de CODEOWNERS solicitó una revisión a bob"),
        (Locale::Russian, "Автоматизация CODEOWNERS запросила ревью у bob"),
    ] {
        let timeline = Timeline {
            entries: vec![
                event(
                    1,
                    None,
                    TimelineEvent::CodeOwnerReviewRequested { reviewer: bob.id },
                    EventSubjects { account: Some(bob.clone()), ..Default::default() },
                ),
                event(
                    2,
                    None,
                    TimelineEvent::ReviewRequested { reviewer: bob.id },
                    EventSubjects { account: Some(bob.clone()), ..Default::default() },
                ),
            ],
            next: None,
        };
        let views = items(&ctx, &timeline, &refs(), TimelineOptions { locale, ..Default::default() });
        let automatic = views[0].event.as_ref().unwrap();
        assert_eq!(Fragment::plain(&automatic.sentence), expected);
        assert!(!automatic.fallback);
        assert!(automatic.sentence[0].url.is_none(), "automation is not an account link");
        let deleted = views[1].event.as_ref().unwrap();
        let actor = deleted.sentence.iter().find(|part| part.kind == "actor").unwrap();
        assert_eq!(actor.text, "ghost", "deleted human actors keep their original meaning");
    }
}
