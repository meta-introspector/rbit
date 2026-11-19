//! Administrator and organization-owner MFA enrollment policy editors.
use crate::{
    AppState,
    error::{WebError, WebResult},
    extract::{SignedIn, SiteAdmin},
    views::{
        admin::{AdminLink, AdminSection},
        components::FormError,
        layout::Page,
    },
};
use askama::Template;
use askama_web::WebTemplate;
use axum::{
    Form, Router,
    extract::{Path, State},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use serde::Deserialize;

#[derive(Debug, Template, WebTemplate)]
#[template(path = "admin/security.html")]
pub struct SiteSecurityPage {
    pub page: Page,
    pub admin_nav: Vec<AdminLink>,
    pub required: bool,
    pub error: FormError,
}

#[derive(Debug, Template, WebTemplate)]
#[template(path = "org/security.html")]
pub struct OrgSecurityPage {
    pub page: Page,
    pub name: String,
    pub settings_url: String,
    pub action_url: String,
    pub required: bool,
    pub site_required: bool,
    pub error: FormError,
}

#[derive(Deserialize)]
pub struct PolicyForm {
    required: Option<String>,
}

impl PolicyForm {
    fn required(&self) -> Result<bool, rbit_services::ServiceError> {
        match self.required.as_deref() {
            Some("optional") => Ok(false),
            Some("required") => Ok(true),
            _ => Err(rbit_services::ServiceError::Invalid("Choose optional or required MFA.".into())),
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/security", get(site).post(save_site))
        .route("/{owner}/settings/security", get(organization).post(save_organization))
}

async fn site(State(state): State<AppState>, SiteAdmin(_): SiteAdmin, page: Page) -> WebResult<SiteSecurityPage> {
    Ok(SiteSecurityPage {
        page: titled(page),
        admin_nav: AdminSection::Security.nav(),
        required: state.services.authentication_policy.site_required().await?,
        error: FormError::default(),
    })
}

async fn save_site(
    State(state): State<AppState>,
    SiteAdmin(user): SiteAdmin,
    page: Page,
    Form(form): Form<PolicyForm>,
) -> WebResult<Response> {
    let result = match form.required() {
        Ok(required) => state.services.authentication_policy.set_site(&user, required).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => Ok(Redirect::to("/admin/security").into_response()),
        Err(rbit_services::ServiceError::Invalid(message)) => {
            let view = SiteSecurityPage {
                page: titled(page),
                admin_nav: AdminSection::Security.nav(),
                required: form.required().unwrap_or(false),
                error: FormError::new(message),
            };
            Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, view).into_response())
        }
        Err(error) => Err(error.into()),
    }
}

async fn org_page(state: &AppState, user: &rbit_core::User, page: Page, owner: &str) -> WebResult<OrgSecurityPage> {
    let ctx = state.services.orgs.open(Some(user), owner).await?;
    if !ctx.can_manage {
        return Err(WebError::NotFound);
    }
    let settings_url = format!("/{}/settings", ctx.org.name());
    Ok(OrgSecurityPage {
        page: titled(page),
        name: ctx.org.label().to_owned(),
        action_url: format!("{settings_url}/security"),
        settings_url,
        required: state.services.authentication_policy.organization_required(ctx.org.account.id).await?,
        site_required: state.services.authentication_policy.site_required().await?,
        error: FormError::default(),
    })
}

async fn organization(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(owner): Path<String>,
) -> WebResult<OrgSecurityPage> {
    org_page(&state, &user, page, &owner).await
}

async fn save_organization(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(owner): Path<String>,
    Form(form): Form<PolicyForm>,
) -> WebResult<Response> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    if !ctx.can_manage {
        return Err(WebError::NotFound);
    }
    let result = match form.required() {
        Ok(required) => state.services.authentication_policy.set_organization(&user, &ctx.org, required).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => Ok(Redirect::to(&format!("/{}/settings/security", ctx.org.name())).into_response()),
        Err(rbit_services::ServiceError::Invalid(message)) => {
            let mut view = org_page(&state, &user, page, &owner).await?;
            view.error = FormError::new(message);
            Ok((axum::http::StatusCode::UNPROCESSABLE_ENTITY, view).into_response())
        }
        Err(error) => Err(error.into()),
    }
}

/// Security editors use the same localized browser title as their heading.
fn titled(page: Page) -> Page {
    let title = rbit_localization::settings::authentication_policy::PolicyMessage::Security.text(&page.locale);
    page.titled(title.as_str())
}
