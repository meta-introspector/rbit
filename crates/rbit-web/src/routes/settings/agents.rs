//! Owner-controlled agent settings use current service authority and recent credential proof.
use crate::{
    AppState,
    error::WebResult,
    extract::SignedIn,
    sudo,
    views::{
        components::FormError,
        layout::Page,
        settings::agents::{AgentForm, AgentPage, AgentsPage},
    },
};
use axum::{
    Form, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_extra::extract::CookieJar;
use rbit_core::{AccountId, User};
use rbit_services::ServiceError;
use serde::Deserialize;
use std::sync::Arc;
mod credentials;
mod forms;
mod policy;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/settings/agents", get(list).post(create))
        .route("/settings/agents/new", get(new))
        .route("/settings/agents/{id}", get(edit).post(update))
        .route("/settings/agents/{id}/disable", post(disable))
        .route("/settings/agents/{id}/credentials", post(credentials::create))
        .route("/settings/agents/{id}/credentials/{token}/revoke", post(credentials::revoke))
        .route("/settings/agents/{id}/credentials/{token}/rotate", post(credentials::rotate))
        .route("/{owner}/settings/agents", get(policy::show).post(policy::save))
}
async fn list(State(state): State<AppState>, SignedIn(user): SignedIn, page: Page) -> WebResult<AgentsPage> {
    Ok(AgentsPage::new(page, state.services.agents.list(&user).await?))
}
async fn new(State(state): State<AppState>, SignedIn(user): SignedIn, page: Page) -> WebResult<AgentPage> {
    state.services.agents.list(&user).await?;
    agent_page(&state, &user, page, None, None, FormError::default()).await
}
async fn edit(State(state): State<AppState>, SignedIn(user): SignedIn, page: Page, Path(id): Path<i64>) -> WebResult<AgentPage> {
    agent_page(&state, &user, page, Some(id), None, FormError::default()).await
}
pub(super) async fn agent_page(
    state: &AppState,
    user: &User,
    page: Page,
    id: Option<i64>,
    submitted: Option<AgentForm>,
    error: FormError,
) -> WebResult<AgentPage> {
    let mut view = if let Some(id) = id {
        let agent = state.services.agents.find(user, AccountId(id)).await?;
        let scopes = state.services.agents.scope_names(user, AccountId(id)).await?;
        let tokens = state.services.agents.managed_credentials(user, AccountId(id)).await?;
        let form = submitted.unwrap_or_else(|| AgentForm::stored(&agent, &scopes));
        AgentPage::new(page, Some(&agent), form, &scopes, &tokens)
    } else {
        AgentPage::new(page, None, submitted.unwrap_or_else(AgentForm::blank), &[], &[])
    };
    view.error = error;
    Ok(view)
}
pub(super) async fn confirm(state: &AppState, user: &User, stamp: sudo::Stamp, password: String) -> Result<CookieJar, ServiceError> {
    let session = crate::middleware::session_token(&stamp.jar).map(|token| token.encode()).unwrap_or_default();
    sudo::confirm(state, stamp.jar, stamp.secure, &session, user, password).await
}
pub(super) fn validation(error: ServiceError) -> WebResult<FormError> {
    match error {
        ServiceError::Invalid(message) => {
            Ok(if message.ends_with('.') { FormError::new(message) } else { FormError::from_message(&message) })
        }
        conflict @ ServiceError::Conflict(_) => Ok(FormError::from_message(&conflict.to_string())),
        other => Err(other.into()),
    }
}
#[derive(Default, Deserialize)]
#[serde(default)]
#[expect(clippy::struct_excessive_bools, reason = "one checkbox per independent capability and enabled flag")]
struct AgentWrite {
    slug: String,
    name: String,
    read_repositories: String,
    write_repositories: String,
    read_organizations: String,
    write_organizations: String,
    revision: i64,
    enabled: bool,
    push: bool,
    issues: bool,
    pull_requests: bool,
    statuses: bool,
    dispatch: bool,
    read_logs: bool,
    merge: bool,
    current_password: String,
}
impl AgentWrite {
    fn into_form(self) -> (AgentForm, String) {
        (
            AgentForm {
                slug: self.slug,
                name: self.name,
                read_repositories: self.read_repositories,
                write_repositories: self.write_repositories,
                read_organizations: self.read_organizations,
                write_organizations: self.write_organizations,
                revision: self.revision,
                enabled: self.enabled,
                push: self.push,
                issues: self.issues,
                pull_requests: self.pull_requests,
                statuses: self.statuses,
                dispatch: self.dispatch,
                read_logs: self.read_logs,
                merge: self.merge,
            },
            self.current_password,
        )
    }
}

async fn create(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    stamp: sudo::Stamp,
    Form(form): Form<AgentWrite>,
) -> WebResult<Response> {
    save(state, user, page, None, stamp, form).await
}
async fn update(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(id): Path<i64>,
    stamp: sudo::Stamp,
    Form(form): Form<AgentWrite>,
) -> WebResult<Response> {
    save(state, user, page, Some(id), stamp, form).await
}
async fn save(state: AppState, user: Arc<User>, page: Page, id: Option<i64>, stamp: sudo::Stamp, form: AgentWrite) -> WebResult<Response> {
    let (fields, current_password) = form.into_form();
    let result = async {
        if let Some(id) = id {
            state.services.agents.find(&user, AccountId(id)).await?;
        }
        let jar = confirm(&state, &user, stamp, current_password).await?;
        let input = fields.input(&state, &user, page.locale).await?;
        let agent = if let Some(id) = id {
            state.services.agents.update(&user, AccountId(id), fields.revision, &input, fields.enabled).await?
        } else {
            state.services.agents.create(&user, &input).await?
        };
        Ok::<_, ServiceError>((jar, agent))
    }
    .await;
    match result {
        Ok((jar, agent)) => Ok((jar, Redirect::to(&format!("/settings/agents/{}", agent.account_id.0))).into_response()),
        Err(error) => {
            let error = validation(error)?;
            let view = agent_page(&state, &user, page, id, Some(fields), error).await?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, view).into_response())
        }
    }
}
#[derive(Deserialize)]
struct Revision {
    revision: i64,
}
async fn disable(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path(id): Path<i64>,
    Form(form): Form<Revision>,
) -> WebResult<Redirect> {
    state.services.agents.disable(&user, AccountId(id), form.revision).await?;
    Ok(Redirect::to(&format!("/settings/agents/{id}")))
}
