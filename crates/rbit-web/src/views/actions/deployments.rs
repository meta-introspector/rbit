//! Deployment cards keep requested source identity separate from workflow outcomes and observed links.
use super::{
    StateLook,
    job_text::{LocalizedState, localized_state},
};
use crate::views::{components::FormError, layout::Page, repo::RepoHeader, when::When};
use askama::Template;
use askama_web::WebTemplate;
use rbit_core::actions::Status;
use rbit_localization::{ActionsEnvironmentMessage as Message, Catalog};
use rbit_services::actions::deployments::Dashboard;

#[derive(Debug, Default, Clone)]
pub struct Draft {
    pub configuration: String,
    pub source_attempt: String,
    pub operation: String,
    pub artifact_id: String,
    pub artifact_digest: String,
}

#[derive(Debug)]
pub struct AttemptRow {
    pub id: i64,
    pub environment: String,
    pub name: String,
    pub sha: String,
    pub workflow_sha: String,
    pub origin_attempt: Option<i64>,
    pub origin_artifact: Option<(i64, String)>,
    pub run_url: Option<String>,
    pub run_number: i64,
    pub attempt: i64,
    pub look: LocalizedState,
    pub observed_url: Option<String>,
    pub selectable: bool,
    pub when: Option<When>,
}

#[derive(Debug)]
pub struct ConfigurationRow {
    pub value: String,
    pub target: String,
    pub source: String,
    pub workflow: String,
}

#[derive(Debug)]
pub struct IntentRow {
    pub id: i64,
    pub operation: Message<'static>,
    pub environment: String,
    pub source_attempt: i64,
    pub origin_attempt: i64,
    pub origin_artifact: Option<(i64, String)>,
    pub source_sha: String,
    pub workflow_sha: String,
    pub workflow: String,
    pub artifact: Option<(i64, String)>,
    pub run_url: Option<String>,
    pub run_number: i64,
    pub look: Option<LocalizedState>,
    pub when: When,
}

#[derive(Debug, Template, WebTemplate)]
#[template(path = "actions/deployments.html")]
pub struct DeploymentsPage {
    pub page: Page,
    pub header: RepoHeader,
    pub action_url: String,
    pub configure_url: String,
    pub attempts: Vec<AttemptRow>,
    pub active: Vec<AttemptRow>,
    pub intents: Vec<IntentRow>,
    pub configurations: Vec<ConfigurationRow>,
    pub can_write: bool,
    pub can_admin: bool,
    pub next_url: Option<String>,
    pub error: FormError,
    pub draft: Draft,
}

impl DeploymentsPage {
    pub fn load(&mut self, data: Dashboard) {
        let catalog = Catalog::new(self.page.locale);
        let home = &self.header.home_url;
        self.next_url =
            data.attempts.last().filter(|_| data.attempts.len() == 50).map(|last| format!("{home}/deployments?before={}", last.id));
        self.attempts = data
            .attempts
            .into_iter()
            .map(|attempt| AttemptRow {
                id: attempt.id,
                environment: attempt.environment.unwrap_or_default(),
                name: attempt.job_name,
                sha: attempt.sha,
                workflow_sha: attempt.workflow_sha,
                origin_attempt: attempt.origin_attempt_id,
                origin_artifact: attempt.artifact_id.zip(attempt.artifact_digest),
                run_url: attempt.run_available.then(|| format!("{home}/actions/runs/{}", attempt.run_number)),
                run_number: attempt.run_number,
                attempt: attempt.job_attempt,
                look: localized_state(catalog, &StateLook::of(attempt.status)),
                observed_url: observed_url(attempt.environment_url),
                when: attempt.finished_at.map(When),
                selectable: attempt.status == Status::Success && attempt.trusted && !attempt.is_fork && attempt.event != "pull_request",
            })
            .collect();
        self.active = data
            .active
            .into_iter()
            .map(|attempt| AttemptRow {
                id: attempt.job_id,
                environment: attempt.environment,
                name: attempt.job_name,
                sha: attempt.sha,
                workflow_sha: attempt.workflow_sha,
                origin_attempt: attempt.origin_attempt_id,
                origin_artifact: attempt.artifact_id.zip(attempt.artifact_digest),
                run_url: Some(format!("{home}/actions/runs/{}", attempt.run_number)),
                run_number: attempt.run_number,
                attempt: attempt.job_attempt,
                look: localized_state(catalog, &StateLook::of(Status::from_code(attempt.status).unwrap_or(Status::Waiting))),
                observed_url: observed_url(attempt.environment_url),
                selectable: false,
                when: None,
            })
            .collect();
        self.configurations = data
            .configurations
            .into_iter()
            .map(|config| ConfigurationRow {
                value: format!("{}:{}", config.id, config.generation),
                target: config.environment,
                source: config.source_environment,
                workflow: config.workflow_path,
            })
            .collect();
        self.intents = data
            .intents
            .into_iter()
            .map(|intent| IntentRow {
                id: intent.id,
                when: When(rbit_core::Timestamp(intent.created_at)),
                operation: if intent.operation == "rollback" { Message::RollbackDeployment } else { Message::PromoteDeployment },
                environment: intent.environment,
                source_attempt: intent.source_attempt_id,
                origin_attempt: intent.origin_attempt_id,
                origin_artifact: intent.origin_artifact_id.zip(intent.origin_artifact_digest),
                source_sha: intent.source_sha,
                workflow_sha: intent.workflow_sha,
                workflow: intent.workflow_path,
                artifact: intent.artifact_id.zip(intent.artifact_digest),
                run_url: intent.run_id.map(|_| format!("{home}/actions/runs/{}", intent.run_number)),
                run_number: intent.run_number,
                look: intent.workflow_status.and_then(Status::from_code).map(|status| localized_state(catalog, &StateLook::of(status))),
            })
            .collect();
    }
}

fn observed_url(url: Option<String>) -> Option<String> {
    url.filter(|value| {
        value.parse::<axum::http::Uri>().is_ok_and(|uri| {
            matches!(uri.scheme_str(), Some("http" | "https")) && uri.authority().is_some_and(|authority| !authority.host().is_empty())
        })
    })
}
