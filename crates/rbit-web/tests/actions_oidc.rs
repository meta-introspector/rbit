//! OIDC through real runner claims and HTTP: public keys, scoped JWTs and live revocation.
use axum::{
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier as _};
use rbit_core::{
    Timestamp,
    actions::{Run, RunEvent},
};
use rbit_storage::sqlite::actions_runs::{NewJob, NewRun, RunStart};
use rbit_workflow::spec::{Access, JobSpec, JobTemplate, TokenPermissions};
use serde_json::Value;
use tower::ServiceExt;

use crate::{
    actions_runner::Fixture,
    actions_runner_tasks::{poll, spec, template},
};

pub(super) async fn get(fixture: &Fixture, path: &str, bearer: Option<&str>) -> (StatusCode, HeaderMap, Value) {
    let mut request = Request::builder().uri(path).header(header::HOST, "git.example.com");
    if let Some(bearer) = bearer {
        request = request.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = fixture.router.clone().oneshot(request.body(Body::empty()).unwrap()).await.unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let body = to_bytes(response.into_body(), 32 * 1024).await.unwrap();
    (status, headers, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

pub(super) fn job(enabled: bool) -> JobTemplate {
    let mut job = template("deploy", &["linux"], &[]);
    job.permissions =
        Some(TokenPermissions { contents: Access::Read, issues: Access::None, pull_requests: Access::None, id_token: enabled });
    job
}

pub(super) async fn seed(fixture: &Fixture, job: &JobTemplate, event: RunEvent, trusted: bool, fork: bool) -> Run {
    seed_by(fixture, job, event, trusted, fork, fixture.alice.id(), fixture.alice.id()).await
}

pub(super) async fn seed_by(
    fixture: &Fixture,
    job: &JobTemplate,
    event: RunEvent,
    trusted: bool,
    fork: bool,
    actor: rbit_core::AccountId,
    trigger: rbit_core::AccountId,
) -> Run {
    let repository = fixture.repo().await.repo.repo.id;
    let payload = serde_json::to_vec(job).unwrap();
    let permissions = job.permissions.map(|permissions| serde_json::to_string(&permissions).unwrap());
    let new = NewJob {
        key: &job.job_id,
        name: &job.name,
        matrix: None,
        needs: &job.needs,
        runs_on: &job.runs_on,
        permissions: permissions.as_deref(),
        payload: &payload,
        condition: false,
        max_parallel: None,
        group: None,
        environment: job.environment.as_deref(),
    };
    let run = fixture
        .database
        .actions_runs()
        .create_run(&NewRun {
            repo: repository,
            workflow_path: ".rbit/workflows/deploy.yml",
            workflow_name: "Deploy",
            title: "Deploy",
            event,
            reference: "refs/heads/main",
            sha: "0123456789abcdef0123456789abcdef01234567",
            actor: Some(actor),
            trigger_actor: Some(trigger),
            pull_number: None,
            is_fork: fork,
            trusted,
            concurrency_group: None,
            cancel_in_progress: false,
            event_payload: "{}",
            dispatch_inputs: None,
            start: RunStart::Runnable,
            jobs: &[new],
            error: None,
            now: Timestamp::now(),
        })
        .await
        .unwrap();
    fixture.actions().bump_tasks_version();
    run
}

pub(super) async fn claim(fixture: &Fixture) -> JobSpec {
    let runner = fixture.runner(&["linux"]).await;
    let (status, body) = poll(fixture, &runner, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    spec(&body)
}

pub(super) async fn issued(fixture: &Fixture, spec: &JobSpec) -> Value {
    let (status, headers, body) =
        get(fixture, "/api/actions/oidc/token?api-version=1&audience=sts.amazonaws.com", Some(spec.token.expose())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    body
}

fn verify(token: &str, jwks: &Value) -> Value {
    let (signed, signature) = token.rsplit_once('.').unwrap();
    let (header, payload) = signed.split_once('.').unwrap();
    let header: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(header).unwrap()).unwrap();
    assert_eq!(header["alg"], "ES256");
    let key = jwks["keys"].as_array().unwrap().iter().find(|key| key["kid"] == header["kid"]).unwrap();
    assert!(key.get("d").is_none());
    let mut point = vec![4];
    point.extend(URL_SAFE_NO_PAD.decode(key["x"].as_str().unwrap()).unwrap());
    point.extend(URL_SAFE_NO_PAD.decode(key["y"].as_str().unwrap()).unwrap());
    let key = VerifyingKey::from_sec1_bytes(&point).unwrap();
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature).unwrap()).unwrap();
    key.verify(signed.as_bytes(), &signature).unwrap();
    assert!(key.verify(format!("{signed}x").as_bytes(), &signature).is_err());
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap()
}

#[tokio::test]
async fn oidc_public_discovery_and_real_runner_token_match_cloud_contract() {
    let fixture = Fixture::new().await;
    let (status, _, discovery) = get(&fixture, "/api/actions/oidc/.well-known/openid-configuration", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(discovery["issuer"], "https://git.example.com/api/actions/oidc");
    let run = seed(&fixture, &job(true), RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    assert_eq!(spec.oidc_request_url.as_deref(), Some("https://git.example.com/api/actions/oidc/token?api-version=1"));
    let body = issued(&fixture, &spec).await;
    let (status, _, jwks) = get(&fixture, "/api/actions/oidc/jwks", None).await;
    assert_eq!(status, StatusCode::OK);
    let claims = verify(body["value"].as_str().unwrap(), &jwks);
    assert_eq!(claims["aud"], "sts.amazonaws.com");
    assert_eq!(claims["iss"], discovery["issuer"]);
    assert_eq!(claims["sha"], run.sha);
    assert_eq!(claims["run_id"], run.id.0.to_string());
    assert_eq!(claims["task_id"], spec.task_id.0.to_string());
    assert_eq!(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap(), 300);
    assert!(claims["sub"].as_str().unwrap().ends_with(":ref:refs/heads/main"));
    assert_eq!(claims["repository_uid"].as_str().unwrap().len(), 32);
    assert_eq!(claims["repository_owner_uid"].as_str().unwrap().len(), 32);
    assert_ne!(issued(&fixture, &spec).await["value"], body["value"]);
    let (status, _, default) = get(&fixture, "/api/actions/oidc/token?api-version=1", Some(spec.token.expose())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verify(default["value"].as_str().unwrap(), &jwks)["aud"], discovery["issuer"]);
    fixture.database.actions_runs().cancel_run(run.repo_id, run.id, Timestamp::now()).await.unwrap();
    assert!(!get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0.is_success());
}

#[tokio::test]
async fn oidc_never_grants_default_untrusted_fork_or_pull_request_jobs() {
    for (permission, trusted, fork, event) in [
        (false, true, false, RunEvent::Push),
        (true, false, false, RunEvent::Push),
        (true, true, true, RunEvent::Push),
        (true, true, false, RunEvent::PullRequest),
    ] {
        let fixture = Fixture::new().await;
        seed(&fixture, &job(permission), event, trusted, fork).await;
        let spec = claim(&fixture).await;
        assert!(spec.oidc_request_url.is_none());
        let (status, _, _) = get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await;
        assert!(!status.is_success(), "{permission}/{trusted}/{fork}/{event:?}");
    }
}

#[tokio::test]
async fn oidc_rechecks_live_heartbeat_repository_switch_and_token_replacement() {
    let fixture = Fixture::new().await;
    let run = seed(&fixture, &job(true), RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    issued(&fixture, &spec).await;
    let path = "/api/actions/oidc/token?audience=sts.amazonaws.com";
    // Heartbeats are monotonic; exercise the admission cutoff without rewinding one.
    let digest = rbit_auth::TaskTokenSecret::parse(spec.token.expose()).unwrap().digest();
    let (task, _) = fixture.database.actions_runs().running_task_by_token(&digest).await.unwrap().unwrap();
    let now = Timestamp::now().0;
    let store = fixture.database.actions_oidc();
    assert!(store.issue::<bool>(&digest, now, task.heartbeat_at.0 + 1, |_| panic!("stale task reached signer")).await.unwrap().is_none());
    assert_eq!(store.issue(&digest, now, task.heartbeat_at.0, |_| Some(true)).await.unwrap(), Some(true));
    fixture.database.actions_settings().set_enabled(run.repo_id, false, Timestamp::now()).await.unwrap();
    assert!(!get(&fixture, path, Some(spec.token.expose())).await.0.is_success());
    fixture.database.actions_settings().set_enabled(run.repo_id, true, Timestamp::now()).await.unwrap();
    issued(&fixture, &spec).await;
    fixture.database.actions_runs().replace_task_token(rbit_core::actions::TaskId(spec.task_id.0), &[19; 32]).await.unwrap();
    assert!(!get(&fixture, path, Some(spec.token.expose())).await.0.is_success());
}

#[tokio::test]
async fn oidc_refuses_unsafe_issuers_anonymous_callers_and_invalid_audiences() {
    let fixture = Fixture::new().await;
    assert_eq!(get(&fixture, "/api/actions/oidc/token?audience=x", None).await.0, StatusCode::UNAUTHORIZED);
    seed(&fixture, &job(true), RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    for path in ["/api/actions/oidc/token?audience=", "/api/actions/oidc/token?audience=a%0Ab", "/api/actions/oidc/token?api-version=2"] {
        assert_eq!(get(&fixture, path, Some(spec.token.expose())).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let disabled = Fixture::with(|config| config.actions.enabled = false).await;
    assert_eq!(get(&disabled, "/api/actions/oidc/jwks", None).await.0, StatusCode::NOT_FOUND);
    for value in ["http://example.com", "https://a:b@example.com", "https://example.com?x=y", "https://example.com#x"] {
        assert!(rbit_services::actions::oidc::issuer(value).is_err());
    }
}

#[tokio::test]
async fn oidc_expiry_is_bounded_by_job_timeout_and_caller_ceiling() {
    let fixture = Fixture::new().await;
    let mut bounded = job(true);
    bounded.timeout_minutes = 1;
    seed(&fixture, &bounded, RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    let body = issued(&fixture, &spec).await;
    let (_, _, jwks) = get(&fixture, "/api/actions/oidc/jwks", None).await;
    let claims = verify(body["value"].as_str().unwrap(), &jwks);
    assert!((1..=60).contains(&(claims["exp"].as_i64().unwrap() - claims["iat"].as_i64().unwrap())));
    let fixture = Fixture::new().await;
    let mut called = job(true);
    called.job_id = "caller/deploy".to_owned();
    called.default_ceiling = true;
    seed(&fixture, &called, RunEvent::Push, true, false).await;
    let spec = claim(&fixture).await;
    assert!(spec.oidc_request_url.is_none());
    assert_eq!(
        get(&fixture, "/api/actions/oidc/token?audience=sts.amazonaws.com", Some(spec.token.expose())).await.0,
        StatusCode::FORBIDDEN
    );
}
