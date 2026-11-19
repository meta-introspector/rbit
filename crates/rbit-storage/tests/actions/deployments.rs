//! Completed attempt history survives reruns and retention; stale tasks cannot rewrite it.
use super::*;

#[tokio::test]
async fn deployment_attempt_history_is_immutable_and_task_urls_are_attempt_scoped() {
    let fx = Fixture::new().await;
    let store = fx.db.actions_runs();
    let history = fx.db.actions_deployments();
    let (none, linux) = (Vec::new(), strings(&["linux"]));
    let jobs = [NewJob { environment: Some("staging"), ..job("deploy", &none, &linux) }];
    let run = store.create_run(&new_run(fx.rocket, &jobs, RunStart::Runnable)).await.unwrap();
    let scope = RunnerScope::Repository(fx.rocket);
    let (runner, labels) = fx.runner(scope, &["linux"], 90).await;
    let first = store.claim(&claim(runner, scope, &labels, &[91; 32])).await.unwrap().unwrap();
    store.set_environment_url(first.task.id, "https://staging.example/v1").await.unwrap();
    store.finish_task(first.task.id, Status::Success, NOW).await.unwrap();
    let saved = history.attempts(fx.rocket, None, 100).await.unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].run_number, saved[0].job_attempt, saved[0].status), (run.number, 1, Status::Success));
    assert_eq!(saved[0].environment_url.as_deref(), Some("https://staging.example/v1"));
    assert!(history.attempt(fx.other, saved[0].id).await.unwrap().is_none());

    store.rerun(fx.rocket, run.id, Rerun::All, RunStart::Runnable, NOW, None).await.unwrap().unwrap();
    let second = store.claim(&claim(runner, scope, &labels, &[92; 32])).await.unwrap().unwrap();
    assert_eq!(second.job.attempt, 2);
    store.set_environment_url(first.task.id, "https://stale.example").await.unwrap();
    assert!(store.jobs(run.id).await.unwrap()[0].environment_url.is_none());
    store.set_environment_url(second.task.id, "https://staging.example/v2").await.unwrap();
    store.finish_task(second.task.id, Status::Failure, NOW).await.unwrap();
    store.set_environment_url(second.task.id, "https://late.example").await.unwrap();
    let attempts = history.attempts(fx.rocket, None, 100).await.unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!((attempts[0].job_attempt, attempts[0].status), (2, Status::Failure));
    assert_eq!(attempts[0].environment_url.as_deref(), Some("https://staging.example/v2"));
    assert_eq!(attempts[1].environment_url.as_deref(), Some("https://staging.example/v1"));
    assert_eq!(history.attempts(fx.rocket, Some(attempts[0].id), 100).await.unwrap().len(), 1);

    let connection =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(fx.dir.path().join("rbit.db"))).await.unwrap();
    assert!(
        sqlx::query("UPDATE actions_job_attempt_history SET status = 2 WHERE id = ?").bind(saved[0].id).execute(&connection).await.is_err()
    );
    store.delete_finished_before(Timestamp(NOW.0 + 1)).await.unwrap();
    assert!(store.run(run.id).await.unwrap().is_none());
    assert_eq!(history.attempts(fx.rocket, None, 100).await.unwrap().len(), 2);
    // SQLite can reuse mutable run/job ids after retention. A new run number is a new history identity.
    let next = store.create_run(&new_run(fx.rocket, &jobs, RunStart::Runnable)).await.unwrap();
    assert!(next.number > run.number);
    let third = store.claim(&claim(runner, scope, &labels, &[93; 32])).await.unwrap().unwrap();
    store.finish_task(third.task.id, Status::Success, NOW).await.unwrap();
    assert_eq!(history.attempts(fx.rocket, None, 100).await.unwrap().len(), 3);
    connection.close().await;
}

// Actions: environments.
#[tokio::test]
async fn jobs_of_a_protected_environment_wait_for_a_reviewer() {
    use rbit_core::actions::{BranchPolicy, ReviewDecision};
    use rbit_storage::sqlite::actions_environments::EnvironmentChanges;
    let fx = Fixture::new().await;
    let store = fx.db.actions_runs();
    let environments = fx.db.actions_environments();
    let none: Vec<String> = Vec::new();
    let linux = strings(&["linux"]);
    let build = strings(&["build"]);
    let jobs = [
        job("build", &none, &linux),
        NewJob { environment: Some("Production"), ..job("deploy", &build, &linux) },
        NewJob { environment: Some("preview"), ..job("preview", &none, &linux) },
    ];
    let run = store.create_run(&new_run(fx.rocket, &jobs, RunStart::Runnable)).await.unwrap();
    // Named for the first time, the environments exist, unprotected.
    let names: Vec<_> = environments.list(fx.rocket).await.unwrap().into_iter().map(|environment| environment.name).collect();
    assert_eq!(names, ["preview", "Production"]);
    let changes = EnvironmentChanges { reviewers: &[fx.bob], prevent_self_review: false, branch_policy: &BranchPolicy::All, wait_timer: 0 };
    environments.put(fx.rocket, "production", &changes, NOW).await.unwrap();

    let scope = RunnerScope::Repository(fx.rocket);
    let (runner, labels) = fx.runner(scope, &["linux"], 1).await;
    let mut seed = 10u8;
    let mut claim_one = || {
        seed += 1;
        [seed; 32]
    };
    // `build` and the unprotected `preview` run; `deploy` waits for the reviewer.
    for _ in 0..2 {
        let claimed = store.claim(&claim(runner, scope, &labels, &claim_one())).await.unwrap().unwrap();
        store.finish_task(claimed.task.id, Status::Success, NOW).await.unwrap();
    }
    let deploy = store.jobs(run.id).await.unwrap().into_iter().find(|job| job.key == "deploy").unwrap();
    assert_eq!(
        (deploy.status, deploy.awaiting_review, deploy.environment.as_deref()),
        (Status::AwaitingApproval, true, Some("Production"))
    );
    assert_eq!(store.run(run.id).await.unwrap().unwrap().status, Status::Waiting, "the run is not held, only the job");
    assert!(store.claim(&claim(runner, scope, &labels, &claim_one())).await.unwrap().is_none(), "no runner takes it");

    let approved = store.review_jobs(run.id, &[deploy.id], fx.bob, ReviewDecision::Approved, "ship it", NOW).await.unwrap().unwrap();
    assert_eq!(approved.now_waiting, [deploy.id]);
    assert!(store.review_jobs(run.id, &[deploy.id], fx.bob, ReviewDecision::Approved, "", NOW).await.unwrap().is_none(), "decided once");
    let claimed = store.claim(&claim(runner, scope, &labels, &claim_one())).await.unwrap().unwrap();
    assert_eq!(claimed.job.id, deploy.id);
    store.set_environment_url(claimed.task.id, "https://example.com/v1").await.unwrap();
    store.finish_task(claimed.task.id, Status::Success, NOW).await.unwrap();
    let reviews = store.deployment_reviews(run.id).await.unwrap();
    assert_eq!((reviews.len(), reviews[0].decision, reviews[0].comment.as_str()), (1, ReviewDecision::Approved, "ship it"));
    let deploy = store.jobs(run.id).await.unwrap().into_iter().find(|job| job.key == "deploy").unwrap();
    assert_eq!(deploy.environment_url.as_deref(), Some("https://example.com/v1"));

    // A rerun is a new attempt: it waits again, and a rejection fails it.
    store.rerun(fx.rocket, run.id, Rerun::Job(deploy.id), RunStart::Runnable, NOW, None).await.unwrap().unwrap();
    let deploy = store.jobs(run.id).await.unwrap().into_iter().find(|job| job.key == "deploy").unwrap();
    assert!(deploy.awaiting_review && deploy.environment_url.is_none(), "{deploy:?}");
    store.review_jobs(run.id, &[deploy.id], fx.bob, ReviewDecision::Rejected, "not today", NOW).await.unwrap().unwrap();
    let after = store.run(run.id).await.unwrap().unwrap();
    assert_eq!(after.status, Status::Failure);

    // Reviewers removed while a job waits: it runs without review.
    store.rerun(fx.rocket, run.id, Rerun::Job(deploy.id), RunStart::Runnable, NOW, None).await.unwrap().unwrap();
    let changes = EnvironmentChanges { reviewers: &[], prevent_self_review: false, branch_policy: &BranchPolicy::All, wait_timer: 0 };
    environments.put(fx.rocket, "Production", &changes, NOW).await.unwrap();
    let released = store.release_reviews(fx.rocket, NOW).await.unwrap();
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].now_waiting, [deploy.id]);
    // Reviewers added while it waits for a runner: no runner may take it any more.
    let changes = EnvironmentChanges { reviewers: &[fx.bob], prevent_self_review: false, branch_policy: &BranchPolicy::All, wait_timer: 0 };
    environments.put(fx.rocket, "Production", &changes, NOW).await.unwrap();
    assert_eq!(store.release_reviews(fx.rocket, NOW).await.unwrap().len(), 1);
    let deploy = store.jobs(run.id).await.unwrap().into_iter().find(|job| job.key == "deploy").unwrap();
    assert_eq!((deploy.status, deploy.awaiting_review), (Status::AwaitingApproval, true));
    assert!(store.claim(&claim(runner, scope, &labels, &claim_one())).await.unwrap().is_none());
    assert!(store.release_reviews(fx.rocket, NOW).await.unwrap().is_empty(), "nothing else to bring in line");
    assert!(environments.delete(fx.rocket, "PRODUCTION").await.unwrap(), "names compare without case");

    // Workflows create environments only up to the repository's cap.
    let unprotected = EnvironmentChanges { reviewers: &[], prevent_self_review: false, branch_policy: &BranchPolicy::All, wait_timer: 0 };
    for n in 1..100 {
        environments.put(fx.rocket, &format!("e{n}"), &unprotected, NOW).await.unwrap();
    }
    let overflow = [NewJob { environment: Some("overflow"), ..job("deploy", &none, &linux) }];
    store.create_run(&new_run(fx.rocket, &overflow, RunStart::Runnable)).await.unwrap();
    let all = environments.list(fx.rocket).await.unwrap();
    assert_eq!((all.len(), all.iter().any(|environment| environment.name == "overflow")), (100, false));
}

#[tokio::test]
async fn environment_secrets_and_variables_stay_in_their_environment() {
    use rbit_core::actions::{BranchPolicy, SecretScope};
    use rbit_storage::sqlite::actions_environments::EnvironmentChanges;
    let fx = Fixture::new().await;
    let secrets = fx.db.actions_secrets();
    let changes = EnvironmentChanges {
        reviewers: &[],
        prevent_self_review: true,
        branch_policy: &BranchPolicy::Selected(vec!["main".into()]),
        wait_timer: 0,
    };
    let production = fx.db.actions_environments().put(fx.rocket, "production", &changes, NOW).await.unwrap();
    assert_eq!((production.prevent_self_review, &production.branch_policy), (true, &BranchPolicy::Selected(vec!["main".into()])));
    let scope = SecretScope::Environment(production.id);
    let name: rbit_core::actions::SecretName = "DEPLOY_KEY".parse().unwrap();
    let sealed = |id: rbit_core::actions::SecretId| SealedSecret {
        key_id: [1; 8],
        nonce: [2; 12],
        ciphertext: vec![u8::try_from(id.0).unwrap(); 16],
    };
    let info = secrets.put_secret(scope, &name, SecretVisibility::All, fx.alice, NOW, sealed).await.unwrap();
    assert_eq!(info.scope, scope);
    assert_eq!(secrets.secrets(scope).await.unwrap().len(), 1);
    assert!(secrets.secrets(SecretScope::Repository(fx.rocket)).await.unwrap().is_empty(), "not the repository's");
    assert!(secrets.secrets_for_task(fx.rocket, fx.alice).await.unwrap().is_empty(), "not given to every job");
    assert_eq!(secrets.secrets_for_environment(production.id).await.unwrap().len(), 1);
    assert!(secrets.any_secret().await.unwrap());
    let variable: rbit_core::actions::VariableName = "URL".parse().unwrap();
    secrets.put_variable(scope, &variable, "https://a", fx.alice, NOW).await.unwrap();
    let updated = secrets.put_variable(scope, &variable, "https://b", fx.alice, NOW).await.unwrap();
    assert_eq!((updated.value.as_str(), secrets.count_variables(scope).await.unwrap()), ("https://b", 1));
    assert_eq!(secrets.variables_for_task(fx.rocket, fx.alice).await.unwrap(), []);
    // Deleting the environment takes its secrets and variables along.
    assert!(fx.db.actions_environments().delete(fx.rocket, "production").await.unwrap());
    assert!(secrets.secrets(scope).await.unwrap().is_empty() && secrets.variables(scope).await.unwrap().is_empty());
    assert!(!secrets.delete_secret(scope, &name).await.unwrap());
}

#[tokio::test]
async fn a_wait_timer_holds_an_environments_jobs_until_it_is_over() {
    use rbit_core::actions::BranchPolicy;
    use rbit_storage::sqlite::actions_environments::EnvironmentChanges;
    let fx = Fixture::new().await;
    let store = fx.db.actions_runs();
    let changes = EnvironmentChanges { reviewers: &[], prevent_self_review: false, branch_policy: &BranchPolicy::All, wait_timer: 1 };
    let staging = fx.db.actions_environments().put(fx.rocket, "staging", &changes, NOW).await.unwrap();
    assert_eq!(staging.wait_timer, 1);
    let (none, linux) = (Vec::new(), strings(&["linux"]));
    let jobs = [NewJob { environment: Some("staging"), ..job("deploy", &none, &linux) }];
    store.create_run(&new_run(fx.rocket, &jobs, RunStart::Runnable)).await.unwrap();
    let scope = RunnerScope::Repository(fx.rocket);
    let (runner, labels) = fx.runner(scope, &["linux"], 1).await;
    assert!(store.claim(&claim(runner, scope, &labels, &[60; 32])).await.unwrap().is_none(), "ten seconds in, the minute is not over");
    let later = ClaimRequest { now: Timestamp(NOW.0 + 61), ..claim(runner, scope, &labels, &[61; 32]) };
    let claimed = store.claim(&later).await.unwrap().expect("the wait is over");
    assert_eq!(claimed.job.key, "deploy");
}

#[tokio::test]
async fn deployment_intent_and_gated_current_workflow_run_publish_atomically() {
    use rbit_core::actions::BranchPolicy;
    use rbit_storage::sqlite::{actions_deployments::NewIntent, actions_environments::EnvironmentChanges};
    let fx = Fixture::new().await;
    let runs = fx.db.actions_runs();
    let deployments = fx.db.actions_deployments();
    let (none, linux) = (Vec::new(), strings(&["linux"]));
    let source_jobs = [NewJob { environment: Some("staging"), ..job("deploy", &none, &linux) }];
    runs.create_run(&new_run(fx.rocket, &source_jobs, RunStart::Runnable)).await.unwrap();
    let scope = RunnerScope::Repository(fx.rocket);
    let (runner, labels) = fx.runner(scope, &["linux"], 70).await;
    let task = runs.claim(&claim(runner, scope, &labels, &[71; 32])).await.unwrap().unwrap();
    runs.finish_task(task.task.id, Status::Success, NOW).await.unwrap();
    let source = deployments.attempts(fx.rocket, None, 100).await.unwrap().remove(0);
    let changes = EnvironmentChanges { reviewers: &[fx.bob], prevent_self_review: true, branch_policy: &BranchPolicy::All, wait_timer: 1 };
    fx.db.actions_environments().put(fx.rocket, "production", &changes, NOW).await.unwrap();
    let config = deployments
        .publication_fence()
        .await
        .unwrap()
        .put_configuration(fx.rocket, "production", "staging", ".forgejo/workflows/ci.yml", NOW)
        .await
        .unwrap();
    let jobs = [NewJob { environment: Some("production"), ..job("deploy", &none, &linux) }];
    let current = "abcdef0123456789abcdef0123456789abcdef01";
    let planned =
        NewRun { event: RunEvent::WorkflowDispatch, actor: Some(fx.alice), sha: current, ..new_run(fx.rocket, &jobs, RunStart::Runnable) };
    let intent = NewIntent {
        actor: fx.alice,
        operation: "promote",
        environment: "production",
        source_attempt: source.id,
        source_sha: &source.sha,
        artifact: None,
        configuration_id: config.id,
        configuration_generation: config.generation,
        workflow_sha: current,
    };
    let run = runs.create_deployment_run(&planned, deployments.publication_fence().await.unwrap(), &intent).await.unwrap();
    assert_eq!(run.sha, current, "execute the current trusted workflow rather than the selected historical SHA");
    let intents = deployments.intents(fx.rocket, 10).await.unwrap();
    assert_eq!(intents[0].source_sha, source.sha);
    assert_eq!(intents[0].origin_attempt_id, source.id);
    assert_eq!(intents[0].workflow_sha, current);

    let target = runs.jobs(run.id).await.unwrap().remove(0);
    assert!(target.awaiting_review, "the new run goes through the current environment review gate");
    assert!(runs.claim(&claim(runner, scope, &labels, &[72; 32])).await.unwrap().is_none());
    assert_eq!(fx.count("actions_deployment_intents").await, 1);
    let before = fx.count("actions_runs").await;
    let stale = NewIntent { configuration_generation: config.generation + 1, ..intent };
    assert!(runs.create_deployment_run(&planned, deployments.publication_fence().await.unwrap(), &stale).await.is_err());
    assert_eq!(fx.count("actions_runs").await, before, "refused intent rolls its run back");
    assert_eq!(fx.count("actions_deployment_intents").await, 1);
}
