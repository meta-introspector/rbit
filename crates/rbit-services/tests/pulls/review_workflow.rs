//! Review workflow against real commits, current grants and persisted reviewer state.
use super::*;
use rbit_core::RefUpdate;
use rbit_services::pulls::review_workflow::{SuggestionSelection, body_digest};

async fn suggest(f: &Fixture, actor: &User, ctx: &PullContext, path: &str, line: u32, replacement: &str) -> SuggestionSelection {
    let body = format!("```suggestion\n{replacement}\n```");
    let comment = f
        .services
        .pulls
        .comment_on_line(
            actor,
            ctx,
            LineComment {
                expected_head: Some(ctx.pull.head_sha.clone()),
                path: path.into(),
                side: ReviewSide::Right,
                line,
                body: body.clone(),
                in_reply_to: None,
                start_review: false,
            },
        )
        .await
        .unwrap();
    SuggestionSelection { comment_id: comment.id.0, body_digest: body_digest(&body) }
}

#[tokio::test]
async fn review_workflow_applies_one_atomic_commit_and_rejects_stale_selection() {
    let f = Fixture::new().await;
    git(&f.work, &["checkout", "-q", "feature"]);
    commit(&f.work, "second.txt", "two\n", "Second file");
    f.push(&f.alice, "alice", &["feature"]).await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let bob = f.open_pull(&f.bob, n).await;
    let first = suggest(&f, &f.bob, &bob, "feature.txt", 1, "ONE").await;
    let second = suggest(&f, &f.bob, &bob, "second.txt", 1, "TWO").await;
    let ctx = f.open_pull(&f.alice, n).await;
    let head = f.repo(&f.alice, "alice").await;
    let mut edited = first.clone();
    edited.body_digest = "0".repeat(64);
    assert!(f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, &[edited]).await.is_err());
    assert!(f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, &[first.clone(), first.clone()]).await.is_err());
    assert_eq!(head.reader.commit_id("feature".into()).await.unwrap(), ctx.pull.head_sha);
    let id = f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, &[first.clone(), second]).await.unwrap();
    // No background listener runs in this fixture: success must already be readable.
    assert_eq!(f.open_pull(&f.alice, n).await.pull.head_sha, id);
    let comments = f.database.reviews().comments(ctx.issue().id).await.unwrap();
    assert_eq!(comments.len(), 2);
    assert!(comments.iter().all(|comment| comment.outdated));
    assert_eq!(head.reader.blob(id.clone(), "feature.txt".into(), 100).await.unwrap().text(), Some("ONE\n"));
    assert_eq!(head.reader.blob(id.clone(), "second.txt".into(), 100).await.unwrap().text(), Some("TWO\n"));
    assert_eq!(git(&head.disk_path, &["rev-parse", &format!("{id}^")]), ctx.pull.head_sha);
    assert!(f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, &[first]).await.is_err());
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_requires_exact_composition_head_and_current_fork_writer() {
    let f = Fixture::new().await;
    let n = f.create(&f.bob, "bob", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, n).await;
    let mut input = LineComment {
        expected_head: None,
        path: "feature.txt".into(),
        side: ReviewSide::Right,
        line: 1,
        body: "```suggestion\nnew\n```".into(),
        in_reply_to: None,
        start_review: false,
    };
    assert!(f.services.pulls.comment_on_line(&f.alice, &ctx, input.clone()).await.is_err());
    input.expected_head = Some(ctx.pull.head_sha.clone());
    input.side = ReviewSide::Left;
    assert!(f.services.pulls.comment_on_line(&f.alice, &ctx, input).await.is_err());
    let selection = suggest(&f, &f.alice, &ctx, "feature.txt", 1, "new").await;
    let fork = f.repo(&f.alice, "bob").await;
    assert!(matches!(
        f.services.pulls.apply_suggestions(&f.alice, &ctx, &fork, &ctx.pull.head_sha, std::slice::from_ref(&selection)).await,
        Err(ServiceError::Forbidden)
    ));
    let bob = f.open_pull(&f.bob, n).await;
    let fork = f.repo(&f.bob, "bob").await;
    let id = f.services.pulls.apply_suggestions(&f.bob, &bob, &fork, &bob.pull.head_sha, &[selection]).await.unwrap();
    let updated = f.open_pull(&f.alice, n).await;
    assert_eq!(updated.pull.head_sha, id);
    assert_eq!(updated.repo().reader.blob(id, "feature.txt".into(), 100).await.unwrap().text(), Some("new\n"));
    assert!(f.database.reviews().comments(ctx.issue().id).await.unwrap().iter().all(|comment| comment.outdated));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_stale_writer_context_cannot_bypass_revocation() {
    let f = Fixture::new().await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.carol, n).await;
    let head = f.repo(&f.carol, "alice").await;
    let selection = suggest(&f, &f.carol, &ctx, "feature.txt", 1, "new").await;
    let owner = f.repo(&f.alice, "alice").await;
    f.services.collaborators.remove(&f.alice, &owner, "carol").await.unwrap();
    assert!(matches!(
        f.services.pulls.apply_suggestions(&f.carol, &ctx, &head, &ctx.pull.head_sha, &[selection]).await,
        Err(ServiceError::Forbidden)
    ));
    assert_eq!(head.reader.commit_id("feature".into()).await.unwrap(), ctx.pull.head_sha);
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_progress_and_last_review_follow_exact_content() {
    let f = Fixture::new().await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.carol, n).await;
    let head = f.repo(&f.carol, "alice").await;
    assert!(f.services.pulls.changes_since_review(&f.carol, &ctx, &head).await.unwrap().is_none());
    let progress = f.services.pulls.review_progress(&f.carol, &ctx).await.unwrap();
    assert_eq!(progress.len(), 1);
    let p = &progress[0];
    f.services.pulls.mark_reviewed_file(&f.carol, &ctx, &p.path, &ctx.pull.head_sha, &p.fingerprint, true).await.unwrap();
    f.services.pulls.submit_review(&f.carol, &ctx, ReviewVerdict::Approve, "").await.unwrap();
    let alice = f.open_pull(&f.alice, n).await;
    assert!(!f.services.pulls.review_progress(&f.alice, &alice).await.unwrap()[0].viewed);
    git(&f.work, &["checkout", "-q", "feature"]);
    commit(&f.work, "other.txt", "new\n", "Unrelated file");
    f.push(&f.alice, "alice", &["feature"]).await;
    let next = f.open_pull(&f.carol, n).await;
    let progress = f.services.pulls.review_progress(&f.carol, &next).await.unwrap();
    assert!(progress.iter().find(|p| p.path == "feature.txt").unwrap().viewed);
    let (baseline, changes) = f.services.pulls.changes_since_review(&f.carol, &next, &head).await.unwrap().unwrap();
    assert_eq!(baseline, ctx.pull.head_sha);
    assert_eq!(changes.files.len(), 1);
    assert_eq!(changes.files[0].path, "other.txt");
    commit(&f.work, "feature.txt", "changed\n", "Changed reviewed file");
    f.push(&f.alice, "alice", &["feature"]).await;
    let latest = f.open_pull(&f.carol, n).await;
    assert!(!f.services.pulls.review_progress(&f.carol, &latest).await.unwrap().iter().find(|p| p.path == "feature.txt").unwrap().viewed);
    assert!(f.services.pulls.mark_reviewed_file(&f.carol, &latest, &p.path, &ctx.pull.head_sha, &p.fingerprint, true).await.is_err());
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_protection_and_unpublished_threads_cannot_be_bypassed() {
    let f = Fixture::new().await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, n).await;
    let head = f.repo(&f.alice, "alice").await;
    let selection = suggest(&f, &f.alice, &ctx, "feature.txt", 1, "new").await;
    let protected = rbit_services::protection::ProtectionInput {
        pattern: "feature".into(),
        settings: rbit_core::protection::RuleSettings { require_pull_request: true, ..Default::default() },
        users: vec![],
        teams: vec![],
        contexts: vec![],
    };
    f.services.protection.create(&f.alice, &head, &protected).await.unwrap();
    assert!(
        f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, std::slice::from_ref(&selection)).await.is_err(),
        "even repository admins cannot bypass protection while applying suggestions"
    );
    assert_eq!(head.reader.commit_id("feature".into()).await.unwrap(), ctx.pull.head_sha);
    let bob = f.open_pull(&f.bob, n).await;
    let body = "```suggestion\npending\n```";
    let pending = f
        .services
        .pulls
        .comment_on_line(
            &f.bob,
            &bob,
            LineComment {
                expected_head: Some(ctx.pull.head_sha.clone()),
                path: "feature.txt".into(),
                side: ReviewSide::Right,
                line: 1,
                body: body.into(),
                in_reply_to: None,
                start_review: true,
            },
        )
        .await
        .unwrap();
    // Pending comments remain private in the normal review projection.
    assert!(!f.services.pulls.reviews(Some(&f.alice), &ctx).await.unwrap().visible_comments().any(|(c, _)| c.id == pending.id));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_since_review_compares_rewritten_trees_exactly() {
    let f = Fixture::new().await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.carol, n).await;
    let head = f.repo(&f.carol, "alice").await;
    f.services.pulls.submit_review(&f.carol, &ctx, ReviewVerdict::Approve, "").await.unwrap();
    git(&f.work, &["checkout", "-q", "feature"]);
    git(&f.work, &["reset", "--hard", "main"]);
    commit(&f.work, "rewritten.txt", "replacement history\n", "Rewrite feature");
    f.push(&f.alice, "alice", &["+feature"]).await;
    assert!(matches!(f.services.pulls.changes_since_review(&f.carol, &ctx, &head).await, Err(ServiceError::Conflict(_))));
    let current = f.open_pull(&f.carol, n).await;
    let (baseline, changes) = f.services.pulls.changes_since_review(&f.carol, &current, &head).await.unwrap().unwrap();
    assert_eq!(baseline, ctx.pull.head_sha);
    assert!(changes.files.iter().any(|file| file.path == "feature.txt" && file.deletions == 1));
    assert!(changes.files.iter().any(|file| file.path == "rewritten.txt" && file.additions == 1));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_since_review_rechecks_a_fork_that_became_private() {
    let f = Fixture::new().await;
    let number = f.create(&f.bob, "bob", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, number).await;
    let captured = f.repo(&f.alice, "bob").await;
    f.services.pulls.submit_review(&f.alice, &ctx, ReviewVerdict::Approve, "").await.unwrap();
    let fork = f.repo(&f.bob, "bob").await;
    let private = rbit_services::repo_settings::UpdateRepository { visibility: Visibility::Private, ..Default::default() };
    f.services.repo_settings.update(&fork, private).await.unwrap();
    assert!(matches!(f.services.pulls.changes_since_review(&f.alice, &ctx, &captured).await, Err(ServiceError::NotFound)));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_displayed_markers_cannot_attest_a_newer_base_comparison() {
    let f = Fixture::new().await;
    git(&f.work, &["checkout", "-q", "feature"]);
    let intermediate = git(&f.work, &["rev-parse", "HEAD"]);
    commit(&f.work, "feature.txt", "two\n", "Second feature revision");
    f.push(&f.alice, "alice", &["feature"]).await;
    let number = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.carol, number).await;
    let displayed = f.services.pulls.changes(&ctx, DiffLimits::default()).await.unwrap();
    // The base incorporates the first feature commit after the page computes its diff.
    // Its new merge base changes the file's preimage without changing the pull head.
    f.push(&f.alice, "alice", &[&format!("{intermediate}:main")]).await;
    let markers = f.services.pulls.review_progress_for_comparison(&f.carol, &ctx, &displayed).await.unwrap();
    let shown = markers.iter().find(|file| file.path == "feature.txt").unwrap();
    let current = f.open_pull(&f.carol, number).await;
    assert_eq!(current.pull.head_sha, ctx.pull.head_sha);
    let latest = f.services.pulls.review_progress(&f.carol, &current).await.unwrap();
    let now = latest.iter().find(|file| file.path == "feature.txt").unwrap();
    assert_ne!(shown.fingerprint, now.fingerprint, "the marker must describe the displayed preimage");
    assert!(matches!(
        f.services.pulls.mark_reviewed_file(&f.carol, &current, &shown.path, &ctx.pull.head_sha, &shown.fingerprint, true).await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(!f.services.pulls.review_progress(&f.carol, &current).await.unwrap().iter().any(|file| file.viewed));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn review_workflow_delayed_pushes_cannot_rewind_applied_suggestions() {
    let f = Fixture::new().await;
    let n = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, n).await;
    let head = f.repo(&f.alice, "alice").await;
    let selected = suggest(&f, &f.bob, &f.open_pull(&f.bob, n).await, "feature.txt", 1, "new").await;
    let id = f.services.pulls.apply_suggestions(&f.alice, &ctx, &head, &ctx.pull.head_sha, &[selected]).await.unwrap();
    let events = f.events(&f.alice, n).await;
    for after in [Some(ctx.pull.head_sha.clone()), None, Some(id.clone())] {
        let update = RefUpdate { name: "refs/heads/feature".into(), before: Some(ctx.pull.head_sha.clone()), after };
        f.services.pulls.handle_push(head.repo.repo.id, PushSource::User(f.alice.id()), &[update]).await.unwrap();
        assert_eq!(f.open_pull(&f.alice, n).await.pull.head_sha, id);
        assert_eq!(head.reader.commit_id(format!("refs/pull/{n}/head")).await.unwrap(), id);
        assert_eq!(f.events(&f.alice, n).await, events);
    }
    f.services.pulls.shutdown().await;
}
