//! Repository content search pages and their bounded query inputs.
use super::RepoPage;
pub use crate::views::code_search::CodeSearchQuery;
use crate::{AppState, error::WebResult, extract::Viewer, views::layout::Page};
use axum::{
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};

pub async fn search(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<CodeSearchQuery>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &name).await?;
    let results = if query.q.is_empty() {
        rbit_git::CodeQuery::new(" ".into(), query.path.clone(), query.language.clone())
            .map_err(|message| crate::error::WebError::BadRequest(message.into()))?;
        rbit_git::CodeResults::default()
    } else {
        rbit_services::repositories::search_code(&loaded.ctx, query.q.clone(), query.path.clone(), query.language.clone()).await?
    };
    let matches = results
        .matches
        .into_iter()
        .map(|m| crate::views::code_search::Match {
            url: format!("{}#L{}", loaded.urls.blob(&results.commit, &m.path), m.line),
            path: m.path,
            line: m.line,
            snippet: m.snippet,
        })
        .collect();
    Ok(crate::views::code_search::SearchPage {
        page: crate::views::repo_text::titled(page, rbit_localization::RepoBrowseLabel::CodeSearch, &loaded.ctx.repo.full_name()),
        header: loaded.header(&state, "search"),
        query,
        matches,
        truncated: results.truncated,
        skipped: results.skipped_files,
        commit: results.commit,
    }
    .into_response())
}
