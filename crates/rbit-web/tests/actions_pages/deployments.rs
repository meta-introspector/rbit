//! Deployment evidence pages and scoped/origin-protected current-workflow request adapters.
use super::*;
use rbit_core::TokenScope;
use rbit_services::{CreateAccessToken, actions::environments::EnvironmentInput};
use serde_json::{Value, json};

const WORKFLOW: &str = "name: Trusted deploy
on:
  workflow_dispatch:
    inputs:
      deployment_sha: {type: string, required: true}
      deployment_environment: {type: string, required: true}
      deployment_operation: {type: string, required: true}
jobs:
  deploy:
    runs-on: x
    environment: ${{ inputs.deployment_environment }}
    steps: [{run: echo deploy}]
";

async fn setup() -> (Fixture, i64, i64, i64, String) {
    let f = Fixture::new().await;
    let repo = f.services.repositories.open(Some(&f.alice), "alice", "rocket").await.unwrap();
    f.database.actions_settings().set_enabled(repo.repo.repo.id, true, Timestamp::now()).await.unwrap();
    push(&repo, &[(".forgejo/workflows/deploy.yml", WORKFLOW)]);
    let sha = repo.reader.commit_id("refs/heads/main".into()).await.unwrap();
    let mut template = crate::actions_runner_tasks::template("deploy", &["deployment-ui"], &[]);
    template.name = "Deploy <unsafe>".into();
    template.environment = Some("staging".into());
    let payload = serde_json::to_vec(&template).unwrap();
    let jobs = [NewJob {
        key: "deploy",
        name: "Deploy <unsafe>",
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
    f.database
        .actions_runs()
        .create_run(&NewRun {
            repo: repo.repo.repo.id,
            workflow_path: ".forgejo/workflows/deploy.yml",
            workflow_name: "Deploy",
            title: "Deploy",
            event: RunEvent::Push,
            reference: "refs/heads/main",
            sha: &sha,
            actor: Some(f.alice.id()),
            trigger_actor: Some(f.alice.id()),
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
    finish_source(&f, &repo).await;
    push(&repo, &[(".forgejo/workflows/deploy.yml", &WORKFLOW.replace("echo deploy", "echo current workflow"))]);
    let actions = f.services.actions.as_ref().unwrap();
    actions.put_environment(&f.alice, &repo, "production", &EnvironmentInput::default()).await.unwrap();
    let config = actions.configure_deployment_workflow(&f.alice, &repo, "production", "staging", "deploy.yml").await.unwrap();
    let source = f.database.actions_deployments().attempts(repo.repo.repo.id, None, 100).await.unwrap()[0].id;
    (f, source, config.id, config.generation, sha)
}

async fn finish_source(f: &Fixture, repo: &rbit_services::RepoContext) {
    use rbit_services::actions::runners::{ManagedScope, RegistrationOptions};
    use rbit_workflow::protocol::{PollRequest, RegisterRequest, TaskStateRequest};
    let actions = f.services.actions.as_ref().unwrap();
    let token = actions.create_registration_token(&f.alice, ManagedScope::Repository(repo), RegistrationOptions::default()).await.unwrap();
    let request: RegisterRequest =
        serde_json::from_value(crate::actions_runner::register_body(token.secret.expose(), "deployment-ui-runner", &["deployment-ui"]))
            .unwrap();
    let registered = actions.register_runner(&request).await.unwrap();
    let runner = actions.authenticate_runner(registered.credential.expose()).await.unwrap().unwrap();
    actions.bump_tasks_version();
    let task = actions.poll(&runner, &PollRequest { free_slots: 1 }, Some("deployment-source")).await.unwrap().unwrap();
    let update: TaskStateRequest =
        serde_json::from_value(json!({"steps":[],"outputs":{},"result":"success","environment_url":"javascript:alert(1)"})).unwrap();
    actions.task_state(&runner, rbit_core::actions::TaskId(task.task_id.0), &update).await.unwrap();
}

#[tokio::test]
async fn deployment_dashboard_escapes_evidence_keeps_reader_controls_and_checks_origins() {
    let (f, source, config, generation, sha) = Box::pin(setup()).await;
    let owner = f.page("/alice/rocket/deployments", &f.owner).await;
    assert!(owner.contains("Deploy &#60;unsafe&#62;") && !owner.contains("<unsafe>"));
    assert!(!owner.contains("href=\"javascript:"));
    assert!(owner.contains("Live environment health is not monitored"));
    assert!(owner.contains("/deployments/dispatch") && owner.contains("/deployments/configure"));
    let reader = f.page("/alice/rocket/deployments", &f.reader).await;
    assert!(!reader.contains("action=\"/alice/rocket/deployments/dispatch\""));
    assert!(!reader.contains("action=\"/alice/rocket/deployments/configure\""));
    let form = format!("source_attempt={source}&configuration={config}%3A{generation}&operation=promote&artifact_id=&artifact_digest=");
    assert_eq!(f.send(Method::POST, "/alice/rocket/deployments/dispatch", &f.reader, &form).await.0, StatusCode::FORBIDDEN);
    let request = Request::builder()
        .method("POST")
        .uri("/alice/rocket/deployments/dispatch")
        .header(header::COOKIE, &f.owner)
        .header(header::HOST, "localhost:3000")
        .header(header::ORIGIN, "https://hostile.example")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(form.clone()))
        .unwrap();
    assert_eq!(f.router.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN);
    let refused = format!("{form}%3Cscript%3E");
    let (status, html, _) = f.send(Method::POST, "/alice/rocket/deployments/dispatch", &f.owner, &refused).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(html.contains("&#60;script&#62;") && !html.contains("<script>"));
    let (status, _, location) = f.send(Method::POST, "/alice/rocket/deployments/dispatch", &f.owner, &form).await;
    assert_eq!((status, location.as_deref()), (StatusCode::SEE_OTHER, Some("/alice/rocket/deployments")));
    let html = f.page("/alice/rocket/deployments", &f.owner).await;
    assert!(html.contains("Requested source commit") && html.contains("Workflow outcome") && html.contains(&sha));
}

async fn api(f: &Fixture, method: &str, path: &str, token: &str, input: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(format!("/api/v1{path}"))
        .header(header::HOST, "localhost:3000")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(input.to_string()))
        .unwrap();
    let response = f.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn deployment_api_scopes_and_exact_source_requests_match_browser_authority() {
    let (f, source, config, generation, sha) = Box::pin(setup()).await;
    let mut tokens = Vec::new();
    for scope in [TokenScope::Read, TokenScope::Write] {
        tokens.push(
            f.services
                .tokens
                .create(&f.alice, CreateAccessToken { name: format!("deployment-{scope:?}"), scope, expires_in_days: None })
                .await
                .unwrap()
                .1,
        );
    }
    let body = json!({"environment":"production","operation":"promote","source_attempt":source,"configuration_id":config,"configuration_generation":generation});
    let (_, denied) = f
        .services
        .tokens
        .create_limited(
            &f.alice,
            CreateAccessToken { name: "no-dispatch".into(), scope: TokenScope::Write, expires_in_days: None },
            &rbit_services::tokens::TokenLimit { repo: Some(f.repo_id().await), rights: Vec::new() },
        )
        .await
        .unwrap();
    assert_eq!(api(&f, "POST", "/repos/alice/rocket/deployments/requests", &denied, body.clone()).await.0, StatusCode::FORBIDDEN);
    let other = f
        .services
        .repositories
        .create(&f.alice, CreateRepository { name: "other".into(), description: String::new(), visibility: rbit_core::Visibility::Private })
        .await
        .unwrap();
    let (_, scoped) = f
        .services
        .tokens
        .create_limited(
            &f.alice,
            CreateAccessToken { name: "other-only".into(), scope: TokenScope::Write, expires_in_days: None },
            &rbit_services::tokens::TokenLimit { repo: Some(other.repo.id), rights: vec![rbit_services::tokens::Right::Dispatch] },
        )
        .await
        .unwrap();
    assert_eq!(api(&f, "POST", "/repos/alice/rocket/deployments/requests", &scoped, body.clone()).await.0, StatusCode::NOT_FOUND);

    let (status, data) = api(&f, "GET", "/repos/alice/rocket/deployments", &tokens[0], Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(data["attempts"][0]["sha"], sha);
    assert_eq!(api(&f, "POST", "/repos/alice/rocket/deployments/requests", &tokens[0], body.clone()).await.0, StatusCode::FORBIDDEN);
    let (status, created) = api(&f, "POST", "/repos/alice/rocket/deployments/requests", &tokens[1], body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let repo = f.services.repositories.open(Some(&f.alice), "alice", "rocket").await.unwrap();
    let current = repo.reader.commit_id("refs/heads/main".into()).await.unwrap();
    assert_eq!(created["workflow_sha"], current);
    assert_ne!(created["workflow_sha"], sha);
    let (_, evidence) = api(&f, "GET", "/repos/alice/rocket/deployments", &tokens[0], Value::Null).await;
    assert_eq!(evidence["active"][0]["sha"], sha);
    assert_eq!(evidence["active"][0]["workflow_sha"], current);
    let mut absent = body;
    absent["source_attempt"] = json!(source + 10000);
    assert_eq!(api(&f, "POST", "/repos/alice/rocket/deployments/requests", &tokens[1], absent).await.0, StatusCode::NOT_FOUND);
}
