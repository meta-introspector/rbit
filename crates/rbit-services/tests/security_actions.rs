//! Live Actions grants upload explicit targets only through the configured default workflow.
use crate::actions_fixture::Fixture;
use rbit_core::{
    PushSource, Timestamp,
    actions::{DefaultTokenPermission, PermissionLevel, Status, TokenPermissions},
};
use rbit_services::{
    actions::tokens::TaskGrant,
    security::{Settings, Uploader},
};
const WORKFLOW: &str = "name: Security\non: push\njobs:\n  scan:\n    runs-on: x\n    steps: [{run: echo scan}]\n";
fn report() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"scanner"}},"invocations":[{"executionSuccessful":true}],"results":[]}]})).unwrap()
}
#[tokio::test]
async fn security_live_trusted_actions_can_report_other_exact_commits_but_stale_or_stopped_runs_cannot() {
    let f = Fixture::new().await;
    let target = f.commit(&[("README.md", "base")], "Base");
    f.push_ref("refs/heads/main").await;
    f.commit(&[(".rbit/workflows/security.yml", WORKFLOW)], "Security workflow");
    let update = f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.alice).await;
    f.services
        .security
        .configure(
            &f.alice,
            &repo,
            Settings {
                scanner: "scanner".into(),
                workflow: ".rbit/workflows/security.yml".into(),
                require_scan: true,
                minimum_severity: 3,
            },
        )
        .await
        .unwrap();
    f.database.actions_settings().set_token_permission(repo.repo.repo.id, DefaultTokenPermission::Write, Timestamp::now()).await.unwrap();
    let run = f.actions().handle_push(repo.repo.repo.id, PushSource::User(f.alice.id()), &[update]).await.unwrap().remove(0);
    let claimed = f.claim(&["x"], 17).await.unwrap();
    let grant = TaskGrant {
        task: claimed.task.id,
        job: claimed.job.id,
        run: run.id,
        repo: repo.repo.repo.id,
        permissions: TokenPermissions::uniform(PermissionLevel::Write),
    };
    let scan = f.services.security.begin(Uploader::Task(&grant), &repo, &target).await.unwrap();
    assert_eq!(scan.commit_sha, target, "trusted default workflow can scan exact PR head/base target, not just its own execution commit");
    f.services.security.finish(Uploader::Task(&grant), &repo, scan.id, report()).await.unwrap();
    let pending = f.services.security.begin(Uploader::Task(&grant), &repo, &target).await.unwrap();
    f.database.actions_settings().set_token_permission(repo.repo.repo.id, DefaultTokenPermission::Read, Timestamp::now()).await.unwrap();
    assert!(
        f.services.security.finish(Uploader::Task(&grant), &repo, pending.id, report()).await.is_err(),
        "captured grant cannot retain a revoked repository default at upload completion"
    );
    f.database.actions_settings().set_token_permission(repo.repo.repo.id, DefaultTokenPermission::Write, Timestamp::now()).await.unwrap();
    let mut owner = f.database.actions_settings().owner_settings(f.alice.id()).await.unwrap();
    owner.policies.token_permission = DefaultTokenPermission::Read;
    f.database.actions_settings().set_owner_settings(f.alice.id(), &owner, Timestamp::now()).await.unwrap();
    assert!(
        f.services.security.finish(Uploader::Task(&grant), &repo, pending.id, report()).await.is_err(),
        "the current owner's ceiling also narrows a previously captured grant"
    );
    owner.policies.token_permission = DefaultTokenPermission::Write;
    f.database.actions_settings().set_owner_settings(f.alice.id(), &owner, Timestamp::now()).await.unwrap();
    f.services.security.finish(Uploader::Task(&grant), &repo, pending.id, report()).await.unwrap();
    let no_write = TaskGrant { permissions: TokenPermissions::uniform(PermissionLevel::Read), ..grant };
    assert!(f.services.security.begin(Uploader::Task(&no_write), &repo, &target).await.is_err());
    f.commit(&[("README.md", "changed")], "Move default branch");
    f.push_ref("refs/heads/main").await;
    assert!(
        f.services.security.begin(Uploader::Task(&grant), &repo, &target).await.is_err(),
        "stale default workflow cannot attest after the trust tip moves"
    );
    f.database.actions_runs().finish_task(grant.task, Status::Success, Timestamp::now()).await.unwrap();
    assert!(f.services.security.begin(Uploader::Task(&grant), &repo, &target).await.is_err());
    f.services.shutdown().await;
}

#[tokio::test]
async fn security_fork_pull_and_reusable_tasks_cannot_attest_even_with_captured_write_grants() {
    use rbit_core::actions::RunEvent;
    use rbit_storage::sqlite::actions_runs::{NewJob, NewRun, RunStart};
    let f = Fixture::new().await;
    let sha = f.commit(&[(".rbit/workflows/security.yml", WORKFLOW)], "Trusted workflow");
    f.push_ref("refs/heads/main").await;
    let repo = f.repo(&f.alice).await;
    f.services
        .security
        .configure(
            &f.alice,
            &repo,
            Settings {
                scanner: "scanner".into(),
                workflow: ".rbit/workflows/security.yml".into(),
                require_scan: true,
                minimum_severity: 3,
            },
        )
        .await
        .unwrap();
    let permissions = serde_json::to_string(&TokenPermissions::uniform(PermissionLevel::Write)).unwrap();
    for (index, (fork, event, key, trusted)) in [
        (true, RunEvent::Push, "scan", true),
        (false, RunEvent::PullRequest, "scan", true),
        (false, RunEvent::Push, "caller/scan", true),
        (false, RunEvent::Push, "scan", false),
    ]
    .into_iter()
    .enumerate()
    {
        let jobs = [NewJob {
            key,
            name: "scan",
            matrix: None,
            needs: &[],
            runs_on: &["x".into()],
            permissions: Some(&permissions),
            payload: b"{}",
            condition: false,
            max_parallel: None,
            group: None,
            environment: None,
        }];
        let run = f
            .database
            .actions_runs()
            .create_run(&NewRun {
                repo: repo.repo.repo.id,
                workflow_path: ".rbit/workflows/security.yml",
                workflow_name: "Security",
                title: "provenance fixture",
                event,
                reference: "refs/heads/main",
                sha: &sha,
                actor: Some(f.alice.id()),
                trigger_actor: Some(f.alice.id()),
                pull_number: None,
                is_fork: fork,
                trusted,
                concurrency_group: None,
                cancel_in_progress: false,
                event_payload: "{}",
                dispatch_inputs: None,
                start: RunStart::Runnable,
                jobs: &jobs,
                error: None,
                now: Timestamp::now(),
            })
            .await
            .unwrap();
        let task = f.claim(&["x"], u8::try_from(index + 50).unwrap()).await.unwrap();
        let grant = TaskGrant {
            task: task.task.id,
            job: task.job.id,
            run: run.id,
            repo: repo.repo.repo.id,
            permissions: TokenPermissions::uniform(PermissionLevel::Write),
        };
        assert!(
            f.services.security.begin(Uploader::Task(&grant), &repo, &sha).await.is_err(),
            "case {index} must not attest clean evidence"
        );
        f.database.actions_runs().finish_task(grant.task, Status::Success, Timestamp::now()).await.unwrap();
    }
    f.services.shutdown().await;
}
