//! API authentication, redirect and legacy token scope regressions.
use rbit_core::{Account, AccountId, AccountKind, Timestamp};

use super::*;

fn caller(scope: Option<TokenScope>) -> Caller {
    let account = Account {
        id: AccountId(1),
        kind: AccountKind::User,
        name: "ada".parse().unwrap(),
        display_name: None,
        bio: None,
        created_at: Timestamp(0),
    };
    let user = User { is_agent: false, agent_owner_id: None, account, email: "ada@example.com".parse().unwrap(), is_admin: false };
    Caller { user: Some(Arc::new(user)), scope, task: None, token_limit: None, token: None, request: Arc::default() }
}

#[test]
fn a_limited_token_reaches_its_repository_endpoints_only() {
    assert!(limited_route(ApiMethod::Get, "/repos/{owner}/{repo}"));
    assert!(limited_route(ApiMethod::Post, "/repos/{owner}/{repo}/pulls/{number}/queue"));
    assert!(limited_route(ApiMethod::Get, "/user"));
    assert!(!limited_route(ApiMethod::Get, "/user/repos"), "it would name every repository");
    assert!(!limited_route(ApiMethod::Post, "/user/repos"));
    assert!(!limited_route(ApiMethod::Patch, "/user"));
    assert!(!limited_route(ApiMethod::Get, "/search/repositories"));
    assert!(!limited_route(ApiMethod::Get, "/repos/{owner}/{repo}s"), "a prefix is not the repository");
}

#[test]
fn old_repository_names_are_replaced_in_place() {
    let target =
        |method: Method, path: &str, query: Option<&str>| RequestTarget { method, path: path.to_owned(), query: query.map(str::to_owned) };
    let base = "https://git.example.com";
    let commits = target(Method::GET, "/api/v1/repos/Alice/Before/commits/main", Some("per_page=1"));
    assert_eq!(
        commits.moved(base, "alice", "before", "acme/after").as_deref(),
        Some("https://git.example.com/api/v1/repos/acme/after/commits/main?per_page=1")
    );
    let star = target(Method::PUT, "/api/v1/user/starred/alice/before", None);
    assert_eq!(
        star.moved(base, "alice", "before", "alice/after").as_deref(),
        Some("https://git.example.com/api/v1/user/starred/alice/after")
    );
    assert_eq!(star.moved(base, "bob", "before", "bob/after"), None);
    // Only the repository's own segments change, not look-alikes further along.
    let branch = target(Method::GET, "/api/v1/repos/repos/x/branches/repos/x", None);
    assert_eq!(branch.moved(base, "repos", "x", "a/b").as_deref(), Some("https://git.example.com/api/v1/repos/a/b/branches/repos/x"));
}

#[test]
fn reads_bearer_and_token_schemes() {
    let token = |value: &'static str| presented_token(&HeaderValue::from_static(value)).map(str::to_owned);
    assert_eq!(token("Bearer rbit_abc").as_deref(), Some("rbit_abc"));
    assert_eq!(token("bearer   rbit_abc ").as_deref(), Some("rbit_abc"));
    assert_eq!(token("token rbit_abc").as_deref(), Some("rbit_abc"));
    assert_eq!(token("Basic YWxpY2U6cGFzcw=="), None);
    assert_eq!(token("Bearer"), None);
    assert_eq!(token("Bearer "), None);
}

#[test]
fn scopes_gate_actions_and_cap_access() {
    let read = caller(Some(TokenScope::Read));
    assert!(read.require(TokenScope::Read).is_ok());
    let refused = read.require(TokenScope::Write).unwrap_err();
    assert_eq!(refused.status, axum::http::StatusCode::FORBIDDEN);
    assert!(refused.message.contains("needs the write scope"));
    assert_eq!(read.limit(RepoAccess::Admin), RepoAccess::Read);

    let admin = caller(Some(TokenScope::Admin));
    assert!(admin.require(TokenScope::Admin).is_ok());
    assert_eq!(admin.limit(RepoAccess::Write), RepoAccess::Write, "never above the owner's access");

    let anonymous = Caller::default();
    assert_eq!(anonymous.require(TokenScope::Read).unwrap_err().status, axum::http::StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.limit(RepoAccess::Read), RepoAccess::Read);
}
