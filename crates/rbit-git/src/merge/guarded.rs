//! Publication with a caller-owned authority fence retained until the native CAS is reaped.
use super::{Git, MergeEnvironment, MergeError, MergeSpec, Merged, RESULT_REF, candidate, expect_success, prepare};
use crate::{
    pull_refs::{branch_ref, validate_id},
    ref_alias::case_alias_at,
};
use std::future::Future;
use std::{path::Path, process::Stdio, time::Duration};

#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    #[error(transparent)]
    Git(#[from] MergeError),
    #[error("publication authority changed")]
    Guard(E),
}

/// Compute and transfer before acquiring authority. The caller must own this future
/// through completion; dropping an HTTP waiter must not cancel native publication.
pub async fn merge<G, E>(
    env: &MergeEnvironment,
    spec: &MergeSpec<'_>,
    guard: impl Future<Output = Result<G, E>>,
) -> Result<Merged, Error<E>> {
    let commit =
        tokio::time::timeout(env.timeout, prepare_and_transfer(env, spec)).await.map_err(|_| MergeError::TimedOut(env.timeout))??;
    land(env, spec.repo, spec.base_branch, &commit, spec.base_tip, guard).await?;
    Ok(Merged { commit })
}

async fn prepare_and_transfer(env: &MergeEnvironment, spec: &MergeSpec<'_>) -> Result<String, MergeError> {
    let (scratch, commit) = prepare(env, spec).await?;
    let git = Git { binary: &env.git_binary, dir: scratch.path() };
    git.run(["update-ref", "--no-deref", "--end-of-options", RESULT_REF, &commit]).await?;
    let base = Git { binary: &env.git_binary, dir: spec.repo };
    let mut fetch = base.command();
    fetch.args(["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--"]).arg(git.dir).arg(RESULT_REF);
    expect_success(fetch.output().await?, "fetch")?;
    Ok(commit)
}

/// Land an already computed candidate while retaining the authority fence through
/// successful completion or timeout kill-and-reap. No timeout wraps the owned guard.
pub async fn land<G, E>(
    env: &MergeEnvironment,
    repo: &Path,
    branch: &str,
    commit: &str,
    expected: &str,
    guard: impl Future<Output = Result<G, E>>,
) -> Result<(), Error<E>> {
    validate_id(commit).map_err(MergeError::from)?;
    validate_id(expected).map_err(MergeError::from)?;
    let target = branch_ref(branch).map_err(MergeError::from)?;
    if case_alias_at(repo, &target).await.map_err(MergeError::from)?.is_some() {
        return Err(MergeError::BaseMoved.into());
    }
    let limit = Duration::from_secs(5);
    let _fence = tokio::time::timeout(limit, guard).await.map_err(|_| MergeError::TimedOut(limit))?.map_err(Error::Guard)?;
    let base = Git { binary: &env.git_binary, dir: repo };
    let mut command = base.command();
    command
        .args(["update-ref", "--no-deref", "-m", "rbit: merge pull request", "--end-of-options"])
        .arg(&target)
        .arg(commit)
        .arg(expected)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(MergeError::from)?;
    let status = match tokio::time::timeout(limit, child.wait()).await {
        Ok(Ok(status)) => status,
        result => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(match result {
                Ok(Err(error)) => MergeError::from(error),
                _ => MergeError::TimedOut(limit),
            }
            .into());
        }
    };
    if status.success() {
        return Ok(());
    }
    let current = tokio::time::timeout(limit, candidate::ref_target(&base, &target)).await.map_err(|_| MergeError::TimedOut(limit))??;
    if current.as_deref() == Some(expected) {
        Err(MergeError::Failed(format!("update-ref ({status})")).into())
    } else {
        Err(MergeError::BaseMoved.into())
    }
}

#[cfg(test)]
mod tests;
