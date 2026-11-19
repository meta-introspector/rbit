//! Exact bounded review comparisons and tree-entry identities for personal progress.
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

use crate::{
    CommitDetail, DiffLimits, FileDiff, GitError, RepositoryReader,
    changes::diff_trees,
    reader::{resolve_commit, summarize},
};

impl RepositoryReader {
    /// Read a bounded regular file for suggestions; links and submodules are never editable here.
    pub async fn review_blob(&self, revision: String, path: String, limit: usize) -> Result<crate::BlobView, GitError> {
        let (commit, file) = (revision.clone(), path.clone());
        self.with_repo(move |repo| {
            let tree = repo.find_commit(resolve_commit(repo, &commit)?).map_err(GitError::backend)?.tree().map_err(GitError::backend)?;
            let entry = tree.lookup_entry_by_path(&file).map_err(GitError::backend)?.ok_or(GitError::NotFound("regular review file"))?;
            if !entry.mode().is_blob() {
                return Err(GitError::NotFound("regular review file"));
            }
            Ok(())
        })
        .await?;
        self.blob(revision, path, limit).await
    }

    /// Compare exact trees, including rewritten history; never substitute a merge base.
    pub async fn review_changes(&self, base: String, head: String, limits: DiffLimits) -> Result<CommitDetail, GitError> {
        self.with_repo(move |repo| {
            let old = repo.find_commit(resolve_commit(repo, &base)?).map_err(GitError::backend)?;
            let new = repo.find_commit(resolve_commit(repo, &head)?).map_err(GitError::backend)?;
            let old_tree = old.tree().map_err(GitError::backend)?;
            let new_tree = new.tree().map_err(GitError::backend)?;
            let (files, truncated) = diff_trees(repo, Some(&old_tree), &new_tree, limits)?;
            Ok(CommitDetail { commit: summarize(&new)?, committer_name: String::new(), files, truncated })
        })
        .await
    }

    /// Identities include both sides, modes and source paths, so renames and base changes invalidate progress.
    pub async fn review_fingerprints(&self, base: String, head: String, files: &[FileDiff]) -> Result<Vec<(String, String)>, GitError> {
        let paths: Vec<_> =
            files.iter().filter(|file| file.path.len() <= 4096).take(1000).map(|f| (f.path.clone(), f.old_path.clone())).collect();
        self.with_repo(move |repo| {
            let old = repo.find_commit(resolve_commit(repo, &base)?).map_err(GitError::backend)?.tree().map_err(GitError::backend)?;
            let new = repo.find_commit(resolve_commit(repo, &head)?).map_err(GitError::backend)?.tree().map_err(GitError::backend)?;
            paths
                .into_iter()
                .map(|(path, source)| {
                    let before = old.lookup_entry_by_path(source.as_deref().unwrap_or(&path)).map_err(GitError::backend)?;
                    let after = new.lookup_entry_by_path(&path).map_err(GitError::backend)?;
                    let entry = |e: Option<gix::object::tree::Entry<'_>>| {
                        e.map_or_else(|| "absent".into(), |e| format!("{}:{:?}", e.object_id(), e.mode()))
                    };
                    // Length-prefix the source bytes so unusual paths cannot make the identity ambiguous or unbounded.
                    let source = source.as_deref().unwrap_or(&path);
                    let mut digest = Sha256::new();
                    digest.update((source.len() as u64).to_le_bytes());
                    digest.update(source.as_bytes());
                    digest.update(entry(before));
                    digest.update(entry(after));
                    let fingerprint = digest.finalize().iter().fold(String::with_capacity(64), |mut text, byte| {
                        let _ = write!(text, "{byte:02x}");
                        text
                    });
                    Ok((path, fingerprint))
                })
                .collect()
        })
        .await
    }
}
