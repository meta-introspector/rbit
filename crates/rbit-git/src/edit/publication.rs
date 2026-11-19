//! Guarded edit publication: transfer first, then retain authority until the CAS child is reaped.
use super::{Committed, Duration, EditError, EditSpec, EditWorkspace, Git, Stdio, branch_ref, expect_success, validate_id};

impl EditWorkspace {
    /// The caller owns this future through completion (including HTTP cancellation).
    /// No timeout drops the acquired guard while a native ref-update child is alive.
    pub async fn commit_guarded<G>(
        &self,
        spec: &EditSpec<'_>,
        guard: impl std::future::Future<Output = Result<G, EditError>>,
    ) -> Result<Committed, EditError> {
        validate_id(spec.parent)?;
        let branch = spec.new_branch.unwrap_or(spec.branch);
        let target = branch_ref(branch)?;
        let expected = if spec.new_branch.is_some() { "0".repeat(spec.parent.len()) } else { spec.parent.to_owned() };
        let commit =
            tokio::time::timeout(self.timeout, self.prepare_and_transfer(spec)).await.map_err(|_| EditError::TimedOut(self.timeout))??;
        if crate::ref_alias::case_alias_at(&self.repo, &target).await?.is_some() {
            return Err(EditError::BranchMoved);
        }
        let limit = Duration::from_secs(5);
        let _guard = tokio::time::timeout(limit, guard).await.map_err(|_| EditError::TimedOut(limit))??;
        let base = Git { binary: &self.binary, dir: &self.repo };
        let mut command = base.command();
        command
            .args(["update-ref", "--no-deref", "-m", "rbit: guarded edit", "--end-of-options"])
            .arg(target)
            .arg(&commit)
            .arg(expected)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        let status = match tokio::time::timeout(limit, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(error)) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(error.into());
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return Err(EditError::TimedOut(limit));
            }
        };
        if !status.success() {
            return Err(EditError::BranchMoved);
        }
        Ok(Committed { commit, branch: branch.to_owned() })
    }

    async fn prepare_and_transfer(&self, spec: &EditSpec<'_>) -> Result<String, EditError> {
        let commit = self.prepare_commit(spec).await?;
        let git = self.git();
        let temporary = "refs/heads/rbit-review-result";
        git.run(["update-ref", "--no-deref", "--end-of-options", temporary, &commit]).await?;
        let base = Git { binary: &self.binary, dir: &self.repo };
        let mut fetch = base.command();
        fetch.args(["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--"]).arg(git.dir).arg(temporary);
        expect_success(fetch.output().await?, "fetch")?;
        Ok(commit)
    }
}
