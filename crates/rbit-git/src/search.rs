//! Bounded literal content search over one immutable Git commit.

use crate::{GitError, RepositoryReader, reader::resolve_commit};
use gix::bstr::ByteSlice;
use std::{
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;

static CAPACITY: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
const MAX_ENTRIES: usize = 20_000;
const MAX_FILE: u64 = 1024 * 1024;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MATCHES: usize = 100;

/// Validated literal query and optional path prefix and detected language name.
#[derive(Debug, Clone)]
pub struct CodeQuery {
    text: String,
    path: String,
    language: String,
}
impl CodeQuery {
    pub fn new(text: String, path: String, language: String) -> Result<Self, &'static str> {
        if text.is_empty() || text.len() > 256 || text.contains(['\n', '\r', '\0']) {
            return Err("Search text must be 1–256 bytes on one line.");
        }
        if path.len() > 1024 || path.contains(['\0', '\n', '\r']) || language.len() > 64 || language.contains(['\0', '\n', '\r']) {
            return Err("Search filter is too long or contains invalid characters.");
        }
        Ok(Self { text, path, language })
    }
}

#[derive(Debug, Clone)]
pub struct CodeMatch {
    pub path: String,
    pub line: usize,
    pub snippet: String,
}
#[derive(Debug, Default)]
pub struct CodeResults {
    pub commit: String,
    pub matches: Vec<CodeMatch>,
    /// Work or output bounds were reached. Absence of a match is then inconclusive.
    pub truncated: bool,
    pub skipped_files: usize,
}

impl RepositoryReader {
    /// Search the named branch at one resolved commit. Admission remains owned by the
    /// blocking worker even when the HTTP request is cancelled.
    pub async fn search_code(&self, revision: String, query: CodeQuery) -> Result<Option<CodeResults>, GitError> {
        let Ok(permit) = CAPACITY.clone().try_acquire_owned() else { return Ok(None) };
        self.with_repo(move |repo| {
            let _permit = permit;
            let started = Instant::now();
            let commit = repo.find_commit(resolve_commit(repo, &revision)?).map_err(GitError::backend)?;
            let mut result = CodeResults { commit: commit.id.to_string(), ..CodeResults::default() };
            let mut pending = vec![(String::new(), commit.tree_id().map_err(GitError::backend)?.detach())];
            let mut entries = 0;
            let mut bytes = 0;
            'walk: while let Some((prefix, id)) = pending.pop() {
                let header = repo.find_header(id).map_err(GitError::backend)?;
                if header.size() > MAX_FILE || bytes + header.size() > MAX_BYTES {
                    result.truncated = true;
                    break;
                }
                bytes += header.size();
                let tree = repo.find_tree(id).map_err(GitError::backend)?;
                for entry in tree.iter() {
                    let entry = entry.map_err(GitError::backend)?;
                    entries += 1;
                    if entries > MAX_ENTRIES || started.elapsed() > Duration::from_secs(2) {
                        result.truncated = true;
                        break 'walk;
                    }
                    let Ok(name) = entry.filename().to_str() else {
                        result.skipped_files += 1;
                        continue;
                    };
                    let path = format!("{prefix}{name}");
                    if path.len() > 4096 {
                        result.truncated = true;
                        continue;
                    }
                    if entry.mode().is_tree() {
                        pending.push((format!("{path}/"), entry.object_id()));
                        continue;
                    }
                    if !entry.mode().is_blob() || !path.starts_with(&query.path) {
                        continue;
                    }
                    if !query.language.is_empty()
                        && !crate::languages::detect(&path).is_some_and(|l| l.name.eq_ignore_ascii_case(&query.language))
                    {
                        continue;
                    }
                    let header = repo.find_header(entry.object_id()).map_err(GitError::backend)?;
                    if header.size() > MAX_FILE {
                        result.skipped_files += 1;
                        continue;
                    }
                    if bytes + header.size() > MAX_BYTES {
                        result.truncated = true;
                        break 'walk;
                    }
                    bytes += header.size();
                    let blob = repo.find_blob(entry.object_id()).map_err(GitError::backend)?;
                    let Ok(text) = std::str::from_utf8(&blob.data) else {
                        result.skipped_files += 1;
                        continue;
                    };
                    if text.contains('\0') {
                        result.skipped_files += 1;
                        continue;
                    }
                    for (line, content) in text.lines().enumerate() {
                        if let Some(offset) = content.find(&query.text) {
                            if result.matches.len() == MAX_MATCHES {
                                result.truncated = true;
                                break 'walk;
                            }
                            let mut start = offset.saturating_sub(80);
                            while !content.is_char_boundary(start) {
                                start += 1;
                            }
                            let snippet: String = content[start..].chars().take(400).collect();
                            result.matches.push(CodeMatch { path: path.clone(), line: line + 1, snippet });
                        }
                    }
                }
            }
            result.matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
            Ok(Some(result))
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::{CAPACITY, CodeQuery};
    #[tokio::test]
    async fn code_search_refuses_before_opening_repository_when_capacity_is_full() {
        let permits = CAPACITY.clone().acquire_many_owned(2).await.unwrap();
        let reader = crate::RepositoryReader::new(std::path::PathBuf::from("does-not-exist"));
        let query = CodeQuery::new("needle".into(), String::new(), String::new()).unwrap();
        assert!(reader.search_code("main".into(), query).await.unwrap().is_none());
        drop(permits);
    }

    #[test]
    fn validates_bounded_literal_queries() {
        assert!(CodeQuery::new("literal.*".into(), "src/".into(), "Rust".into()).is_ok());
        for text in [String::new(), "x".repeat(257), "a\nb".into(), "a\0b".into()] {
            assert!(CodeQuery::new(text, String::new(), String::new()).is_err());
        }
    }
}
