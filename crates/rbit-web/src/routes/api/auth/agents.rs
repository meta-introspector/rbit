//! Closed-world API admission for delegated identities; repository grants are checked separately.

use rbit_core::issue::IssueKind;
use rbit_services::tokens::{Right, TokenLimit};

use super::{ApiError, ApiMethod, ApiResult, Caller, Method};

const READ: &[&str] = &[
    "",
    "/branches",
    "/branches/{*branch}",
    "/tags",
    "/commits",
    "/commits/{*reference}",
    "/search/code",
    "/issues",
    "/issues/{number}",
    "/issues/{number}/comments",
    "/issues/comments/{id}",
    "/issues/{number}/timeline",
    "/issues/{number}/labels",
    "/assignees",
    "/labels",
    "/labels/{*name}",
    "/milestones",
    "/milestones/{id}",
    "/pulls",
    "/pulls/{number}",
    "/pulls/{number}/commits",
    "/pulls/{number}/files",
    "/pulls/{number}/merge",
    "/pulls/{number}/reviews",
    "/pulls/{number}/reviews/{id}",
    "/pulls/{number}/comments",
    "/pulls/{number}/requested_reviewers",
    "/pulls/{number}/review-progress",
    "/pulls/{number}/review-changes",
    "/pulls/{number}/readiness",
    "/pulls/{number}/watch",
    "/pulls/{number}/queue",
    "/pulls/{number}/manifest",
    "/pulls/{number}/inbox",
    "/agent/compare",
    "/agent/contains",
    "/agent/commits/{sha}/pulls",
    "/agent/issues",
    "/agent/preflight/push",
    "/deployments",
    "/actions/workflows",
    "/actions/workflows/{workflow_id}",
    "/actions/workflows/{workflow_id}/runs",
    "/actions/runs",
    "/actions/runs/{run_id}",
    "/actions/runs/{run_id}/jobs",
    "/actions/jobs/{job_id}",
    "/releases",
    "/releases/latest",
    "/releases/tags/{*tag}",
    "/releases/{release_id}",
    "/releases/{release_id}/assets",
    "/releases/assets/{asset_id}",
];

const LOGS: &[&str] = &[
    "/actions/runs/{run_id}/logs",
    "/actions/jobs/{job_id}/logs",
    "/actions/runs/{run_id}/failure",
    "/actions/jobs/{job_id}/steps/{step}/log",
    "/actions/jobs/{job_id}/annotations",
    "/actions/jobs/{job_id}/attempts",
    "/actions/runs/{run_id}/artifacts",
    "/actions/artifacts",
    "/actions/artifacts/{artifact_id}",
    "/actions/artifacts/{artifact_id}/{archive_format}",
    "/agent/artifacts",
    "/agent/status",
    "/agent/watch",
    "/agent/bisect",
];

#[derive(Debug, Clone, Copy)]
enum Need {
    Read,
    One(Right),
    Thread,
    PushPull,
}

fn need(method: ApiMethod, path: &str) -> Option<Need> {
    use ApiMethod::{Delete, Get, Patch, Post, Put};
    if method == Get && matches!(path, "/user" | "/version") {
        return Some(Need::Read);
    }
    let path = path.strip_prefix("/repos/{owner}/{repo}")?;
    if method == Get {
        return if READ.contains(&path) {
            Some(Need::Read)
        } else if LOGS.contains(&path) {
            Some(Need::One(Right::ReadLogs))
        } else {
            None
        };
    }
    Some(match (method, path) {
        (Post, "/issues" | "/labels" | "/milestones") | (Patch | Delete, "/labels/{*name}" | "/milestones/{id}") => {
            Need::One(Right::Issues)
        }
        (Patch | Delete, "/issues/{number}" | "/issues/comments/{id}")
        | (Post, "/issues/{number}/comments" | "/issues/{number}/labels" | "/issues/{number}/assignees" | "/issues/{number}/take")
        | (Put, "/issues/{number}/lock" | "/issues/{number}/labels")
        | (
            Delete,
            "/issues/{number}/lock" | "/issues/{number}/labels" | "/issues/{number}/labels/{*name}" | "/issues/{number}/assignees",
        ) => Need::Thread,
        (
            Post,
            "/pulls"
            | "/pulls/ensure"
            | "/pulls/{number}/ready"
            | "/pulls/{number}/reviews"
            | "/pulls/{number}/comments"
            | "/pulls/{number}/requested_reviewers",
        )
        | (Patch, "/pulls/{number}")
        | (Delete, "/pulls/{number}/requested_reviewers")
        | (Put, "/pulls/{number}/review-progress") => Need::One(Right::PullRequests),
        (Post, "/pulls/{number}/suggestions/apply" | "/pulls/{number}/update-branch" | "/agent/revert") => Need::PushPull,
        (Post, "/agent/branch-update") => Need::One(Right::Push),
        (Put, "/pulls/{number}/merge") | (Post | Delete, "/pulls/{number}/queue") => Need::One(Right::Merge),
        (Post, "/statuses/{sha}") => Need::One(Right::Statuses),
        (Post, "/actions/workflows/{workflow_id}/dispatches" | "/dispatches" | "/deployments/requests") => Need::One(Right::Dispatch),
        _ => return None,
    })
}

pub(super) fn check(method: ApiMethod, path: &str, limit: Option<&TokenLimit>) -> ApiResult<()> {
    let allowed = limit.is_some_and(|limit| match need(method, path) {
        Some(Need::Read) => true,
        Some(Need::One(right)) => limit.allows(right),
        Some(Need::Thread) => limit.allows(Right::Issues) || limit.allows(Right::PullRequests),
        Some(Need::PushPull) => limit.allows(Right::Push) && limit.allows(Right::PullRequests),
        None => false,
    });
    if allowed { Ok(()) } else { Err(ApiError::forbidden("This agent does not have access to this operation")) }
}

impl Caller {
    /// Issues and PRs share comment/metadata routes. Check the actual thread kind
    /// after visibility resolution, including edits addressed only by comment ID.
    pub fn require_agent_thread(&self, kind: IssueKind) -> ApiResult<()> {
        if self.user().is_some_and(|user| user.is_agent) && self.request.method != Method::GET {
            self.require_right(match kind {
                IssueKind::Issue => Right::Issues,
                IssueKind::Pull => Right::PullRequests,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_read_allowlist_names_existing_routes_and_refuses_sensitive_surfaces() {
        let mut config = rbit_core::Config::default();
        config.actions.enabled = true;
        let routes = super::super::super::table(&config);
        for suffix in READ.iter().chain(LOGS) {
            let path = format!("/repos/{{owner}}/{{repo}}{suffix}");
            assert!(routes.iter().any(|route| route.method == ApiMethod::Get && route.path == path), "missing route {path}");
        }
        let limit = TokenLimit { repo: None, rights: Right::ALL.to_vec() };
        for suffix in ["/hooks", "/actions/secrets", "/actions/variables", "/actions/runners", "/security/settings", "/future-endpoint"] {
            let path = format!("/repos/{{owner}}/{{repo}}{suffix}");
            assert!(check(ApiMethod::Get, &path, Some(&limit)).is_err(), "sensitive route {path}");
        }
        assert!(check(ApiMethod::Post, "/repos/{owner}/{repo}/actions/runs/{run_id}/approve", Some(&limit)).is_err());
        assert!(check(ApiMethod::Get, "/user/repos", Some(&limit)).is_err());
        assert!(check(ApiMethod::Post, "/user/agents", Some(&limit)).is_err());
        assert!(check(ApiMethod::Get, "/user", None).is_err());
    }

    #[test]
    fn agent_deployment_requests_require_dispatch_and_never_allow_configuration() {
        let prefix = "/repos/{owner}/{repo}";
        let read = TokenLimit { repo: None, rights: vec![] };
        let dispatch = TokenLimit { repo: None, rights: vec![Right::Dispatch] };
        let all = TokenLimit { repo: None, rights: Right::ALL.to_vec() };
        assert!(check(ApiMethod::Get, &format!("{prefix}/deployments"), Some(&read)).is_ok());
        assert!(check(ApiMethod::Post, &format!("{prefix}/deployments/requests"), Some(&read)).is_err());
        assert!(check(ApiMethod::Post, &format!("{prefix}/deployments/requests"), Some(&dispatch)).is_ok());
        assert!(check(ApiMethod::Put, &format!("{prefix}/deployments/workflows/{{environment}}"), Some(&all)).is_err());
        assert!(check(ApiMethod::Post, &format!("{prefix}/actions/runs/{{run_id}}/approve"), Some(&all)).is_err());
    }

    #[test]
    fn agent_capabilities_separate_reads_comments_pushes_merges_and_logs() {
        let read = TokenLimit { repo: None, rights: vec![] };
        assert!(check(ApiMethod::Get, "/repos/{owner}/{repo}/pulls", Some(&read)).is_ok());
        assert!(check(ApiMethod::Get, "/repos/{owner}/{repo}/actions/runs/{run_id}/logs", Some(&read)).is_err());
        assert!(check(ApiMethod::Post, "/repos/{owner}/{repo}/issues", Some(&read)).is_err());
        let writer = TokenLimit { repo: None, rights: vec![Right::PullRequests] };
        assert!(check(ApiMethod::Post, "/repos/{owner}/{repo}/pulls", Some(&writer)).is_ok());
        assert!(check(ApiMethod::Post, "/repos/{owner}/{repo}/pulls/{number}/suggestions/apply", Some(&writer)).is_err());
        assert!(check(ApiMethod::Put, "/repos/{owner}/{repo}/pulls/{number}/merge", Some(&writer)).is_err());
    }
}
