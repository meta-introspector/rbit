//! Real repositories prove trusted-base ownership, routing and merge enforcement.
use super::*;
use rbit_core::{protection::RuleSettings, pull::Requirement};
use rbit_services::protection::ProtectionInput;

async fn protect(fixture: &Fixture) {
    let repo = fixture.repo(&fixture.alice, "alice").await;
    let input = ProtectionInput {
        pattern: "main".into(),
        settings: RuleSettings { require_pull_request: true, require_code_owner_reviews: true, ..RuleSettings::default() },
        users: Vec::new(),
        teams: Vec::new(),
        contexts: Vec::new(),
    };
    fixture.services.protection.create(&fixture.alice, &repo, &input).await.unwrap();
}

async fn source(fixture: &Fixture, contents: &str) {
    git(&fixture.work, &["checkout", "-q", "main"]);
    commit(&fixture.work, "CODEOWNERS", contents, "Set owners");
    fixture.push(&fixture.alice, "alice", &["main"]).await;
}

async fn missing(fixture: &Fixture, number: u32) -> usize {
    let ctx = fixture.open_pull(&fixture.alice, number).await;
    let state = fixture.services.pulls.merge_readiness(Some(&fixture.alice), &ctx, None).await.unwrap();
    state
        .evaluation
        .checks
        .into_iter()
        .find_map(|check| match check.requirement {
            Requirement::CodeOwners { missing } => Some(missing),
            _ => None,
        })
        .expect("code owner requirement")
}

#[tokio::test]
async fn trusted_base_requests_owner_and_blocks_direct_and_queued_merge() {
    let fixture = Fixture::new().await;
    source(&fixture, "* @carol\n").await;
    protect(&fixture).await;
    // A malicious head tries to replace its reviewer; ownership still comes from main.
    git(&fixture.work, &["checkout", "-q", "feature"]);
    commit(&fixture.work, "CODEOWNERS", "* @bob\n", "Replace reviewers");
    fixture.push(&fixture.alice, "alice", &["feature"]).await;
    let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
    let pulls = &fixture.services.pulls;
    let ctx = fixture.open_pull(&fixture.alice, number).await;
    assert_eq!(fixture.database.reviews().requested(ctx.issue().id).await.unwrap(), vec![fixture.carol.id()]);
    assert_eq!(missing(&fixture, number).await, 1);
    assert!(pulls.enqueue(&fixture.alice, &ctx, MergeStrategy::Merge, None).await.is_err());
    assert!(pulls.merge(&fixture.alice, &ctx, Fixture::merge_input(MergeStrategy::Merge)).await.is_err());
    let carol = fixture.open_pull(&fixture.carol, number).await;
    pulls.submit_review(&fixture.carol, &carol, ReviewVerdict::Approve, "Reviewed").await.unwrap();
    assert_eq!(missing(&fixture, number).await, 0);
    // Removing write access removes approval authority immediately.
    fixture.services.collaborators.remove(&fixture.alice, ctx.repo(), "carol").await.unwrap();
    assert_eq!(missing(&fixture, number).await, 1);
    fixture.services.shutdown().await;
}

#[tokio::test]
async fn malformed_rules_and_unresolved_owners_fail_closed() {
    for contents in ["[a-z] @carol\n", "* @missing-user\n", "* @alice\n"] {
        let fixture = Fixture::new().await;
        source(&fixture, contents).await;
        protect(&fixture).await;
        let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
        assert!(missing(&fixture, number).await > 0, "{contents}");
        fixture.services.shutdown().await;
    }
}

#[tokio::test]
async fn ownership_changes_on_base_are_rechecked() {
    let fixture = Fixture::new().await;
    source(&fixture, "* @carol\n").await;
    protect(&fixture).await;
    let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
    let carol = fixture.open_pull(&fixture.carol, number).await;
    fixture.services.pulls.submit_review(&fixture.carol, &carol, ReviewVerdict::Approve, "").await.unwrap();
    assert_eq!(missing(&fixture, number).await, 0);
    source(&fixture, "* @nobody\n").await;
    assert_eq!(missing(&fixture, number).await, 1);
    fixture.services.shutdown().await;
}

#[tokio::test]
async fn approved_owned_change_merges_and_suspension_revokes_approval() {
    let fixture = Fixture::new().await;
    source(&fixture, "* @carol\n").await;
    protect(&fixture).await;
    let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
    let pulls = &fixture.services.pulls;
    pulls.check_unknown().await.unwrap();
    let ctx = fixture.open_pull(&fixture.alice, number).await;
    let refused = pulls.merge(&fixture.alice, &ctx, Fixture::merge_input(MergeStrategy::Merge)).await.unwrap_err();
    assert!(matches!(refused, MergeFailure::Blocked(ref blocked) if blocked.code == "merge_requires_code_owners"));
    let carol = fixture.open_pull(&fixture.carol, number).await;
    pulls.submit_review(&fixture.carol, &carol, ReviewVerdict::Approve, "Approved").await.unwrap();
    fixture.database.admin().set_suspended(fixture.carol.id(), true).await.unwrap();
    assert_eq!(missing(&fixture, number).await, 1);
    fixture.database.admin().set_suspended(fixture.carol.id(), false).await.unwrap();
    assert_eq!(missing(&fixture, number).await, 0);
    pulls.merge(&fixture.alice, &ctx, Fixture::merge_input(MergeStrategy::Merge)).await.unwrap();
    assert!(fixture.open_pull(&fixture.alice, number).await.pull.is_merged());
    fixture.services.shutdown().await;
}

#[tokio::test]
async fn ownership_covers_large_files_and_both_sides_of_renames() {
    let fixture = Fixture::new().await;
    source(&fixture, "/feature.txt @carol\n/large.bin @carol\n").await;
    protect(&fixture).await;
    git(&fixture.work, &["checkout", "-q", "feature"]);
    git(&fixture.work, &["mv", "feature.txt", "renamed.txt"]);
    commit(&fixture.work, "large.bin", &"x".repeat(600_000), "Move and grow");
    fixture.push(&fixture.alice, "alice", &["feature"]).await;
    let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
    assert_eq!(missing(&fixture, number).await, 1);
    let carol = fixture.open_pull(&fixture.carol, number).await;
    fixture.services.pulls.submit_review(&fixture.carol, &carol, ReviewVerdict::Approve, "").await.unwrap();
    assert_eq!(missing(&fixture, number).await, 0, "large content must not make the path listing incomplete");
    fixture.services.shutdown().await;
}

#[tokio::test]
async fn team_ownership_follows_current_membership() {
    use rbit_core::OrgRole;
    use rbit_services::{orgs::CreateOrganization, teams::TeamInput};
    let fixture = Fixture::new().await;
    let services = &fixture.services;
    let org = services.orgs.create(&fixture.alice, CreateOrganization { name: "acme".into(), ..Default::default() }).await.unwrap();
    services.orgs.add_member(&fixture.alice, &org, "carol", OrgRole::Member).await.unwrap();
    services
        .repositories
        .create_in(
            &fixture.alice,
            "acme",
            CreateRepository { name: "owned".into(), description: String::new(), visibility: Visibility::Private },
        )
        .await
        .unwrap();
    let repo = services.repositories.open(Some(&fixture.alice), "acme", "owned").await.unwrap();
    services.teams.create(&fixture.alice, &org, TeamInput { name: "core.team".into(), description: String::new() }).await.unwrap();
    let team = services.teams.open(Some(&fixture.alice), &org, "core.team").await.unwrap();
    services.teams.add_member(&fixture.alice, &team, "carol").await.unwrap();
    services.teams.set_repository(&fixture.alice, &team, "owned", Permission::Write).await.unwrap();
    git(&fixture.work, &["checkout", "-q", "main"]);
    commit(&fixture.work, "CODEOWNERS", "* @acme/core.team\n", "Set team owners");
    git(&fixture.work, &["push", "-q", repo.disk_path.to_str().unwrap(), "main", "feature"]);
    services
        .protection
        .create(
            &fixture.alice,
            &repo,
            &ProtectionInput {
                pattern: "main".into(),
                settings: RuleSettings { require_pull_request: true, require_code_owner_reviews: true, ..Default::default() },
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let (issue, _) = services
        .pulls
        .create(
            &fixture.alice,
            &repo,
            &repo,
            NewPullInput { base_branch: "main".into(), head_branch: "feature".into(), ..Default::default() },
        )
        .await
        .unwrap();
    assert_eq!(fixture.database.reviews().requested(issue.id).await.unwrap(), vec![fixture.carol.id()]);
    let carol_repo = services.repositories.open(Some(&fixture.carol), "acme", "owned").await.unwrap();
    let ctx = services.pulls.open(Some(&fixture.carol), carol_repo, issue.number).await.unwrap();
    services.pulls.submit_review(&fixture.carol, &ctx, ReviewVerdict::Approve, "").await.unwrap();
    let alice_ctx = services.pulls.open(Some(&fixture.alice), repo, issue.number).await.unwrap();
    let before = services.pulls.merge_readiness(Some(&fixture.alice), &alice_ctx, None).await.unwrap();
    assert!(before.evaluation.checks.iter().any(|check| matches!(check.requirement, Requirement::CodeOwners { missing: 0 })));
    services.teams.remove_member(&fixture.alice, &team, "carol").await.unwrap();
    let after = services.pulls.merge_readiness(Some(&fixture.alice), &alice_ctx, None).await.unwrap();
    assert!(after.evaluation.checks.iter().any(|check| matches!(check.requirement, Requirement::CodeOwners { missing: 1 })));
    services.shutdown().await;
}

#[tokio::test]
async fn unrelated_histories_never_certify_complete_ownership_paths() {
    let fixture = Fixture::new().await;
    let repo = fixture.repo(&fixture.alice, "alice").await;
    git(&fixture.work, &["checkout", "-q", "--orphan", "unrelated"]);
    commit(&fixture.work, "another.txt", "new root", "Unrelated history");
    git(&fixture.work, &["push", "-q", repo.disk_path.to_str().unwrap(), "unrelated"]);
    let (_, incomplete) = repo.reader.changed_paths("main".into(), "unrelated".into(), 100).await.unwrap();
    assert!(incomplete);
    fixture.services.shutdown().await;
}

#[tokio::test]
async fn queue_rechecks_owner_authority_after_candidate_checks_pass() {
    use rbit_services::statuses::NewStatus;
    use std::time::Duration;
    let fixture = Fixture::new().await;
    source(&fixture, "* @carol\n").await;
    let repo = fixture.repo(&fixture.alice, "alice").await;
    let before = repo.reader.commit_id("refs/heads/main".into()).await.unwrap();
    fixture
        .services
        .protection
        .create(
            &fixture.alice,
            &repo,
            &ProtectionInput {
                pattern: "main".into(),
                settings: RuleSettings {
                    require_pull_request: true,
                    require_code_owner_reviews: true,
                    require_status_checks: true,
                    ..Default::default()
                },
                contexts: vec!["ci/test".into()],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let number = fixture.create(&fixture.alice, "alice", "feature").await.unwrap();
    let pulls = &fixture.services.pulls;
    pulls.check_unknown().await.unwrap();
    let carol = fixture.open_pull(&fixture.carol, number).await;
    pulls.submit_review(&fixture.carol, &carol, ReviewVerdict::Approve, "Approved").await.unwrap();
    let ctx = fixture.open_pull(&fixture.alice, number).await;
    pulls.spawn_background();
    pulls.enqueue(&fixture.alice, &ctx, MergeStrategy::Merge, Some(&ctx.pull.head_sha)).await.unwrap();
    let testing = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let entry = fixture.database.actions_runs().queue_of_pull(ctx.issue().id).await.unwrap().unwrap();
            if entry.state == "testing" {
                break entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("candidate reaches checks");
    fixture.services.collaborators.remove(&fixture.alice, &repo, "carol").await.unwrap();
    let success = NewStatus { state: "success".into(), context: Some("ci/test".into()), ..Default::default() };
    fixture.services.statuses.create(&fixture.alice, &repo, &testing.candidate_sha.unwrap(), &success).await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let entry = fixture.database.actions_runs().queue_of_pull(ctx.issue().id).await.unwrap().unwrap();
            if entry.state != "testing" {
                break entry;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("candidate settles after checks");
    assert_eq!(ended.state, "failed");
    assert!(ended.reason.as_deref().is_some_and(|reason| reason.contains("owner")), "{ended:?}");
    assert_eq!(repo.reader.commit_id("refs/heads/main".into()).await.unwrap(), before);
    assert!(!fixture.open_pull(&fixture.alice, number).await.pull.is_merged());
    fixture.services.shutdown().await;
}
