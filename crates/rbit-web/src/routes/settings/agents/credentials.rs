//! Recent-proof credential creation and atomic replacement render secrets exactly once.
use super::{agent_page, confirm, validation};
use crate::{
    AppState,
    error::WebResult,
    extract::SignedIn,
    sudo,
    views::{
        components::FormError,
        layout::Page,
        settings::tokens::{ExpiryChoice, parse_expiry},
    },
};
use axum::{
    Form,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use rbit_core::{AccountId, TokenScope};
use rbit_services::{AccessTokenService, CreateAccessToken, ServiceError};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub(super) struct CredentialForm {
    revision: i64,
    #[serde(default)]
    name: String,
    scope: Option<String>,
    expires: Option<String>,
    #[serde(default)]
    current_password: String,
}
fn expiry(form: &CredentialForm) -> Result<Option<u32>, ServiceError> {
    parse_expiry(form.expires.as_deref().unwrap_or("30"), AccessTokenService::EXPIRY_CHOICES)
        .ok_or_else(|| ServiceError::Invalid("choose when the token expires".into()))
}
pub(super) async fn create(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(id): Path<i64>,
    stamp: sudo::Stamp,
    Form(form): Form<CredentialForm>,
) -> WebResult<Response> {
    issue(state, user, page, id, None, stamp, form).await
}
pub(super) async fn rotate(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path((id, token)): Path<(i64, i64)>,
    stamp: sudo::Stamp,
    Form(form): Form<CredentialForm>,
) -> WebResult<Response> {
    issue(state, user, page, id, Some(token), stamp, form).await
}
async fn issue(
    state: AppState,
    user: Arc<rbit_core::User>,
    page: Page,
    id: i64,
    replaced: Option<i64>,
    stamp: sudo::Stamp,
    form: CredentialForm,
) -> WebResult<Response> {
    // Prepare every fallible read before issuing a credential. A successful issuance
    // can then return its secret even if access changes immediately afterward.
    let mut view = agent_page(&state, &user, page, Some(id), None, FormError::default()).await?;
    view.credential_name.clone_from(&form.name);
    view.credential_write = form.scope.as_deref().unwrap_or("write") == "write";
    if let Ok(selected) = expiry(&form) {
        view.expiry_choices = ExpiryChoice::list(AccessTokenService::EXPIRY_CHOICES, selected);
    }
    let result = async {
        let expires_in_days = expiry(&form)?;
        let (name, scope) = if let Some(token) = replaced {
            let credentials = state.services.agents.managed_credentials(&user, AccountId(id)).await?;
            let current = credentials.iter().find(|current| current.id == token).ok_or(ServiceError::NotFound)?;
            (current.token.name.clone(), current.token.scope)
        } else {
            let scope = match form.scope.as_deref().unwrap_or("write") {
                "read" => TokenScope::Read,
                "write" => TokenScope::Write,
                _ => return Err(ServiceError::Invalid("choose read or write credential scope".into())),
            };
            (form.name.trim().to_owned(), scope)
        };
        let jar = confirm(&state, &user, stamp, form.current_password).await?;
        let input = CreateAccessToken { name, scope, expires_in_days };
        let created = if let Some(token) = replaced {
            state.services.agents.rotate_credential(&user, AccountId(id), form.revision, token, input).await?
        } else {
            state.services.agents.create_managed_credential(&user, AccountId(id), form.revision, input).await?
        };
        Ok::<_, ServiceError>((jar, created))
    }
    .await;
    match result {
        Ok((jar, (token, secret))) => {
            view.show_created(&token, secret, replaced);
            Ok((jar, [(header::CACHE_CONTROL, "no-store")], view).into_response())
        }
        Err(error) => {
            view.error = validation(error)?;
            Ok((StatusCode::UNPROCESSABLE_ENTITY, [(header::CACHE_CONTROL, "no-store")], view).into_response())
        }
    }
}
pub(super) async fn revoke(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((id, token)): Path<(i64, i64)>,
) -> WebResult<Redirect> {
    state.services.agents.revoke_managed_credential(&user, AccountId(id), token).await?;
    Ok(Redirect::to(&format!("/settings/agents/{id}")))
}
