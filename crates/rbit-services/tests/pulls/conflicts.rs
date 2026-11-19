//! Conflict resolution against real branches, current authority and exact snapshots.
use super::*;
use rbit_core::pull::conflicts::Resolution;
use rbit_services::pulls::conflicts::ResolveConflicts;

async fn conflicted(f: &Fixture) -> u32 {
    git(&f.work, &["checkout", "-q", "feature"]);
    commit(&f.work, "README.md", "source\n", "Source conflict");
    f.push(&f.alice, "alice", &["feature"]).await;
    git(&f.work, &["checkout", "-q", "main"]);
    commit(&f.work, "README.md", "target\n", "Target conflict");
    f.push(&f.alice, "alice", &["main"]).await;
    f.create(&f.alice, "alice", "feature").await.unwrap()
}

#[tokio::test]
async fn browser_conflicts_publish_two_parents_and_immediate_projection() {
    let f = Fixture::new().await;
    let n = conflicted(&f).await;
    let ctx = f.open_pull(&f.carol, n).await;
    let head = f.repo(&f.carol, "alice").await;
    let preview = f.services.pulls.preview_conflicts(&f.carol, &ctx, &head).await.unwrap();
    assert_eq!(preview.files.len(), 1);
    assert_eq!(preview.files[0].source, "source\n");
    let input = ResolveConflicts {
        expected_head: preview.head_sha.clone(),
        expected_base: preview.base_sha.clone(),
        resolutions: vec![Resolution { path: "README.md".into(), content: "resolved\n".into() }],
    };
    let id = f.services.pulls.resolve_conflicts(&f.carol, &ctx, &head, input.clone()).await.unwrap();
    assert_eq!(f.open_pull(&f.carol, n).await.pull.head_sha, id);
    assert_eq!(git(&head.disk_path, &["rev-parse", &format!("{id}^1")]), preview.head_sha);
    assert_eq!(git(&head.disk_path, &["rev-parse", &format!("{id}^2")]), preview.base_sha);
    assert_eq!(head.reader.blob(id, "README.md".into(), 100).await.unwrap().text(), Some("resolved\n"));
    assert!(matches!(f.services.pulls.resolve_conflicts(&f.carol, &ctx, &head, input).await, Err(ServiceError::Conflict(_))));
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn browser_conflicts_refuse_stale_target_and_revoked_source_writer() {
    let f = Fixture::new().await;
    let n = conflicted(&f).await;
    let ctx = f.open_pull(&f.carol, n).await;
    let head = f.repo(&f.carol, "alice").await;
    let preview = f.services.pulls.preview_conflicts(&f.carol, &ctx, &head).await.unwrap();
    let mut input = ResolveConflicts {
        expected_head: preview.head_sha,
        expected_base: preview.base_sha,
        resolutions: vec![Resolution { path: "README.md".into(), content: "resolved\n".into() }],
    };
    commit(&f.work, "another.txt", "later\n", "Move target");
    f.push(&f.alice, "alice", &["main"]).await;
    assert!(matches!(f.services.pulls.resolve_conflicts(&f.carol, &ctx, &head, input.clone()).await, Err(ServiceError::Conflict(_))));
    input.expected_base = head.reader.commit_id("refs/heads/main".into()).await.unwrap();
    let owner = f.repo(&f.alice, "alice").await;
    f.services.collaborators.remove(&f.alice, &owner, "carol").await.unwrap();
    assert!(matches!(f.services.pulls.resolve_conflicts(&f.carol, &ctx, &head, input).await, Err(ServiceError::Forbidden)));
    assert_eq!(head.reader.commit_id("refs/heads/feature".into()).await.unwrap(), ctx.pull.head_sha);
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn browser_conflicts_refuse_extra_paths_and_read_only_source_context() {
    let f = Fixture::new().await;
    let n = conflicted(&f).await;
    let ctx = f.open_pull(&f.alice, n).await;
    let mut head = f.repo(&f.alice, "alice").await;
    let preview = f.services.pulls.preview_conflicts(&f.alice, &ctx, &head).await.unwrap();
    let input = ResolveConflicts {
        expected_head: preview.head_sha,
        expected_base: preview.base_sha,
        resolutions: vec![Resolution { path: "other.txt".into(), content: "injected\n".into() }],
    };
    assert!(matches!(f.services.pulls.resolve_conflicts(&f.alice, &ctx, &head, input).await, Err(ServiceError::Invalid(_))));
    head.access = rbit_core::RepoAccess::Read;
    assert!(matches!(f.services.pulls.preview_conflicts(&f.alice, &ctx, &head).await, Err(ServiceError::Forbidden)));
    assert_eq!(head.reader.commit_id("refs/heads/feature".into()).await.unwrap(), ctx.pull.head_sha);
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn browser_conflicts_enforce_source_protection_without_admin_bypass() {
    let f = Fixture::new().await;
    let n = conflicted(&f).await;
    let ctx = f.open_pull(&f.alice, n).await;
    let head = f.repo(&f.alice, "alice").await;
    let preview = f.services.pulls.preview_conflicts(&f.alice, &ctx, &head).await.unwrap();
    let protected = rbit_services::protection::ProtectionInput {
        pattern: "feature".into(),
        settings: rbit_core::protection::RuleSettings { require_pull_request: true, ..Default::default() },
        users: vec![],
        teams: vec![],
        contexts: vec![],
    };
    f.services.protection.create(&f.alice, &head, &protected).await.unwrap();
    let input = ResolveConflicts {
        expected_head: preview.head_sha,
        expected_base: preview.base_sha,
        resolutions: vec![Resolution { path: "README.md".into(), content: "resolved\n".into() }],
    };
    assert!(matches!(f.services.pulls.resolve_conflicts(&f.alice, &ctx, &head, input).await, Err(ServiceError::Invalid(_))));
    assert_eq!(head.reader.commit_id("refs/heads/feature".into()).await.unwrap(), ctx.pull.head_sha);
    f.services.pulls.shutdown().await;
}

#[tokio::test]
async fn browser_conflicts_fork_writer_resolves_without_granting_base_writers_access() {
    let f = Fixture::new().await;
    git(&f.work, &["checkout", "-q", "feature"]);
    commit(&f.work, "README.md", "fork\n", "Fork source");
    f.push(&f.bob, "bob", &["feature"]).await;
    git(&f.work, &["checkout", "-q", "main"]);
    commit(&f.work, "README.md", "base\n", "Base target");
    f.push(&f.alice, "alice", &["main"]).await;
    let n = f.create(&f.bob, "bob", "feature").await.unwrap();
    let alice = f.open_pull(&f.alice, n).await;
    let fork = f.repo(&f.alice, "bob").await;
    assert!(matches!(f.services.pulls.preview_conflicts(&f.alice, &alice, &fork).await, Err(ServiceError::Forbidden)));
    let ctx = f.open_pull(&f.bob, n).await;
    let fork = f.repo(&f.bob, "bob").await;
    let preview = f.services.pulls.preview_conflicts(&f.bob, &ctx, &fork).await.unwrap();
    let input = ResolveConflicts {
        expected_head: preview.head_sha,
        expected_base: preview.base_sha,
        resolutions: vec![Resolution { path: "README.md".into(), content: "resolved\n".into() }],
    };
    let id = f.services.pulls.resolve_conflicts(&f.bob, &ctx, &fork, input).await.unwrap();
    let updated = f.open_pull(&f.alice, n).await;
    assert_eq!(updated.pull.head_sha, id);
    assert_eq!(updated.repo().reader.blob(id, "README.md".into(), 100).await.unwrap().text(), Some("resolved\n"));
    f.services.pulls.shutdown().await;
}

#[cfg(unix)]
mod publication;
