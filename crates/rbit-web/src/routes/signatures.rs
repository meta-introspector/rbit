//! Repository signing trust administration and on-demand verification pages.
use super::repo::RepoPage;
use crate::{
    AppState,
    error::WebResult,
    extract::{SignedIn, Viewer},
    views::{
        layout::Page,
        signatures::{KeyInput, SignatureFragment, SignaturePage, SigningKeysPage},
    },
};
use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/settings/signing-keys", get(keys).post(add_key))
        .route("/{owner}/{repo}/settings/signing-keys/{id}/delete", post(delete_key))
        .route("/{owner}/{repo}/signature/{id}", get(show))
        .route("/{owner}/{repo}/signature/tag/{*tag}", get(tag))
}
#[derive(Debug, Default, Deserialize)]
pub struct SignatureQuery {
    fragment: Option<String>,
}
async fn keys(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo)): Path<(String, String)>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    let keys = state.services.signatures.list_keys(&loaded.ctx).await?;
    let page = crate::views::repo_text::titled(page, rbit_localization::RepoBrowseLabel::SigningKeys, &loaded.ctx.repo.full_name());
    Ok(SigningKeysPage { page, header: loaded.header(&state, "settings"), keys }.into_response())
}
async fn add_key(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((owner, repo)): Path<(String, String)>,
    Form(input): Form<KeyInput>,
) -> WebResult<Redirect> {
    let ctx = state.services.repositories.open(Some(&signed.0), &owner, &repo).await?;
    state.services.signatures.add_key(&signed.0, &ctx, input.name, input.kind, input.public_key).await?;
    Ok(Redirect::to(&format!("/{}/{}/settings/signing-keys", ctx.repo.owner.name, ctx.repo.repo.name)))
}
async fn delete_key(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((owner, repo, id)): Path<(String, String, i64)>,
) -> WebResult<Redirect> {
    let ctx = state.services.repositories.open(Some(&signed.0), &owner, &repo).await?;
    state.services.signatures.delete_key(&signed.0, &ctx, id).await?;
    Ok(Redirect::to(&format!("/{}/{}/settings/signing-keys", ctx.repo.owner.name, ctx.repo.repo.name)))
}
async fn tag(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo, tag)): Path<(String, String, String)>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    let id = loaded.ctx.reader.signature_tag_id(tag).await.map_err(rbit_services::ServiceError::from)?;
    render(&state, loaded, page, id, false).await
}
async fn show(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, repo, id)): Path<(String, String, String)>,
    Query(query): Query<SignatureQuery>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &repo).await?;
    render(&state, loaded, page, id, query.fragment.is_some()).await
}
async fn render(state: &AppState, loaded: RepoPage, page: Page, id: String, fragment: bool) -> WebResult<Response> {
    let result = state.services.signatures.verify(&loaded.ctx, id.clone()).await?;
    let signature = SignatureFragment { locale: page.locale, state: result.state, url: format!("{}/signature/{id}", loaded.urls.home()) };
    let response = if fragment {
        signature.into_response()
    } else {
        let page = crate::views::repo_text::titled(page, rbit_localization::RepoBrowseLabel::SignatureTitle, &loaded.ctx.repo.full_name());
        SignaturePage { page, header: loaded.header(state, "code"), object_id: result.object_id, kind: result.kind, signature }
            .into_response()
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], response).into_response())
}
