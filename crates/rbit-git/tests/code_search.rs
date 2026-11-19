//! Real Git content search: literal matching, filtering, limits and immutable results.
use rbit_git::{CodeQuery, RepositoryReader};
use std::{path::Path, process::Command};
fn git(root: &Path, args: &[&str]) {
    assert!(
        Command::new("git")
            .args(["-c", "user.name=Test", "-c", "user.email=test@example.com", "-c", "init.defaultBranch=main"])
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
            .status
            .success()
    );
}
#[tokio::test]
async fn code_search_filters_and_pins_results() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    git(root, &["init", "-q"]);
    std::fs::create_dir(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "first\nneedle.* <script>\nNEEDLE\n").unwrap();
    std::fs::write(root.join("other.txt"), "needle.*\n").unwrap();
    std::fs::write(root.join("binary.rs"), b"needle.*\0").unwrap();
    std::fs::write(root.join("large.rs"), "needle.*".repeat(150_000)).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("needle.*", root.join("link.rs")).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "initial"]);
    let reader = RepositoryReader::new(root.join(".git"));
    let query = || CodeQuery::new("needle.*".into(), "src/".into(), "Rust".into()).unwrap();
    let first = reader.search_code("main".into(), query()).await.unwrap().unwrap();
    assert_eq!(first.matches.len(), 1);
    assert_eq!(first.matches[0].line, 2);
    assert_eq!(first.matches[0].path, "src/lib.rs");
    assert!(!first.truncated);
    std::fs::write(root.join("src/lib.rs"), "changed").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "changed"]);
    assert!(reader.search_code("main".into(), query()).await.unwrap().unwrap().matches.is_empty());
    assert_eq!(reader.search_code(first.commit, query()).await.unwrap().unwrap().matches.len(), 1);
    let all =
        reader.search_code("main".into(), CodeQuery::new("needle.*".into(), String::new(), String::new()).unwrap()).await.unwrap().unwrap();
    assert_eq!(all.matches.len(), 1);
    assert_eq!(all.skipped_files, 2);
    assert!(!all.truncated);
    std::fs::write(root.join("many.rs"), "needle.*\n".repeat(101)).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "many"]);
    let capped =
        reader.search_code("main".into(), CodeQuery::new("needle.*".into(), "many".into(), String::new()).unwrap()).await.unwrap().unwrap();
    assert_eq!(capped.matches.len(), 100);
    assert!(capped.truncated);
}
