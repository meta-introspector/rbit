//! Origin-checked issue hierarchy and dependency forms for issues and pull requests.
use crate::{
    AppState,
    error::{WebError, WebResult},
    extract::SignedIn,
    views::timeline::ThreadUrls,
};
use axum::{
    Form, Router,
    extract::{Path, State},
    response::Redirect,
    routing::post,
};
use rbit_core::{IssueNumber, User};
use rbit_services::{IssueContext, issue_relations::RelationInput};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/issues/{number}/relations", post(add))
        .route("/{owner}/{repo}/issues/{number}/relations/{edge}/remove", post(remove))
}
async fn open(state: &AppState, user: &User, owner: &str, repo: &str, number: &str) -> WebResult<IssueContext> {
    let number = IssueNumber::parse(number).map_err(|_| WebError::NotFound)?;
    let repo = state.services.repositories.open(Some(user), owner, repo).await?;
    Ok(state.services.timeline.open(Some(user), repo, number).await?)
}
async fn add(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, repo, number)): Path<(String, String, String)>,
    Form(input): Form<RelationInput>,
) -> WebResult<Redirect> {
    let ctx = open(&state, &user, &owner, &repo, &number).await?;
    state.services.issue_relations.add(&user, ctx.issue.id, &input).await?;
    Ok(Redirect::to(&ThreadUrls::for_context(&ctx).page))
}
async fn remove(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, repo, number, edge)): Path<(String, String, String, i64)>,
) -> WebResult<Redirect> {
    let ctx = open(&state, &user, &owner, &repo, &number).await?;
    state.services.issue_relations.remove(&user, ctx.issue.id, edge).await?;
    Ok(Redirect::to(&ThreadUrls::for_context(&ctx).page))
}
