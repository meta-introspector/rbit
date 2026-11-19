//! Organization project pages; the shared page router supplies Origin checks and sessions.

use crate::{
    AppState,
    error::WebResult,
    extract::SignedIn,
    views::{
        layout::Page,
        org::{OrgHeader, OrgTab},
        projects::{ProjectPage, ProjectsPage},
    },
};
use axum::{
    Form, Router,
    extract::{Path, Query, State},
    response::Redirect,
    routing::{get, post},
};
use rbit_services::projects::{BoardInput, Filter};
use serde::Deserialize;

mod planning;

pub fn routes() -> Router<AppState> {
    Router::new()
        .merge(planning::routes())
        .route("/organizations/{owner}/projects", get(list).post(create))
        .route("/organizations/{owner}/projects/{id}", get(show).post(update))
        .route("/organizations/{owner}/projects/{id}/delete", post(delete))
        .route("/organizations/{owner}/projects/{id}/cards", post(add))
        .route("/organizations/{owner}/projects/{id}/cards/{issue}", post(move_card))
}

pub async fn list(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path(owner): Path<String>,
) -> WebResult<ProjectsPage> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    let boards = state.services.projects.list(&user, ctx.org.account.id.0).await?;
    Ok(ProjectsPage {
        page: page.titled("Projects"),
        header: OrgHeader::new(&ctx, OrgTab::Projects),
        boards,
        base: format!("/organizations/{}/projects", ctx.org.name()),
        can_manage: ctx.can_manage,
    })
}

#[derive(Deserialize)]
pub struct BoardForm {
    title: String,
    columns: String,
    #[serde(default)]
    revision: i64,
}
impl BoardForm {
    fn input(&self) -> BoardInput {
        BoardInput { title: self.title.clone(), columns: self.columns.lines().map(|line| line.trim().to_owned()).collect() }
    }
}

pub async fn create(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path(owner): Path<String>,
    Form(form): Form<BoardForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    let board = state.services.projects.create(&user, ctx.org.account.id.0, &form.input()).await?;
    Ok(Redirect::to(&format!("/organizations/{}/projects/{}", ctx.org.name(), board.id)))
}

pub async fn show(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path((owner, id)): Path<(String, i64)>,
    Query(values): Query<std::collections::HashMap<String, String>>,
) -> WebResult<ProjectPage> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    let filter = planning::filter(values)?;
    let data = state.services.projects.page(&user, ctx.org.account.id.0, id, &filter).await?;
    Ok(ProjectPage::new(page, OrgHeader::new(&ctx, OrgTab::Projects), data))
}

pub async fn update(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id)): Path<(String, i64)>,
    Form(form): Form<BoardForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.update(&user, ctx.org.account.id.0, id, form.revision, &form.input()).await?;
    Ok(Redirect::to(&format!("/organizations/{}/projects/{id}", ctx.org.name())))
}

#[derive(Deserialize)]
pub struct RevisionForm {
    revision: i64,
    confirm: Option<String>,
}
pub async fn delete(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id)): Path<(String, i64)>,
    Form(form): Form<RevisionForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    if form.confirm.as_deref() != Some("yes") {
        return Err(rbit_services::ServiceError::Invalid("confirm project deletion".to_owned()).into());
    }
    state.services.projects.delete(&user, ctx.org.account.id.0, id, form.revision).await?;
    Ok(Redirect::to(&format!("/organizations/{}/projects", ctx.org.name())))
}

#[derive(Deserialize)]
pub struct AddForm {
    repository: String,
    number: i64,
}
pub async fn add(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id)): Path<(String, i64)>,
    Form(form): Form<AddForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.add_named(&user, ctx.org.account.id.0, id, &form.repository, form.number).await?;
    Ok(Redirect::to(&format!("/organizations/{}/projects/{id}", ctx.org.name())))
}

#[derive(Deserialize)]
pub struct MoveForm {
    revision: i64,
    column: Option<i64>,
}
pub async fn move_card(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, issue)): Path<(String, i64, i64)>,
    Form(form): Form<MoveForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.move_card(&user, ctx.org.account.id.0, id, issue, form.revision, form.column).await?;
    Ok(Redirect::to(&format!("/organizations/{}/projects/{id}", ctx.org.name())))
}
