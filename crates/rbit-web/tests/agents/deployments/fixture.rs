//! Real Git and Actions history for the combined agent/deployment HTTP tests.
use super::super::fixture::Fixture;
use rbit_core::{
    Timestamp,
    actions::{RunEvent, Status, TaskId},
};
use rbit_services::actions::{
    environments::EnvironmentInput,
    runners::{ManagedScope, RegistrationOptions},
};
use rbit_storage::sqlite::{
    actions_deployments::{Attempt, Configuration},
    actions_runs::{NewJob, NewRun, RunStart},
};
use rbit_workflow::protocol::{PollRequest, RegisterRequest};
use std::{path::Path, process::Command};

pub(super) const WORKFLOW: &str = "name: Agent deployment\non:\n  workflow_dispatch:\n    inputs:\n      deployment_sha: {type: string, required: true}\n      deployment_environment: {type: string, required: true}\n      deployment_operation: {type: string, required: true}\njobs:\n  deploy:\n    runs-on: agent-deploy\n    environment: ${{ inputs.deployment_environment }}\n    steps: [{run: echo fixture}]\n";
pub(super) struct Prepared {
    pub sha: String,
    pub source: Attempt,
    pub configuration: Configuration,
}
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.test",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}
pub(super) async fn prepare(f: &Fixture) -> Prepared {
    let repo = f.services.repositories.open(Some(&f.owner), "agent-team", "scoped").await.unwrap();
    f.db.actions_settings().set_enabled(f.repo, true, Timestamp::now()).await.unwrap();
    let work = tempfile::tempdir().unwrap();
    git(work.path(), &["init", "-q"]);
    std::fs::create_dir_all(work.path().join(".forgejo/workflows")).unwrap();
    std::fs::write(work.path().join(".forgejo/workflows/deploy.yml"), WORKFLOW).unwrap();
    git(work.path(), &["add", "."]);
    git(work.path(), &["commit", "-qm", "Trusted fixture workflow"]);
    git(work.path(), &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:main"]);
    let sha = repo.reader.commit_id("refs/heads/main".into()).await.unwrap();
    seed_source(f, &sha).await;
    finish_source(f, &repo).await;
    let actions = f.services.actions.as_ref().unwrap();
    actions
        .put_environment(
            &f.owner,
            &repo,
            "production",
            &EnvironmentInput { reviewers: vec![f.owner.name().as_str().into()], prevent_self_review: true, ..EnvironmentInput::default() },
        )
        .await
        .unwrap();
    let configuration = actions.configure_deployment_workflow(&f.owner, &repo, "production", "staging", "deploy.yml").await.unwrap();
    let source = f.db.actions_deployments().attempts(f.repo, None, 100).await.unwrap().remove(0);
    Prepared { sha, source, configuration }
}

async fn seed_source(f: &Fixture, sha: &str) {
    let mut template = crate::actions_runner_tasks::template("deploy", &["agent-deploy"], &[]);
    template.environment = Some("staging".into());
    let payload = serde_json::to_vec(&template).unwrap();
    let jobs = [NewJob {
        key: "deploy",
        name: "Staging fixture",
        matrix: None,
        needs: &[],
        runs_on: &template.runs_on,
        permissions: None,
        payload: &payload,
        condition: false,
        max_parallel: None,
        group: None,
        environment: Some("staging"),
    }];
    f.db.actions_runs()
        .create_run(&NewRun {
            repo: f.repo,
            workflow_path: ".forgejo/workflows/deploy.yml",
            workflow_name: "Fixture",
            title: "Staging fixture",
            event: RunEvent::Push,
            reference: "refs/heads/main",
            sha,
            actor: Some(f.owner.id()),
            trigger_actor: Some(f.owner.id()),
            pull_number: None,
            is_fork: false,
            trusted: true,
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
}

async fn finish_source(f: &Fixture, repo: &rbit_services::RepoContext) {
    let actions = f.services.actions.as_ref().unwrap();
    let registration =
        actions.create_registration_token(&f.owner, ManagedScope::Repository(repo), RegistrationOptions::default()).await.unwrap();
    let request: RegisterRequest = serde_json::from_value(crate::actions_runner::register_body(
        registration.secret.expose(),
        "agent-deploy-runner",
        &["agent-deploy"],
    ))
    .unwrap();
    let registered = actions.register_runner(&request).await.unwrap();
    let runner = actions.authenticate_runner(registered.credential.expose()).await.unwrap().unwrap();
    actions.bump_tasks_version();
    let task = actions.poll(&runner, &PollRequest { free_slots: 1 }, Some("agent-deployment-source")).await.unwrap().unwrap();
    f.db.actions_runs().finish_task(TaskId(task.task_id.0), Status::Success, Timestamp::now()).await.unwrap();
}
