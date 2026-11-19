//! Organization-wide automation policy remains limited to the current human owner.
use super::{confirm, validation};
use crate::{
    AppState,
    error::WebResult,
    extract::SignedIn,
    sudo,
    views::{
        components::FormError,
        layout::Page,
        org::{OrgHeader, OrgTab},
        settings::agents::AgentPolicyPage,
    },
};
use axum::{
    Form,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use rbit_core::User;
use rbit_localization::settings::agents::AgentsMessage;
use rbit_services::ServiceError;
use serde::Deserialize;

async fn view(state: &AppState, user: &User, page: Page, owner: &str) -> WebResult<AgentPolicyPage> {
    let ctx = state.services.orgs.open(Some(user), owner).await?;
    let allow = state.services.agents.organization_policy(user, ctx.org.account.id).await?;
    let title = AgentsMessage::Policy.text(&page.locale);
    Ok(AgentPolicyPage {
        page: page.titled(title.as_str()),
        header: OrgHeader::new(&ctx, OrgTab::Settings),
        action_url: format!("/{}/settings/agents", ctx.org.name()),
        allow,
        error: FormError::default(),
    })
}
pub(super) async fn show(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(owner): Path<String>,
) -> WebResult<AgentPolicyPage> {
    view(&state, &user, page, &owner).await
}
#[derive(Deserialize)]
pub(super) struct PolicyForm {
    policy: String,
    #[serde(default)]
    current_password: String,
}
pub(super) async fn save(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(owner): Path<String>,
    stamp: sudo::Stamp,
    Form(form): Form<PolicyForm>,
) -> WebResult<Response> {
    let mut page = view(&state, &user, page, &owner).await?;
    let result = async {
        let allow = match form.policy.as_str() {
            "allow" => true,
            "block" => false,
            _ => return Err(ServiceError::Invalid("choose allow or block agents".into())),
        };
        let jar = confirm(&state, &user, stamp, form.current_password).await?;
        let ctx = state.services.orgs.open(Some(&user), &owner).await?;
        state.services.agents.set_organization_policy(&user, ctx.org.account.id, allow).await?;
        Ok::<_, ServiceError>(jar)
    }
    .await;
    match result {
        Ok(jar) => Ok((jar, Redirect::to(&page.action_url)).into_response()),
        Err(error) => {
            page.error = validation(error)?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, page).into_response())
        }
    }
}
