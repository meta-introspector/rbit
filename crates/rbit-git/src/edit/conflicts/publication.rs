//! Build the resolved merge commit before retaining authority through source-branch CAS.
use super::{ConflictWorkspace, EditError, Git, IndexLine, MAX_FILE_BYTES, MAX_TOTAL_BYTES, MergeEnvironment, unsupported};
use crate::merge::{Signature, expect_success, guarded};
use rbit_core::{
    Timestamp,
    file_edit::{LineEnding, prepare_text},
    pull::{
        CommitIdentity,
        conflicts::{Resolution, validate_paths},
    },
};

impl ConflictWorkspace {
    /// Validate normalized sizes before any work that could be mistaken for a Git failure.
    pub fn validate_resolutions(&self, resolutions: &[Resolution]) -> Result<(), EditError> {
        validate_paths(&self.preview, resolutions).map_err(|message| EditError::Rejected(message.into()))?;
        let mut total = 0;
        for resolution in resolutions {
            let file = self.preview.files.iter().find(|file| file.path == resolution.path).ok_or_else(unsupported)?;
            let content = prepare_text(&resolution.content, LineEnding::detect(file.source.as_bytes()));
            total += content.len();
            if content.len() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
                return Err(unsupported());
            }
        }
        Ok(())
    }
    /// The service owns this future through cancellation. No input path escapes the preview.
    pub async fn commit<G, E>(
        &self,
        branch: &str,
        resolutions: &[Resolution],
        author: &CommitIdentity,
        at: Timestamp,
        guard: impl std::future::Future<Output = Result<G, E>>,
    ) -> Result<String, guarded::Error<E>> {
        let commit = tokio::time::timeout(self.edit.timeout, self.prepare_commit(resolutions, author, at))
            .await
            .map_err(|_| crate::merge::MergeError::TimedOut(self.edit.timeout))?
            .map_err(|error| crate::merge::MergeError::Failed(error.to_string()))?;
        let env = MergeEnvironment {
            git_binary: self.edit.binary.clone(),
            tmp_dir: self.edit.scratch.path().to_owned(),
            timeout: self.edit.timeout,
        };
        guarded::land(&env, &self.edit.repo, branch, &commit, &self.preview.head_sha, guard).await?;
        Ok(commit)
    }

    async fn prepare_commit(&self, resolutions: &[Resolution], author: &CommitIdentity, at: Timestamp) -> Result<String, EditError> {
        self.validate_resolutions(resolutions)?;
        let mut lines = Vec::new();
        let mut total = 0;
        for resolution in resolutions {
            let file = self.preview.files.iter().find(|file| file.path == resolution.path).ok_or_else(unsupported)?;
            let mode = *self.modes.get(&resolution.path).ok_or_else(unsupported)?;
            let content = prepare_text(&resolution.content, LineEnding::detect(file.source.as_bytes()));
            total += content.len();
            if total > MAX_TOTAL_BYTES {
                return Err(unsupported());
            }
            let blob = self.edit.write_blob(content.as_slice(), MAX_FILE_BYTES as u64).await?;
            lines.push(IndexLine::Write { mode, id: blob.as_str().into(), path: resolution.path.clone() });
        }
        // Start from Git's merged tree, so non-conflicting changes from both sides survive.
        let tree = self.edit.write_tree(&self.tree, &lines).await?;
        let signature = Signature::new(author, at);
        let git = self.edit.git();
        let commit = git
            .commit_tree(
                &tree,
                &[&self.preview.head_sha, &self.preview.base_sha],
                b"Resolve pull request conflicts\n",
                &signature,
                &signature,
            )
            .await?;
        let temporary = "refs/heads/rbit-conflict-result";
        git.run(["update-ref", "--no-deref", "--end-of-options", temporary, &commit]).await?;
        let target = Git { binary: &self.edit.binary, dir: &self.edit.repo };
        let mut fetch = target.command();
        fetch.args(["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", "--"]).arg(git.dir).arg(temporary);
        expect_success(fetch.output().await?, "fetch")?;
        Ok(commit)
    }
}
