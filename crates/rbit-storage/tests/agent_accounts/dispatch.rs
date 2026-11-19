//! Last-moment dispatch admission cannot reuse a revoked scope or capability snapshot.
use super::{AgentAccess, AgentScope, Capability, Database, Timestamp, Visibility, input, repo, user};
use rbit_core::{AccountId, RepoId, actions::RunEvent};
use rbit_storage::sqlite::actions_runs::{NewRun, RunStart};

fn dispatch(repo: RepoId, actor: AccountId) -> NewRun<'static> {
    NewRun {
        repo,
        workflow_path: ".rbit/workflows/ci.yml",
        workflow_name: "CI",
        title: "Manual run",
        event: RunEvent::WorkflowDispatch,
        reference: "refs/heads/main",
        sha: "1111111111111111111111111111111111111111",
        actor: Some(actor),
        trigger_actor: Some(actor),
        pull_number: None,
        is_fork: false,
        trusted: true,
        concurrency_group: None,
        cancel_in_progress: false,
        event_payload: "{}",
        dispatch_inputs: None,
        start: RunStart::Runnable,
        jobs: &[],
        error: Some("fixture workflow without jobs"),
        now: Timestamp(100),
    }
}

#[tokio::test]
async fn agent_manual_publication_rechecks_current_scope_and_dispatch_capability() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let repository = repo(&db, owner.id(), "repo", Visibility::Private).await;
    let mut settings = input(AgentScope::Repository { id: repository.0, access: AgentAccess::Write });
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    let request = dispatch(repository, agent.account_id);
    let runs = db.actions_runs();
    let refused = runs.create_run(&request).await.unwrap_err();
    assert!(refused.to_string().contains("agent dispatch delegation revoked"));
    settings.capabilities.push(Capability::Dispatch);
    let agent = store.update(owner.id(), agent.account_id, agent.revision, &settings, true).await.unwrap();
    let admitted = runs.create_run(&request).await.unwrap();
    assert_eq!(admitted.number, 1, "refused admission rolls back the run number too");
    settings.capabilities.retain(|capability| *capability != Capability::Dispatch);
    let agent = store.update(owner.id(), agent.account_id, agent.revision, &settings, true).await.unwrap();
    assert!(runs.create_run(&request).await.is_err(), "earlier request cannot retain removed capability");
    settings.capabilities.push(Capability::Dispatch);
    settings.scopes = vec![AgentScope::Repository { id: repository.0, access: AgentAccess::Read }];
    let agent = store.update(owner.id(), agent.account_id, agent.revision, &settings, true).await.unwrap();
    assert!(runs.create_run(&request).await.is_err(), "dispatch cannot expand a read-only scope");
    store.disable(owner.id(), agent.account_id, agent.revision).await.unwrap();
    assert!(runs.create_run(&request).await.is_err());
    let human = runs.create_run(&dispatch(repository, owner.id())).await.unwrap();
    assert_eq!(human.number, 2, "refused agent publications leave no partial runs");
}
