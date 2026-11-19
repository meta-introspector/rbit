//! REST API: Releases and their attached files: `/repos/{owner}/{repo}/releases…`.
//!
//! Milestone 3 workstream A (API continuation) owns this file and `openapi/releases.json`,
//! the `OpenAPI` fragment documenting exactly these routes (merged into the served
//! document).
//!
//! A child of `api`, so it can use the parent's private `route()` helper, `Caller`,
//! `Pager`, `ApiError` and `ApiJson`.
//!
//! Shapes follow GitHub's (the same release object webhooks send). Everything goes
//! through [`rbit_services::ReleaseService`], so its rules hold: readers see published
//! releases, drafts exist only for writers (404 for everyone else), writers create,
//! change and delete releases and files. Scopes: `read` for every `GET`, `write` for
//! everything else — deleting a release or a file is a writer's action, like deleting a
//! label.
//!
//! Files are uploaded like on GitHub's `upload_url`: `POST …/releases/{id}/assets?name=`
//! with the file as the raw request body. That route and the download
//! (`GET …/releases/assets/{id}` with `Accept: application/octet-stream`) are
//! [streaming](super::ApiRoute::streaming): no 1 MiB body limit, no 30 s timeout, no
//! compression. The upload streams to disk under `[releases] max_asset_bytes` (413 once
//! it is exceeded, at once when `Content-Length` says so) and
//! `[releases] upload_timeout_seconds` (408). Documented deviations: no
//! `target_commitish` in responses (rbit does not record it), file `label`s are accepted
//! and ignored, and files cannot be renamed.

use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::TryStreamExt;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rbit_core::{
    TokenScope,
    release::{ReleaseAssetId, ReleaseId},
};
use rbit_services::{
    RepoContext, ServiceError,
    releases::{ReleaseActor, ReleaseDetail, ReleaseInput, human_bytes},
    tokens::Right,
    webhooks::payloads,
};
use serde::Deserialize;
use serde_json::Value;
use tokio_util::io::{ReaderStream, StreamReader};

use super::{
    ApiMethod::{Delete, Get, Patch, Post},
    ApiRoute,
    auth::Caller,
    error::{ApiError, ApiResult},
    json::ApiJson,
    pagination::{Paged, Pager},
    route,
};
use crate::AppState;

/// This area's endpoints, appended to [`super::table`].
pub(super) fn routes() -> Vec<ApiRoute> {
    let releases = |suffix: &str| format!("/repos/{{owner}}/{{repo}}/releases{suffix}");
    vec![
        route(Get, releases(""), list),
        route(Post, releases(""), create),
        route(Get, releases("/latest"), latest),
        route(Get, releases("/tags/{*tag}"), by_tag),
        route(Get, releases("/{release_id}"), show),
        route(Patch, releases("/{release_id}"), update),
        route(Delete, releases("/{release_id}"), delete),
        route(Get, releases("/{release_id}/assets"), assets),
        route(Post, releases("/{release_id}/assets"), upload).streaming(),
        route(Get, releases("/assets/{asset_id}"), asset).streaming(),
        route(Delete, releases("/assets/{asset_id}"), delete_asset),
    ]
}

/// The path parameters of every endpoint.
#[derive(Debug, Deserialize)]
pub struct ReleasePath {
    owner: String,
    repo: String,
    release_id: Option<String>,
    asset_id: Option<String>,
    tag: Option<String>,
}

/// A positive numeric id from the path; anything else is 404.
fn parse_id(id: Option<&str>) -> ApiResult<i64> {
    id.and_then(|id| id.parse::<i64>().ok()).filter(|id| *id > 0).ok_or_else(ApiError::not_found)
}

impl ReleasePath {
    fn release(&self) -> ApiResult<ReleaseId> {
        parse_id(self.release_id.as_deref()).map(ReleaseId)
    }

    fn asset(&self) -> ApiResult<ReleaseAssetId> {
        parse_id(self.asset_id.as_deref()).map(ReleaseAssetId)
    }
}

/// The JSON of `releases` (one query for their authors and uploaders).
async fn values(state: &AppState, ctx: &RepoContext, releases: &[ReleaseDetail]) -> ApiResult<Vec<Value>> {
    let people = state.services.releases.people(releases).await?;
    let base = state.config.server.base_url();
    Ok(releases
        .iter()
        .map(|detail| {
            let author = detail.release.author.as_ref().and_then(|author| people.iter().find(|account| account.id == author.id));
            payloads::release(&detail.release, &detail.assets, author, &people, &ctx.repo, base)
        })
        .collect())
}

async fn value(state: &AppState, ctx: &RepoContext, release: &ReleaseDetail) -> ApiResult<Value> {
    Ok(values(state, ctx, std::slice::from_ref(release)).await?.pop().unwrap_or_default())
}

/// The JSON of file `id` of `release` (as it appears in the release's `assets`).
async fn asset_value(state: &AppState, ctx: &RepoContext, release: &ReleaseDetail, id: ReleaseAssetId) -> ApiResult<Value> {
    let release = value(state, ctx, release).await?;
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    assets.into_iter().find(|asset| asset["id"] == id.0).ok_or_else(ApiError::not_found)
}

// Actions: releases from workflows.
/// Who writes: a workflow task (checked when authenticated), or a person with `write` (and `push`, when limited: tags).
fn writer(caller: &Caller) -> ApiResult<ReleaseActor<'_>> {
    match caller.task() {
        Some(grant) => Ok(ReleaseActor::Task(grant)),
        None => caller.require_right(Right::Push).and_then(|()| caller.require(TokenScope::Write)).map(ReleaseActor::User),
    }
}

/// Writers only (403 for readers), with the `write` scope.
fn require_writer(ctx: &RepoContext) -> ApiResult<()> {
    if ctx.access.can_write() { Ok(()) } else { Err(ApiError::forbidden("You need write access to this repository")) }
}

/// `201 Created` with `Location` set to the body's `url`.
fn created(body: Value) -> Response {
    let location = body["url"].as_str().unwrap_or_default().to_owned();
    (StatusCode::CREATED, [(header::LOCATION, location)], Json(body)).into_response()
}

/// Validation messages say what they are about; attribute them to the closest field.
fn field_error(error: ServiceError) -> ApiError {
    let field = match &error {
        ServiceError::Invalid(message) if message.contains("branch or commit") => "target_commitish",
        ServiceError::Invalid(message) if message.contains("title") => "name",
        ServiceError::Invalid(message) if message.contains("notes") => "body",
        _ => "tag_name",
    };
    ApiError::service(error, field)
}

// ------------------------------------------------------------------ releases

/// `GET /repos/{owner}/{repo}/releases`
pub async fn list(State(state): State<AppState>, caller: Caller, pager: Pager, Path(path): Path<ReleasePath>) -> ApiResult<Paged<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let limit = u32::try_from(pager.limit()).unwrap_or(u32::MAX);
    let offset = u32::try_from(pager.offset()).unwrap_or(u32::MAX);
    let (releases, total) = state.services.releases.page(&ctx, limit, offset).await?;
    Ok(pager.paged(values(&state, &ctx, &releases).await?, Some(total), false))
}

/// The body of `POST` and `PATCH`; absent fields keep their value (or the default of a
/// new release).
#[derive(Debug, Default, Deserialize)]
pub struct ReleaseBody {
    tag_name: Option<String>,
    target_commitish: Option<String>,
    name: Option<String>,
    body: Option<String>,
    draft: Option<bool>,
    prerelease: Option<bool>,
}

/// `POST /repos/{owner}/{repo}/releases`: `201 Created`. A missing tag is created at
/// `target_commitish`, by default the default branch (as on GitHub).
pub async fn create(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<ReleasePath>,
    ApiJson(body): ApiJson<ReleaseBody>,
) -> ApiResult<Response> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let user = writer(&caller)?;
    require_writer(&ctx)?;
    let tag_name =
        body.tag_name.filter(|tag| !tag.trim().is_empty()).ok_or_else(|| ApiError::invalid("tag_name", "tag_name is required"))?;
    let input = ReleaseInput {
        tag_name,
        name: body.name.unwrap_or_default(),
        notes: body.body.unwrap_or_default(),
        is_draft: body.draft.unwrap_or(false),
        is_prerelease: body.prerelease.unwrap_or(false),
        target: body.target_commitish.filter(|target| !target.trim().is_empty()).unwrap_or_else(|| ctx.repo.repo.default_branch.clone()),
    };
    let releases = &state.services.releases;
    let release = releases.create(&ctx, user, input).await.map_err(field_error)?;
    let detail = releases.by_id(&ctx, release.id).await?;
    Ok(created(value(&state, &ctx, &detail).await?))
}

/// `GET /repos/{owner}/{repo}/releases/latest`: full releases before pre-releases, never
/// drafts; 404 when there is none.
pub async fn latest(State(state): State<AppState>, caller: Caller, Path(path): Path<ReleasePath>) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let releases = &state.services.releases;
    let release = releases.latest(&ctx).await?.ok_or_else(ApiError::not_found)?;
    let detail = releases.by_id(&ctx, release.id).await?;
    Ok(Json(value(&state, &ctx, &detail).await?))
}

/// `GET /repos/{owner}/{repo}/releases/tags/{tag}`; the tag may contain `/`.
pub async fn by_tag(State(state): State<AppState>, caller: Caller, Path(path): Path<ReleasePath>) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let detail = state.services.releases.by_tag(&ctx, path.tag.as_deref().unwrap_or_default()).await?;
    Ok(Json(value(&state, &ctx, &detail).await?))
}

/// `GET /repos/{owner}/{repo}/releases/{release_id}`
pub async fn show(State(state): State<AppState>, caller: Caller, Path(path): Path<ReleasePath>) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let detail = state.services.releases.by_id(&ctx, path.release()?).await?;
    Ok(Json(value(&state, &ctx, &detail).await?))
}

/// `PATCH /repos/{owner}/{repo}/releases/{release_id}`: absent fields keep their value;
/// publishing a draft sends the `release` webhook (once).
pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<ReleasePath>,
    ApiJson(body): ApiJson<ReleaseBody>,
) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let user = writer(&caller)?;
    let releases = &state.services.releases;
    let current = releases.by_id(&ctx, path.release()?).await?.release;
    require_writer(&ctx)?;
    let input = ReleaseInput {
        tag_name: body.tag_name.unwrap_or(current.tag_name),
        name: body.name.unwrap_or(current.name),
        notes: body.body.unwrap_or(current.notes),
        is_draft: body.draft.unwrap_or(current.is_draft),
        is_prerelease: body.prerelease.unwrap_or(current.is_prerelease),
        target: String::new(),
    };
    releases.update(&ctx, user, current.id, input).await.map_err(field_error)?;
    let detail = releases.by_id(&ctx, current.id).await?;
    Ok(Json(value(&state, &ctx, &detail).await?))
}

/// `DELETE /repos/{owner}/{repo}/releases/{release_id}`: `204`; its files go too, the tag
/// stays.
pub async fn delete(State(state): State<AppState>, caller: Caller, Path(path): Path<ReleasePath>) -> ApiResult<StatusCode> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    writer(&caller)?;
    let releases = &state.services.releases;
    let release = releases.by_id(&ctx, path.release()?).await?.release;
    require_writer(&ctx)?;
    releases.delete(&ctx, release.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------------ attached files

/// `GET /repos/{owner}/{repo}/releases/{release_id}/assets`, by name.
pub async fn assets(State(state): State<AppState>, caller: Caller, pager: Pager, Path(path): Path<ReleasePath>) -> ApiResult<Paged<Value>> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let detail = state.services.releases.by_id(&ctx, path.release()?).await?;
    let release = value(&state, &ctx, &detail).await?;
    Ok(pager.page_of(release["assets"].as_array().cloned().unwrap_or_default()))
}

/// The query of an upload; GitHub's `label` is accepted and ignored.
#[derive(Debug, Deserialize)]
pub struct UploadQuery {
    name: Option<String>,
}

/// The declared `Content-Length`, if any.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers.get(header::CONTENT_LENGTH)?.to_str().ok()?.trim().parse().ok()
}

/// `POST /repos/{owner}/{repo}/releases/{release_id}/assets?name=`: the request body is
/// the file, streamed to disk (see the module docs for the limits); `201 Created`.
pub async fn upload(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<ReleasePath>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Body,
) -> ApiResult<Response> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let user = writer(&caller)?;
    let releases = &state.services.releases;
    let release = releases.by_id(&ctx, path.release()?).await?.release;
    require_writer(&ctx)?;
    let name = query.name.filter(|name| !name.is_empty()).ok_or_else(|| ApiError::invalid("name", "name is required"))?;
    let config = &state.config.releases;
    let too_large =
        || ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, format!("Release files can be at most {}", human_bytes(config.max_asset_bytes)));
    if content_length(&headers).is_some_and(|length| length > config.max_asset_bytes) {
        return Err(too_large());
    }
    let content_type = headers.get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or_default();

    // Count what arrives, to tell "too large" from other refusals.
    let received = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&received);
    let stream = body
        .into_data_stream()
        .inspect_ok(move |chunk| {
            counter.fetch_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX), Ordering::Relaxed);
        })
        .map_err(io::Error::other);
    let upload = releases.upload_asset(&ctx, user, release.id, &name, content_type, StreamReader::new(stream));
    let timeout = Duration::from_secs(config.upload_timeout_seconds);
    let Ok(uploaded) = tokio::time::timeout(timeout, upload).await else {
        return Err(ApiError::new(StatusCode::REQUEST_TIMEOUT, format!("The upload took longer than {} seconds", timeout.as_secs())));
    };
    let asset = match uploaded {
        Ok(asset) => asset,
        Err(ServiceError::Invalid(_)) if received.load(Ordering::Relaxed) > config.max_asset_bytes => return Err(too_large()),
        Err(error) => return Err(ApiError::service(error, "name")),
    };
    let (detail, asset) = releases.asset(&ctx, asset.id).await?;
    Ok(created(asset_value(&state, &ctx, &detail, asset.id).await?))
}

/// Whether `Accept` asks for the file itself.
fn wants_bytes(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|media| media.split(';').next().is_some_and(|media| media.trim().eq_ignore_ascii_case("application/octet-stream")))
}

/// `GET /repos/{owner}/{repo}/releases/assets/{asset_id}`: the file's JSON, or with
/// `Accept: application/octet-stream` the file itself (streamed, counted as a download).
pub async fn asset(
    State(state): State<AppState>,
    caller: Caller,
    Path(path): Path<ReleasePath>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    let releases = &state.services.releases;
    let id = path.asset()?;
    if !wants_bytes(&headers) {
        let (detail, _) = releases.asset(&ctx, id).await?;
        return Ok(Json(asset_value(&state, &ctx, &detail, id).await?).into_response());
    }
    let download = releases.open_asset_by_id(&ctx, id).await?;
    let asset = &download.asset;
    let disposition = format!("attachment; filename*=UTF-8''{}", utf8_percent_encode(&asset.name, NON_ALPHANUMERIC));
    // Percent-encoded names and hex digests are always valid header values.
    let header_value = |value: String| {
        HeaderValue::from_str(&value).map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "An unexpected error occurred."))
    };
    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream")),
        (header::CONTENT_DISPOSITION, header_value(disposition)?),
        (header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox; default-src 'none'")),
        (header::CONTENT_LENGTH, HeaderValue::from(asset.size)),
        (header::ETAG, header_value(format!("\"{}\"", asset.sha256_hex()))?),
    ];
    Ok((headers, Body::from_stream(ReaderStream::new(download.file))).into_response())
}

/// `DELETE /repos/{owner}/{repo}/releases/assets/{asset_id}`: `204`.
pub async fn delete_asset(State(state): State<AppState>, caller: Caller, Path(path): Path<ReleasePath>) -> ApiResult<StatusCode> {
    let ctx = caller.open_repo(&state, &path.owner, &path.repo).await?;
    writer(&caller)?;
    let releases = &state.services.releases;
    let (detail, asset) = releases.asset(&ctx, path.asset()?).await?;
    require_writer(&ctx)?;
    releases.delete_asset(&ctx, detail.release.id, asset.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
