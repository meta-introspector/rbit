//! One repository: read, change, delete, and its branches, tags and commits.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rbit_core::{TokenScope, Visibility};
use rbit_git::RefKind;
use rbit_services::{RepoContext, ServiceError, repo_settings::UpdateRepository};
use serde::Deserialize;
use serde_json::Value;

use super::{
    auth::Caller,
    error::{ApiError, ApiResult},
    json,
    json::ApiJson,
    pagination::{Paged, Pager},
};
use crate::AppState;

/// `{base}/api/v1/repos/{owner}/{repo}` and `{base}/{owner}/{repo}`.
fn urls(state: &AppState, ctx: &RepoContext) -> (String, String) {
    let base = state.config.server.base_url();
    let full_name = ctx.repo.full_name();
    (format!("{base}/api/v1/repos/{full_name}"), format!("{base}/{full_name}"))
}

/// `GET /repos/{owner}/{repo}`
pub async fn show(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    Ok(Json(detail(&state, &caller, &ctx).await?))
}

/// The full repository object of single-repository responses: with `permissions` and,
/// for a fork, GitHub's `parent` (omitted when the caller may not read the parent or it
/// was deleted).
pub(super) async fn detail(state: &AppState, caller: &Caller, ctx: &RepoContext) -> ApiResult<Value> {
    let forks = &state.services.forks;
    let base = state.config.server.base_url();
    let fork = forks.is_fork(ctx.repo.repo.id).await?;
    let mut value = json::repository(&ctx.repo, Some(ctx.access), fork, base);
    if fork && let Some(parent) = forks.parent(caller.user(), ctx).await? {
        value["parent"] = json::repository(&parent, None, forks.is_fork(parent.repo.id).await?, base);
    }
    Ok(value)
}

/// The body of `PATCH /repos/{owner}/{repo}`; absent fields keep their value.
#[derive(Debug, Deserialize)]
pub struct RepositoryChanges {
    name: Option<String>,
    /// A string sets it, `null` clears it.
    #[serde(default, deserialize_with = "json::present")]
    description: Option<Value>,
    private: Option<bool>,
    visibility: Option<String>,
    default_branch: Option<String>,
    /// `true` archives the repository, `false` unarchives it.
    archived: Option<bool>,
}

/// `PATCH /repos/{owner}/{repo}`: administrators with the `admin` scope. `archived` is
/// applied after the other changes, so one request can describe and archive a repository.
pub async fn update(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    ApiJson(changes): ApiJson<RepositoryChanges>,
) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    caller.require(TokenScope::Admin)?;
    require_admin(&ctx)?;

    let current = &ctx.repo.repo;
    let description = match changes.description {
        None => current.description.clone().unwrap_or_default(),
        Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text,
        Some(_) => return Err(ApiError::invalid("description", "description must be a string or null")),
    };
    let visibility = match (changes.visibility.as_deref(), changes.private) {
        (Some(value), _) => {
            Visibility::parse(value).ok_or_else(|| ApiError::invalid("visibility", "visibility must be public or private"))?
        }
        (None, Some(true)) => Visibility::Private,
        (None, Some(false)) => Visibility::Public,
        (None, None) => current.visibility,
    };
    let settings = &state.services.repo_settings;
    let input = UpdateRepository { description, visibility, default_branch: changes.default_branch };
    settings.update(&ctx, input).await.map_err(|error| ApiError::service(error, "body"))?;

    let mut final_name = current.name.to_string();
    if let Some(new_name) = changes.name.filter(|new_name| new_name.trim() != current.name.as_str()) {
        final_name = settings.rename(&ctx, &new_name).await.map_err(|error| ApiError::service(error, "name"))?.to_string();
    }
    let mut ctx = caller.open_repo(&state, ctx.repo.owner.name.as_str(), &final_name).await?;
    if let Some(archived) = changes.archived {
        let user = caller.require_user()?;
        if settings.set_archived(user, &ctx, archived).await? {
            ctx = caller.open_repo(&state, ctx.repo.owner.name.as_str(), &final_name).await?;
        }
    }
    Ok(Json(detail(&state, &caller, &ctx).await?))
}

/// `DELETE /repos/{owner}/{repo}`: administrators with the `admin` scope.
pub async fn delete(State(state): State<AppState>, caller: Caller, Path((owner, name)): Path<(String, String)>) -> ApiResult<StatusCode> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    caller.require(TokenScope::Admin)?;
    require_admin(&ctx)?;
    let user = caller.require_user()?;
    state.services.repo_settings.delete(user, &ctx, &ctx.repo.full_name()).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn require_admin(ctx: &RepoContext) -> ApiResult<()> {
    if ctx.access.can_admin() { Ok(()) } else { Err(ApiError::forbidden("You must be an administrator of this repository")) }
}

async fn refs(state: &AppState, ctx: &RepoContext, kind: RefKind) -> ApiResult<Vec<Value>> {
    let (api, _) = urls(state, ctx);
    let details = ctx.reader.ref_details(kind).await.map_err(ServiceError::from)?;
    Ok(details.iter().map(|detail| json::git_ref(detail, &api, kind == RefKind::Branch)).collect())
}

/// `GET /repos/{owner}/{repo}/branches`
pub async fn branches(
    State(state): State<AppState>,
    caller: Caller,
    pager: Pager,
    Path((owner, name)): Path<(String, String)>,
) -> ApiResult<Paged<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    Ok(pager.page_of(refs(&state, &ctx, RefKind::Branch).await?))
}

/// `GET /repos/{owner}/{repo}/branches/{branch}`; the name may contain `/`.
pub async fn branch(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, branch)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    let found = refs(&state, &ctx, RefKind::Branch).await?.into_iter().find(|value| value["name"] == branch.as_str());
    found.map(Json).ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "Branch not found"))
}

/// `GET /repos/{owner}/{repo}/tags`
pub async fn tags(
    State(state): State<AppState>,
    caller: Caller,
    pager: Pager,
    Path((owner, name)): Path<(String, String)>,
) -> ApiResult<Paged<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    Ok(pager.page_of(refs(&state, &ctx, RefKind::Tag).await?))
}

#[derive(Debug, Deserialize)]
pub struct CommitsQuery {
    sha: Option<String>,
}

/// `GET /repos/{owner}/{repo}/commits?sha=`: history from `sha` (default branch by
/// default), newest first. There is no total, so `Link` has no `last`.
pub async fn commits(
    State(state): State<AppState>,
    caller: Caller,
    pager: Pager,
    Path((owner, name)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<CommitsQuery>,
) -> ApiResult<Paged<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    if ctx.reader.is_empty().await.map_err(ServiceError::from)? {
        return Err(ApiError::new(StatusCode::CONFLICT, "Git Repository is empty."));
    }
    let revision = query.sha.filter(|sha| !sha.is_empty()).unwrap_or_else(|| ctx.repo.repo.default_branch.clone());
    let mut history =
        ctx.reader.history(revision, pager.offset(), pager.limit() + 1).await.map_err(|error| match ServiceError::from(error) {
            ServiceError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "No commit found for that revision"),
            other => other.into(),
        })?;
    let has_next = history.len() > pager.limit();
    history.truncate(pager.limit());
    let (api, html) = urls(&state, &ctx);
    Ok(pager.paged(history.iter().map(|commit| json::commit(commit, &api, &html)).collect(), None, has_next))
}

/// `GET /repos/{owner}/{repo}/commits/{ref}`: the commit a branch, tag or id names.
pub async fn commit(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name, reference)): Path<(String, String, String)>,
) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "No commit found for that revision");
    let history = ctx.reader.history(reference, 0, 1).await.map_err(|error| match ServiceError::from(error) {
        ServiceError::NotFound => not_found(),
        other => other.into(),
    })?;
    let (api, html) = urls(&state, &ctx);
    history.first().map(|commit| Json(json::commit(commit, &api, &html))).ok_or_else(not_found)
}

/// `GET /repos/{owner}/{repo}/search/code`: literal default-branch content search.
pub async fn search_code(
    State(state): State<AppState>,
    caller: Caller,
    Path((owner, name)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<crate::views::code_search::CodeSearchQuery>,
) -> ApiResult<Json<Value>> {
    let ctx = caller.open_repo(&state, &owner, &name).await?;
    let results = rbit_services::repositories::search_code(&ctx, query.q, query.path, query.language).await?;
    let urls = crate::urls::RepoUrls::new(ctx.repo.owner.name.as_str(), ctx.repo.repo.name.as_str());
    let matches: Vec<Value> = results
        .matches
        .iter()
        .map(|m| {
            serde_json::json!({
                "path": m.path, "line": m.line, "snippet": m.snippet,
                "html_url": format!("{}{}#L{}", state.config.server.base_url(), urls.blob(&results.commit, &m.path), m.line),
            })
        })
        .collect();
    Ok(Json(serde_json::json!({"commit": results.commit, "matches": matches,
        "truncated": results.truncated, "skipped_files": results.skipped_files})))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use rbit_core::{TokenScope, Visibility};
    use serde_json::json;

    use super::super::test_support::Fixture;

    #[tokio::test]
    async fn administrators_archive_and_unarchive_through_patch() {
        let fixture = Fixture::new(|_| {}).await;
        let alice = fixture.user("alice").await;
        fixture.repo(&alice, "demo", Visibility::Public).await;
        let admin = fixture.token(&alice, TokenScope::Admin).await;
        let write = fixture.token(&alice, TokenScope::Write).await;
        let repo = "/api/v1/repos/alice/demo";

        let (_, shown) = fixture.send("GET", repo, None, None).await;
        assert_eq!(shown["archived"], false);
        let (response, _) = fixture.send("PATCH", repo, Some(&write), Some(json!({ "archived": true }))).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "archiving needs the admin scope");

        let (response, archived) = fixture.send("PATCH", repo, Some(&admin), Some(json!({ "archived": true }))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(archived["archived"], true);
        let (response, refused) = fixture.send("POST", &format!("{repo}/issues"), Some(&write), Some(json!({ "title": "Bug" }))).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(refused["message"], "Repository was archived so is read-only.");
        let (response, _) = fixture.send("GET", &format!("{repo}/issues"), None, None).await;
        assert_eq!(response.status(), StatusCode::OK, "reading still works");

        let (_, unarchived) = fixture.send("PATCH", repo, Some(&admin), Some(json!({ "archived": false, "description": "Back" }))).await;
        assert_eq!((unarchived["archived"].as_bool(), unarchived["description"].as_str()), (Some(false), Some("Back")));
        let (response, _) = fixture.send("POST", &format!("{repo}/issues"), Some(&write), Some(json!({ "title": "Bug" }))).await;
        assert_eq!(response.status(), StatusCode::CREATED);
    }
}

#[cfg(test)]
mod code_search_tests {
    use super::super::test_support::Fixture;
    use rbit_core::{TokenScope, Visibility};
    #[tokio::test]
    async fn code_search_enforces_repository_visibility_and_validates_input() {
        let f = Fixture::new(|_| {}).await;
        let owner = f.user("search-owner").await;
        let other = f.user("search-other").await;
        f.repo(&owner, "private", Visibility::Private).await;
        f.repo(&owner, "public", Visibility::Public).await;
        let token = f.token(&other, TokenScope::Read).await;
        let (hidden, _) = f.send("GET", "/api/v1/repos/search-owner/private/search/code?q=secret", Some(&token), None).await;
        assert_eq!(hidden.status(), 404);
        let (invalid, _) = f.send("GET", "/api/v1/repos/search-owner/public/search/code?q=", Some(&token), None).await;
        assert_eq!(invalid.status(), 422);
        let (empty, body) = f.send("GET", "/api/v1/repos/search-owner/public/search/code?q=needle", Some(&token), None).await;
        assert_eq!(empty.status(), 200);
        assert_eq!(body["matches"], serde_json::json!([]));
        let ctx = f.services.repositories.open(Some(&owner), "search-owner", "public").await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "init.defaultBranch=main"])
                    .args(args)
                    .current_dir(tmp.path())
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        };
        git(&["init", "-q"]);
        std::fs::write(tmp.path().join("test.rs"), "needle <script>alert(1)</script>\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "fixture"]);
        git(&["push", ctx.disk_path.to_str().unwrap(), "HEAD:refs/heads/main"]);
        let (found, json) = f.send("GET", "/api/v1/repos/search-owner/public/search/code?q=needle&language=Rust", Some(&token), None).await;
        assert_eq!(found.status(), 200);
        assert_eq!(json["matches"][0]["line"], 1);
        let commit = json["commit"].as_str().unwrap();
        assert!(json["matches"][0]["html_url"].as_str().unwrap().contains(commit));
        let (page, html) = f.raw("GET", "/search-owner/public/search?q=needle", None, None, None).await;
        assert_eq!(page.status(), 200);
        let html = String::from_utf8(html.to_vec()).unwrap();
        assert!(html.contains("&lt;script&gt;") || html.contains("&#60;script&#62;"), "{html}");
        assert!(!html.contains("<script>alert(1)</script>"));
        let (hidden_page, _) = f.raw("GET", "/search-owner/private/search?q=secret", None, None, None).await;
        assert_eq!(hidden_page.status(), 404);
    }
}
