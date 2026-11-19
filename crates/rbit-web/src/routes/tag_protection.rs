//! Repository tag protection settings with origin-checked forms and explicit removal.
use super::repo::RepoPage;
use crate::{
    AppState,
    error::{WebError, WebResult},
    extract::{SignedIn, Viewer},
    views::{layout::Page, repo::RepoHeader},
};
use askama::Template;
use askama_web::WebTemplate;
use axum::{
    Form, Router,
    extract::{Path, State},
    response::Redirect,
    routing::{get, post},
};
use rbit_localization::settings::protection::ProtectionMessage;
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/settings/tags", get(list).post(create))
        .route("/{owner}/{repo}/settings/tags/{id}/delete", post(delete))
}
#[derive(Template, WebTemplate)]
#[template(path = "protection/tags.html")]
struct TagsPage {
    page: Page,
    header: RepoHeader,
    rows: Vec<(i64, String)>,
    url: String,
}
async fn load(state: &AppState, viewer: &Viewer, owner: &str, repo: &str) -> WebResult<RepoPage> {
    let loaded = RepoPage::load(state, viewer, owner, repo).await?;
    if !loaded.ctx.access.can_admin() {
        return Err(WebError::NotFound);
    }
    Ok(loaded)
}
async fn list(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo)): Path<(String, String)>,
) -> WebResult<TagsPage> {
    let loaded = load(&state, &viewer, &owner, &repo).await?;
    let rows = state.services.protection.tag_rules(&loaded.ctx).await?.into_iter().map(|(id, pattern)| (id, pattern.to_string())).collect();
    let header = loaded.header(&state, "settings");
    let url = format!("{}/tags", header.settings_url);
    Ok(TagsPage {
        page: {
            let title = ProtectionMessage::TagsTitle.text(&page.locale);
            page.titled(title.as_str())
        },
        header,
        rows,
        url,
    })
}
#[derive(Deserialize)]
struct Input {
    pattern: String,
}
async fn create(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    viewer: Viewer,
    Path((owner, repo)): Path<(String, String)>,
    Form(input): Form<Input>,
) -> WebResult<Redirect> {
    let loaded = load(&state, &viewer, &owner, &repo).await?;
    state.services.protection.create_tag_rule(&loaded.ctx, &user, &input.pattern).await?;
    Ok(Redirect::to(&format!("{}/tags", loaded.header(&state, "settings").settings_url)))
}
#[derive(Deserialize)]
struct DeleteInput {
    confirm: Option<String>,
}
async fn delete(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    viewer: Viewer,
    Path((owner, repo, id)): Path<(String, String, i64)>,
    Form(input): Form<DeleteInput>,
) -> WebResult<Redirect> {
    let loaded = load(&state, &viewer, &owner, &repo).await?;
    if input.confirm.is_none() {
        return Err(rbit_services::ServiceError::Invalid("Confirm removal of tag protection".to_owned()).into());
    }
    state.services.protection.delete_tag_rule(&loaded.ctx, &user, id).await?;
    Ok(Redirect::to(&format!("{}/tags", loaded.header(&state, "settings").settings_url)))
}
