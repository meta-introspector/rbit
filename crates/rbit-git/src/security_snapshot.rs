//! Bounded immutable text inspection; visitors cannot follow links or execute repository files.
use crate::{GitError, RepositoryReader, reader::resolve_commit};
use gix::bstr::ByteSlice;
use std::time::{Duration, Instant};

impl RepositoryReader {
    /// Distinguishes an absent root file from a link/submodule or oversized tree.
    pub async fn security_root_file(&self, revision: String, name: String, limit: usize) -> Result<Option<crate::BlobView>, GitError> {
        let (rev, file) = (revision.clone(), name.clone());
        let exists = self
            .with_repo(move |repo| {
                let commit = repo.find_commit(resolve_commit(repo, &rev)?).map_err(GitError::backend)?;
                let id = commit.tree_id().map_err(GitError::backend)?;
                if repo.find_header(id.detach()).map_err(GitError::backend)?.size() > 2 * 1024 * 1024 {
                    return Err(GitError::Io(std::io::Error::other("root tree exceeds inspection budget")));
                }
                let tree = commit.tree().map_err(GitError::backend)?;
                let Some(entry) = tree.lookup_entry_by_path(&file).map_err(GitError::backend)? else {
                    return Ok(false);
                };
                if !entry.mode().is_blob() {
                    return Err(GitError::Io(std::io::Error::other("manifest must be a regular file")));
                }
                Ok(true)
            })
            .await?;
        if !exists {
            return Ok(None);
        }
        self.blob(revision, name, limit).await.map(Some)
    }
    /// Visits UTF-8 regular blobs only. Oversize trees/files, invalid paths and exhausted
    /// work budgets mark coverage incomplete. Binary files and links are outside text scope.
    pub async fn inspect_security_text<T: Send + 'static>(
        &self,
        revision: String,
        mut state: T,
        mut visit: impl FnMut(&str, &str, &mut T) -> bool + Send + 'static,
    ) -> Result<(T, bool), GitError> {
        self.with_repo(move |repo| {
            let started = Instant::now();
            let commit = repo.find_commit(resolve_commit(repo, &revision)?).map_err(GitError::backend)?;
            let mut pending = vec![(String::new(), commit.tree_id().map_err(GitError::backend)?.detach())];
            let (mut entries, mut bytes, mut complete) = (0, 0, true);
            'walk: while let Some((prefix, id)) = pending.pop() {
                let header = repo.find_header(id).map_err(GitError::backend)?;
                if header.size() > 1024 * 1024 || bytes + header.size() > 64 * 1024 * 1024 {
                    complete = false;
                    break;
                }
                bytes += header.size();
                let tree = repo.find_tree(id).map_err(GitError::backend)?;
                for entry in tree.iter() {
                    let entry = entry.map_err(GitError::backend)?;
                    entries += 1;
                    if entries > 10_000 || started.elapsed() > Duration::from_secs(5) {
                        complete = false;
                        break 'walk;
                    }
                    let Ok(name) = entry.filename().to_str() else {
                        complete = false;
                        continue;
                    };
                    let path = format!("{prefix}{name}");
                    if path.len() > 1024 || path.contains(['\\', ':', '?', '#']) || path.chars().any(char::is_control) {
                        complete = false;
                        continue;
                    }
                    if entry.mode().is_tree() {
                        pending.push((format!("{path}/"), entry.object_id()));
                        continue;
                    }
                    if !entry.mode().is_blob() {
                        continue;
                    }
                    let header = repo.find_header(entry.object_id()).map_err(GitError::backend)?;
                    if header.size() > 1024 * 1024 {
                        complete = false;
                        continue;
                    }
                    if bytes + header.size() > 64 * 1024 * 1024 {
                        complete = false;
                        break 'walk;
                    }
                    bytes += header.size();
                    let blob = repo.find_blob(entry.object_id()).map_err(GitError::backend)?;
                    let Ok(text) = std::str::from_utf8(&blob.data) else {
                        continue;
                    };
                    if !text.contains('\0') && !visit(&path, text, &mut state) {
                        complete = false;
                        break 'walk;
                    }
                }
            }
            Ok((state, complete))
        })
        .await
    }
}
