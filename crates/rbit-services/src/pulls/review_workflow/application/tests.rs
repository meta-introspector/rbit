//! The exact final suggestion writer fence rechecks both delegated capabilities.
use super::*;
use crate::{
    CreateRepository, Services,
    pulls::{LineComment, NewPullInput},
};
use rbit_core::{Config, IssueNumber, Visibility, pull::ReviewSide};
use rbit_identity_models::agents::{Agent, AgentAccess, AgentInput, AgentScope, Capability};
use rbit_storage::{Database, KvStore, NewUser};
use std::{path::Path, process::Command, sync::Arc};

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "user.name=Chase", "-c", "user.email=chase@example.test", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
struct Fixture {
    _dir: tempfile::TempDir,
    services: Services,
    owner: User,
    agent: User,
    record: Agent,
    input: AgentInput,
    pull: PullContext,
    head: RepoContext,
    selected: SuggestionSelection,
}
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.storage.data_dir = dir.path().join("data");
    std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
    let database = Database::open(&config.storage.sqlite_path()).await.unwrap();
    let kv = KvStore::open(&config.storage.kv_path()).unwrap();
    let services = Services::new(Arc::new(config), &database, &kv).await.unwrap();
    let owner = database
        .accounts()
        .create_user(NewUser {
            name: "owner".parse().unwrap(),
            email: "owner@example.test".parse().unwrap(),
            display_name: None,
            password_hash: "test".into(),
            is_admin: false,
        })
        .await
        .unwrap();
    services
        .repositories
        .create(&owner, CreateRepository { name: "repo".into(), description: String::new(), visibility: Visibility::Private })
        .await
        .unwrap();
    let repo = services.repositories.open(Some(&owner), "owner", "repo").await.unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    git(&work, &["init", "-q"]);
    std::fs::write(work.join("README.md"), "base\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "Base"]);
    git(&work, &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:main"]);
    std::fs::write(work.join("README.md"), "feature\n").unwrap();
    git(&work, &["commit", "-qam", "Feature"]);
    git(&work, &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:feature"]);
    let repo = services.repositories.open(Some(&owner), "owner", "repo").await.unwrap();
    let (issue, _) = services
        .pulls
        .create(
            &owner,
            &repo,
            &repo,
            NewPullInput { base_branch: "main".into(), head_branch: "feature".into(), title: "Feature".into(), ..NewPullInput::default() },
        )
        .await
        .unwrap();
    let input = AgentInput {
        slug: "helper".into(),
        name: "Helper".into(),
        capabilities: vec![Capability::Read, Capability::Push, Capability::PullRequests],
        scopes: vec![AgentScope::Repository { id: repo.repo.repo.id.0, access: AgentAccess::Write }],
    };
    let record = services.agents.create(&owner, &input).await.unwrap();
    let agent = database.accounts().find_user(record.account_id).await.unwrap().unwrap();
    let head = services.repositories.open(Some(&agent), "owner", "repo").await.unwrap();
    let pull = services.pulls.open(Some(&agent), head.clone(), IssueNumber::new(issue.number.get()).unwrap()).await.unwrap();
    let body = "```suggestion\nreplacement\n```";
    let comment = services
        .pulls
        .comment_on_line(
            &agent,
            &pull,
            LineComment {
                expected_head: Some(pull.pull.head_sha.clone()),
                path: "README.md".into(),
                side: ReviewSide::Right,
                line: 1,
                body: body.into(),
                in_reply_to: None,
                start_review: false,
            },
        )
        .await
        .unwrap();
    let selected = SuggestionSelection { comment_id: comment.id.0, body_digest: super::super::body_digest(body) };
    Fixture { _dir: dir, services, owner, agent, record, input, pull, head, selected }
}
#[tokio::test]
async fn agent_suggestion_publication_fence_rechecks_push_and_pull_capabilities() {
    let mut f = fixture().await;
    let expected = f.pull.pull.head_sha.clone();
    for removed in [Capability::Push, Capability::PullRequests] {
        f.services.pulls.suggestion_authority(&f.agent, &f.pull, &f.head, &expected).await.unwrap();
        f.input.capabilities.retain(|capability| *capability != removed);
        f.record = f.services.agents.update(&f.owner, f.record.account_id, f.record.revision, &f.input, true).await.unwrap();
        // Invoke the actual final guard after revocation with the captured preflight actor/context.
        assert!(f.services.pulls.suggestion_fence(&f.agent, &f.pull, &f.head, &expected, std::slice::from_ref(&f.selected)).await.is_err());
        if removed == Capability::Push {
            assert!(f.services.protection.authorize_branch_write(&f.head, &f.agent, "feature").await.is_err());
            assert!(f.services.protection.branch_update_refusal(&f.head, &f.agent, "feature").await.is_err());
        }
        assert_eq!(f.head.reader.commit_id("feature".into()).await.unwrap(), expected);
        f.input.capabilities.push(removed);
        f.record = f.services.agents.update(&f.owner, f.record.account_id, f.record.revision, &f.input, true).await.unwrap();
    }
    f.services.shutdown().await;
}
