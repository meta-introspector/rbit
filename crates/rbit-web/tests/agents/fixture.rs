//! A real organization, repository and numbered issue/PR pair behind the production router.
use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use rbit_core::{AccountId, Config, IssueKind, IssueText, IssueTitle, RepoId, Timestamp, TokenScope, User, Visibility};
use rbit_identity_models::agents::{Agent, AgentAccess, AgentInput, AgentScope, Capability};
use rbit_services::{CreateAccessToken, CreateOrganization, CreateRepository, Services, SignUp};
use rbit_storage::{Database, KvStore, sqlite::issues::NewIssue};
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

pub const REPO: &str = "/repos/agent-team/scoped";
pub struct Fixture {
    pub _dir: tempfile::TempDir,
    pub db: Database,
    pub services: Services,
    pub app: axum::Router,
    pub admin: User,
    pub owner: User,
    pub org: AccountId,
    pub repo: RepoId,
}
pub struct Credential {
    pub record: Agent,
    pub input: AgentInput,
    pub token: rbit_core::AccessToken,
    pub secret: String,
}
async fn user(services: &Services, name: &str) -> User {
    services
        .accounts
        .create_user(
            SignUp { username: name.into(), email: format!("{name}@example.test"), password: "correct horse battery".into() },
            None,
        )
        .await
        .unwrap()
}
impl Fixture {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.storage.data_dir = dir.path().join("data");
        config.security.rate_limit = false;
        config.actions.enabled = true;
        std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
        let db = Database::open(&config.storage.sqlite_path()).await.unwrap();
        let kv = KvStore::open(&config.storage.kv_path()).unwrap();
        let config = Arc::new(config);
        let services = Services::new(config.clone(), &db, &kv).await.unwrap();
        let admin = user(&services, "site-owner").await;
        let owner = user(&services, "delegator").await;
        let org = services
            .orgs
            .create(&owner, CreateOrganization { name: "agent-team".into(), display_name: String::new(), description: String::new() })
            .await
            .unwrap()
            .account
            .id;
        let scoped = services
            .repositories
            .create_in(
                &owner,
                "agent-team",
                CreateRepository { name: "scoped".into(), description: String::new(), visibility: Visibility::Private },
            )
            .await
            .unwrap();
        services
            .repositories
            .create(&owner, CreateRepository { name: "outside-public".into(), description: String::new(), visibility: Visibility::Public })
            .await
            .unwrap();
        for (kind, title) in [(IssueKind::Issue, "Issue"), (IssueKind::Pull, "Pull request")] {
            db.issues()
                .create(&NewIssue {
                    repo: scoped.repo.id,
                    kind,
                    title: IssueTitle::parse(title).unwrap(),
                    body: IssueText::body("").unwrap(),
                    author: owner.id(),
                    at: Timestamp::now(),
                })
                .await
                .unwrap();
        }
        let app = rbit_web::router(rbit_web::AppState::new(config, services.clone()));
        Self { _dir: dir, db, services, app, admin, owner, org, repo: scoped.repo.id }
    }
    pub async fn agent(&self, capabilities: Vec<Capability>, organization: bool) -> Credential {
        let scope = if organization {
            AgentScope::Organization { id: self.org.0, access: AgentAccess::Write }
        } else {
            AgentScope::Repository { id: self.repo.0, access: AgentAccess::Write }
        };
        let input = AgentInput { slug: "helper".into(), name: "Assistant".into(), capabilities, scopes: vec![scope] };
        let record = self.services.agents.create(&self.owner, &input).await.unwrap();
        let (token, secret) = self
            .services
            .agents
            .create_credential(
                &self.owner,
                record.account_id,
                record.revision,
                CreateAccessToken { name: "http".into(), scope: TokenScope::Write, expires_in_days: None },
            )
            .await
            .unwrap();
        Credential { record, input, token, secret }
    }
    pub async fn capabilities(&self, credential: &mut Credential, capabilities: Vec<Capability>) {
        credential.input.capabilities = capabilities;
        credential.record = self
            .services
            .agents
            .update(&self.owner, credential.record.account_id, credential.record.revision, &credential.input, true)
            .await
            .unwrap();
    }
    pub async fn request(&self, secret: &str, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(format!("/api/v1{path}"))
                    .header(header::HOST, "localhost")
                    .header(header::AUTHORIZATION, format!("Bearer {secret}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 2_000_000).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }
}
