//! Repository security pages and administrator triage forms.
mod automation;
use super::repo::RepoPage;
use crate::{
    AppState,
    error::WebResult,
    extract::{SignedIn, Viewer},
    views::{layout::Page, repository_security::SecurityPage},
};
use axum::{
    Form, Router,
    extract::{Path, Query, State},
    http::header,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use rbit_services::security::{SecurityService, Settings};
use serde::Deserialize;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/security", get(show))
        .route("/{owner}/{repo}/security/automation", post(automation::configure))
        .route("/{owner}/{repo}/security/automation/scans", post(automation::scan))
        .route("/{owner}/{repo}/security/automation/updates", post(automation::propose))
        .route("/{owner}/{repo}/security/settings", post(configure))
        .route("/{owner}/{repo}/security/findings/{id}/dismissal", post(dismiss))
}
#[derive(Debug, Default, Deserialize)]
struct Selection {
    scan: Option<i64>,
    #[serde(default)]
    offset: u32,
}
#[derive(Debug, Deserialize)]
struct Configuration {
    scanner: String,
    workflow: String,
    require_scan: Option<String>,
    minimum_severity: u8,
}
#[derive(Debug, Deserialize)]
struct Dismissal {
    reason: String,
    #[serde(default)]
    note: String,
    scan: i64,
}
async fn show(
    State(state): State<AppState>,
    viewer: Viewer,
    page: Page,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<Selection>,
) -> WebResult<Response> {
    let loaded = RepoPage::load(&state, &viewer, &owner, &name).await?;
    SecurityService::require_read(&loaded.ctx)?;
    let configuration = state.services.security.configuration(&loaded.ctx).await?;
    let generation = configuration.as_ref().map(|config| config.generation);
    let config = configuration.map_or(Settings { minimum_severity: 3, ..Settings::default() }, |config| config.settings());
    let automatic = state.services.security.automation_configuration(&loaded.ctx).await?;
    let scans = state.services.security.scans(&loaded.ctx).await?;
    let selected = query.scan.or_else(|| scans.first().map(|scan| scan.id));
    let (selected, findings) = match selected {
        Some(id) => {
            let (scan, findings) = state.services.security.findings(&loaded.ctx, id, query.offset).await?;
            (Some(scan), findings)
        }
        None => (None, Vec::new()),
    };
    let tip = loaded.ctx.reader.commit_id(loaded.ctx.repo.repo.default_branch.clone()).await.ok();
    let outdated_configuration = selected.as_ref().is_some_and(|scan| {
        if scan.channel == "external" {
            Some(scan.generation) != generation || scan.scanner != config.scanner
        } else {
            automatic
                .as_ref()
                .is_none_or(|auto| auto.generation != scan.generation || !auto.settings.channels().any(|scanner| scanner == scan.scanner))
                || scan.created_at < rbit_core::Timestamp::now().seconds() - rbit_services::security::automation::FRESH_SECONDS
        }
    });
    let mut packages = Vec::new();
    if loaded.ctx.access.can_admin()
        && let Some(scan) = &selected
        && scan.channel == "npm"
        && scan.state == "complete"
        && tip.as_deref() == Some(&scan.commit_sha)
        && !outdated_configuration
    {
        packages = state.services.security.dependency_inventory(&loaded.ctx, &scan.commit_sha).await.unwrap_or_default();
        packages.retain(|p| p.location == format!("node_modules/{}", p.name));
    }
    let updates = state.services.security.remediation_history(&loaded.ctx).await?;
    let automatic = automatic
        .map_or(rbit_services::security::automation::AutomationSettings { minimum_severity: 3, ..Default::default() }, |config| {
            config.settings
        });
    let historical = selected.as_ref().is_some_and(|scan| tip.as_deref() != Some(&scan.commit_sha));
    let page = crate::views::repo_text::titled(page, rbit_localization::RepoBrowseLabel::Security, &loaded.ctx.repo.full_name());
    let next = findings.len() == 50;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        SecurityPage {
            page,
            header: loaded.header(&state, "security"),
            config,
            automatic,
            tip,
            packages,
            updates,
            scans,
            selected,
            findings,
            historical,
            stale: outdated_configuration,
            offset: query.offset,
            next,
        },
    )
        .into_response())
}
async fn configure(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((owner, name)): Path<(String, String)>,
    Form(input): Form<Configuration>,
) -> WebResult<Redirect> {
    let repo = state.services.repositories.open(Some(&signed.0), &owner, &name).await?;
    state
        .services
        .security
        .configure(
            &signed.0,
            &repo,
            Settings {
                scanner: input.scanner,
                workflow: input.workflow,
                require_scan: input.require_scan.is_some(),
                minimum_severity: input.minimum_severity,
            },
        )
        .await?;
    Ok(Redirect::to(&format!("/{}/security", repo.repo.full_name())))
}
async fn dismiss(
    State(state): State<AppState>,
    signed: SignedIn,
    Path((owner, name, id)): Path<(String, String, i64)>,
    Form(input): Form<Dismissal>,
) -> WebResult<Redirect> {
    let repo = state.services.repositories.open(Some(&signed.0), &owner, &name).await?;
    state.services.security.dismiss(&signed.0, &repo, id, &input.reason, &input.note).await?;
    Ok(Redirect::to(&format!("/{}/security?scan={}", repo.repo.full_name(), input.scan)))
}
