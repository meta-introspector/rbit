//! Resolve a visible thread and enforce delegated capabilities for shared issue/PR routes.

use super::{ApiError, ApiResult, AppState, Caller, IssueContext, IssueNumber};

/// Thread `number` of the repository, for the caller; 404 when hidden or unknown.
pub(super) async fn open_thread(state: &AppState, caller: &Caller, owner: &str, repo: &str, number: &str) -> ApiResult<IssueContext> {
    let ctx = caller.open_repo(state, owner, repo).await?;
    let number = IssueNumber::parse(number).map_err(|_| ApiError::not_found())?;
    let thread = state.services.timeline.open(caller.user(), ctx, number).await?;
    caller.require_agent_thread(thread.issue.kind)?;
    Ok(thread)
}
