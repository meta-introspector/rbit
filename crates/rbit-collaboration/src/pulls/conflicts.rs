//! Bounded text conflict resolution: complete path sets and explicit marker removal.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_FILES: usize = 20;
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_TOTAL_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConflictFile {
    pub path: String,
    pub source: String,
    pub target: String,
    /// Git's combined text, including conflict markers, for the resolution editor.
    pub merged: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub head_sha: String,
    pub base_sha: String,
    pub files: Vec<ConflictFile>,
}

/// Reject before expensive preparation; the exact path set is checked against the preview.
pub fn validate_input(resolutions: &[Resolution]) -> Result<(), &'static str> {
    if resolutions.is_empty() || resolutions.len() > MAX_FILES {
        return Err("resolve between 1 and 20 conflicted files");
    }
    let mut paths = BTreeSet::new();
    let mut total = 0;
    for resolution in resolutions {
        if resolution.path.len() > 1024 || !paths.insert(&resolution.path) {
            return Err("conflict paths must be bounded and unique");
        }
        total += resolution.content.len();
        if resolution.content.len() > MAX_FILE_BYTES || total > MAX_TOTAL_BYTES {
            return Err("conflict resolutions exceed the text size limit");
        }
        if resolution.content.contains('\0') || resolution.content.replace("\r\n", "\n").replace('\r', "\n").lines().any(marker) {
            return Err("remove conflict markers and binary data before committing");
        }
    }
    Ok(())
}

pub fn validate_paths(preview: &Preview, resolutions: &[Resolution]) -> Result<(), &'static str> {
    validate_input(resolutions)?;
    let expected: BTreeSet<_> = preview.files.iter().map(|file| file.path.as_str()).collect();
    let actual: BTreeSet<_> = resolutions.iter().map(|file| file.path.as_str()).collect();
    if expected != actual {
        return Err("resolve every conflicted file without adding other paths");
    }
    Ok(())
}

fn marker(line: &str) -> bool {
    ["<<<<<<<", "|||||||", "=======", ">>>>>>>"].iter().any(|prefix| line.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolutions_require_a_complete_unique_bounded_marker_free_set() {
        let file = ConflictFile { path: "a.txt".into(), source: "a".into(), target: "b".into(), merged: String::new() };
        let preview = Preview { head_sha: "a".into(), base_sha: "b".into(), files: vec![file] };
        let mut input = vec![Resolution { path: "a.txt".into(), content: "resolved\n".into() }];
        assert!(validate_paths(&preview, &input).is_ok());
        input[0].path = "other".into();
        assert!(validate_paths(&preview, &input).is_err());
        input[0].path = "a.txt".into();
        for content in ["<<<<<<< HEAD", "=======", ">>>>>>> branch", "||||||| base", "a\0b", "safe\r<<<<<<< hidden"] {
            input[0].content = content.into();
            assert!(validate_paths(&preview, &input).is_err());
        }
        input[0].content = "x".repeat(MAX_FILE_BYTES + 1);
        assert!(validate_input(&input).is_err());
        input[0].content = String::new();
        assert!(validate_paths(&preview, &input).is_ok(), "an empty file is a valid resolution");
        input.push(input[0].clone());
        assert!(validate_input(&input).is_err());
    }
}
