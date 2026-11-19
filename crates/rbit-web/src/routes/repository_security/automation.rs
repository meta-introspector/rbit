//! Administrator forms for native scan opt-in and reviewable dependency proposals.
use super::{AppState, Form, Path, Redirect, SignedIn, State, WebResult};
use rbit_services::security::{Uploader, automation::AutomationSettings};
use serde::Deserialize;
#[derive(Deserialize)]
pub(super) struct Configuration {
    secrets: Option<String>,
    dependencies: Option<String>,
    package_network_consent: Option<String>,
    require_secrets: Option<String>,
    require_dependencies: Option<String>,
    automatic_pull_requests: Option<String>,
    minimum_severity: u8,
    workflow: String,
}
pub(super) async fn configure(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    Path((owner, name)): Path<(String, String)>,
    Form(input): Form<Configuration>,
) -> WebResult<Redirect> {
    let repo = state.services.repositories.open(Some(&actor), &owner, &name).await?;
    state
        .services
        .security
        .configure_automation(
            &actor,
            &repo,
            AutomationSettings {
                secrets: input.secrets.is_some(),
                dependencies: input.dependencies.is_some(),
                package_network_consent: input.package_network_consent.is_some(),
                require_secrets: input.require_secrets.is_some(),
                require_dependencies: input.require_dependencies.is_some(),
                automatic_pull_requests: input.automatic_pull_requests.is_some(),
                minimum_severity: input.minimum_severity,
                workflow: input.workflow,
            },
        )
        .await?;
    Ok(Redirect::to(&format!("/{}/security", repo.repo.full_name())))
}
#[derive(Deserialize)]
pub(super) struct Revision {
    commit_sha: String,
}
pub(super) async fn scan(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    Path((owner, name)): Path<(String, String)>,
    Form(input): Form<Revision>,
) -> WebResult<Redirect> {
    let repo = state.services.repositories.open(Some(&actor), &owner, &name).await?;
    let ids = state.services.security.request_automatic_scan(Uploader::User(&actor), &repo, &input.commit_sha).await?;
    Ok(Redirect::to(&format!("/{}/security{}", repo.repo.full_name(), ids.first().map_or_else(String::new, |id| format!("?scan={id}")))))
}
#[derive(Deserialize)]
pub(super) struct Proposal {
    commit_sha: String,
    package: String,
}
pub(super) async fn propose(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    Path((owner, name)): Path<(String, String)>,
    Form(input): Form<Proposal>,
) -> WebResult<Redirect> {
    let repo = state.services.repositories.open(Some(&actor), &owner, &name).await?;
    let update = state.services.security.remediate_dependency(&actor, &repo, &input.commit_sha, &input.package).await?;
    let path = update.pull_number.map_or_else(|| "security".into(), |number| format!("pull/{number}"));
    Ok(Redirect::to(&format!("/{}/{path}", repo.repo.full_name())))
}
