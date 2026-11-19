//! Resolve bounded text conflicts on a retained merged tree, without checking out files.
mod parsing;
mod publication;

const LIMIT: usize = 128 * 1024;

use super::{EditError, EditWorkspace, Git, IndexLine, Mode};
use crate::{GitError, RepositoryLease, merge::MergeEnvironment, overlay::Overlay, process::bounded_output, pull_refs::validate_id};
use rbit_core::{
    file_edit::FilePath,
    pull::conflicts::{ConflictFile, MAX_FILE_BYTES, MAX_FILES, MAX_TOTAL_BYTES, Preview},
};
use std::{collections::BTreeMap, path::Path};

/// A preview owns all scratch objects until publication finishes or the workspace drops.
#[derive(Debug)]
pub struct ConflictWorkspace {
    edit: EditWorkspace,
    tree: String,
    modes: BTreeMap<String, Mode>,
    pub preview: Preview,
}

impl ConflictWorkspace {
    pub async fn open(env: &MergeEnvironment, head_repo: &Path, base_repo: &Path, head: &str, base: &str) -> Result<Self, EditError> {
        validate_id(head)?;
        validate_id(base)?;
        let scratch = Overlay::create_in(&env.tmp_dir.join(crate::overlay::MERGES_DIR), &[head_repo, base_repo]).await?;
        let edit = EditWorkspace { scratch, binary: env.git_binary.clone(), repo: head_repo.to_owned(), timeout: env.timeout };
        tokio::time::timeout(env.timeout, Self::prepare(edit, head, base)).await.map_err(|_| EditError::TimedOut(env.timeout))?
    }

    #[must_use]
    pub fn holding(mut self, lease: RepositoryLease) -> Self {
        self.edit = self.edit.holding(lease);
        self
    }

    async fn prepare(edit: EditWorkspace, head: &str, base: &str) -> Result<Self, EditError> {
        let mut command = edit.git().command();
        command.args(["merge-tree", "--write-tree", "-z", "--no-messages", "--end-of-options", head, base]);
        // Retain one extra byte: overflow is rejected, never mistaken for a complete path set.
        let output = bounded_output(&mut command, LIMIT + 1, 4096).await?;
        if output.status.success() {
            return Err(EditError::Rejected("these branches have no conflicts to resolve".into()));
        }
        if output.status.code() != Some(1) {
            return Err(EditError::Rejected("these branches cannot be resolved in the browser".into()));
        }
        if output.stdout.len() > LIMIT {
            return Err(unsupported());
        }
        let (tree, stages) = parsing::parse(&output.stdout)?;
        if stages.is_empty() || stages.len() > MAX_FILES {
            return Err(unsupported());
        }
        let (files, modes) = parsing::read_files(&edit, &tree, head, base, stages).await?;
        let preview = Preview { head_sha: head.into(), base_sha: base.into(), files };
        Ok(Self { edit, tree, modes, preview })
    }
}

fn unsupported() -> EditError {
    EditError::Rejected(
        "resolve these conflicts locally: only bounded regular text files with matching paths and modes are supported".into(),
    )
}

#[cfg(test)]
mod tests;
