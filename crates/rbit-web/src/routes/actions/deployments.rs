//! Reader deployment dashboard and origin-protected writer/admin request forms.
use crate::{
    AppState,
    error::{WebError, WebResult},
    extract::{SignedIn, Viewer},
    routes::repo::RepoPage,
    views::{
        actions::deployments::{DeploymentsPage, Draft},
        components::FormError,
        layout::Page,
    },
};
use axum::{
    Form,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use rbit_services::{ActionsService, ServiceError, actions::deployments::DeploymentRequest};
use serde::Deserialize;

fn actions(state: &AppState) -> WebResult<&ActionsService> {
    state.services.actions.as_ref().ok_or(WebError::NotFound)
}

#[derive(Debug, Default, Deserialize)]
pub struct HistoryQuery {
    before: Option<i64>,
}

async fn page_view(
    state: &AppState,
    loaded: &RepoPage,
    page: Page,
    before: Option<i64>,
    draft: Draft,
    error: FormError,
) -> WebResult<DeploymentsPage> {
    let title =
        rbit_localization::Catalog::new(page.locale).localized(rbit_localization::ActionsEnvironmentMessage::Deployments).to_string();
    let home = loaded.urls.home();
    let mut view = DeploymentsPage {
        page: page.titled(title),
        header: loaded.header(state, "actions"),
        action_url: format!("{home}/deployments/dispatch"),
        configure_url: format!("{home}/deployments/configure"),
        attempts: Vec::new(),
        active: Vec::new(),
        intents: Vec::new(),
        configurations: Vec::new(),
        can_write: loaded.ctx.can_write_content(),
        can_admin: loaded.ctx.access.can_admin() && loaded.ctx.can_write_content(),
        next_url: None,
        draft,
        error,
    };
    view.load(actions(state)?.deployment_dashboard(&loaded.ctx, before).await?);
    Ok(view)
}

pub async fn show(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
) -> WebResult<DeploymentsPage> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    page_view(&state, &loaded, page, query.before, Draft::default(), FormError::default()).await
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeploymentForm {
    configuration: String,
    source_attempt: String,
    operation: String,
    artifact_id: String,
    artifact_digest: String,
}
impl DeploymentForm {
    fn draft(&self) -> Draft {
        Draft {
            configuration: self.configuration.clone(),
            source_attempt: self.source_attempt.clone(),
            operation: self.operation.clone(),
            artifact_id: self.artifact_id.clone(),
            artifact_digest: self.artifact_digest.clone(),
        }
    }
    async fn request(&self, service: &ActionsService, repo: &rbit_services::RepoContext) -> Result<DeploymentRequest, ServiceError> {
        let invalid = || ServiceError::Invalid("choose a source attempt, target workflow and operation".into());
        let (id, generation) = self.configuration.split_once(':').ok_or_else(invalid)?;
        let (id, generation) = (id.parse::<i64>().map_err(|_| invalid())?, generation.parse::<i64>().map_err(|_| invalid())?);
        let source_attempt = self.source_attempt.parse::<i64>().ok().filter(|value| *value > 0).ok_or_else(invalid)?;
        let config = service
            .deployment_dashboard(repo, None)
            .await?
            .configurations
            .into_iter()
            .find(|config| config.id == id)
            .ok_or_else(invalid)?;
        let artifact_id =
            if self.artifact_id.trim().is_empty() { None } else { Some(self.artifact_id.trim().parse::<i64>().map_err(|_| invalid())?) };
        let artifact_digest = (!self.artifact_digest.trim().is_empty()).then(|| self.artifact_digest.trim().to_owned());
        Ok(DeploymentRequest {
            environment: config.environment,
            operation: self.operation.clone(),
            source_attempt,
            configuration_id: id,
            configuration_generation: generation,
            artifact_id,
            artifact_digest,
        })
    }
}

pub async fn dispatch(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    viewer: Viewer,
    page: Page,
    Path((owner, repo)): Path<(String, String)>,
    Form(form): Form<DeploymentForm>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    let service = actions(&state)?;
    let result = match form.request(service, &loaded.ctx).await {
        Ok(request) => service.deploy(&actor, &loaded.ctx, request).await.map(|_| ()),
        Err(error) => Err(error),
    };
    respond(&state, &loaded, page, form.draft(), result).await
}

#[derive(Debug, Deserialize)]
pub struct ConfigurationForm {
    environment: String,
    source_environment: String,
    workflow: String,
}

pub async fn configure(
    State(state): State<AppState>,
    SignedIn(actor): SignedIn,
    viewer: Viewer,
    page: Page,
    Path((owner, repo)): Path<(String, String)>,
    Form(form): Form<ConfigurationForm>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    let result = actions(&state)?
        .configure_deployment_workflow(&actor, &loaded.ctx, &form.environment, &form.source_environment, &form.workflow)
        .await
        .map(|_| ());
    respond(&state, &loaded, page, Draft::default(), result).await
}

async fn respond(state: &AppState, loaded: &RepoPage, page: Page, draft: Draft, result: Result<(), ServiceError>) -> WebResult<Response> {
    match result {
        Ok(()) => Ok(Redirect::to(&format!("{}/deployments", loaded.urls.home())).into_response()),
        Err(error @ (ServiceError::Invalid(_) | ServiceError::Conflict(_))) => {
            let status = if matches!(error, ServiceError::Conflict(_)) { StatusCode::CONFLICT } else { StatusCode::UNPROCESSABLE_ENTITY };
            let view = page_view(state, loaded, page, None, draft, FormError::from_message(&error.to_string())).await?;
            Ok((status, view).into_response())
        }
        Err(error) => Err(error.into()),
    }
}
