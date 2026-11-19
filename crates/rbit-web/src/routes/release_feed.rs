//! Small credential-free release metadata for public update checkers.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::header,
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;

use crate::{
    AppState,
    error::{WebError, WebResult},
    extract::Viewer,
    routes::repo::RepoPage,
};

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/{owner}/{repo}/releases/latest.json", get(latest))
}

#[derive(Serialize)]
struct Feed {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

/// Resolve as an anonymous reader even when a browser sends an administrator session.
/// This endpoint shares browser visibility policy and exposes neither notes nor assets.
async fn latest(State(state): State<AppState>, Path((owner, repo)): Path<(String, String)>) -> WebResult<Response> {
    if state.config.server.require_sign_in {
        return Err(WebError::NotFound);
    }
    let loaded = RepoPage::load(&state, &Viewer::default(), &owner, &repo).await?;
    let release = state.services.releases.latest(&loaded.ctx).await?.ok_or(WebError::NotFound)?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(Feed { tag_name: release.tag_name, draft: release.is_draft, prerelease: release.is_prerelease }),
    )
        .into_response())
}
