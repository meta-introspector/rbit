//! Real Git conflict trees, guarded source updates and unsupported-file refusals.
use super::*;
use rbit_core::{
    Timestamp,
    pull::{CommitIdentity, conflicts::Resolution},
};
use std::{path::PathBuf, process::Command, time::Duration};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=Reviewer", "-c", "user.email=reviewer@example.com", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Fixture {
    _root: tempfile::TempDir,
    repo: PathBuf,
    env: MergeEnvironment,
    head: String,
    base: String,
}
impl Fixture {
    fn new(ours: &[u8], theirs: &[u8]) -> Self {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("file.txt"), "original\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "Original"]);
        git(&repo, &["checkout", "-qb", "feature"]);
        std::fs::write(repo.join("file.txt"), ours).unwrap();
        std::fs::write(repo.join("source-only.txt"), "source\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "Source"]);
        let head = git(&repo, &["rev-parse", "HEAD"]);
        git(&repo, &["checkout", "-q", "main"]);
        std::fs::write(repo.join("file.txt"), theirs).unwrap();
        std::fs::write(repo.join("base-only.txt"), "base\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "Target"]);
        let base = git(&repo, &["rev-parse", "HEAD"]);
        let env = MergeEnvironment { git_binary: "git".into(), tmp_dir: root.path().join("tmp"), timeout: Duration::from_secs(20) };
        Self { _root: root, repo, env, head, base }
    }
    async fn open(&self) -> Result<ConflictWorkspace, EditError> {
        // These Git operations need a bare object source; .git also has the expected layout.
        let repo = self.repo.join(".git");
        ConflictWorkspace::open(&self.env, &repo, &repo, &self.head, &self.base).await
    }
}

#[tokio::test]
async fn conflict_resolution_preserves_clean_merge_changes_and_two_parents() {
    let f = Fixture::new(b"source\r\n", b"target\r\n");
    let work = f.open().await.unwrap();
    assert_eq!(work.preview.files.len(), 1);
    assert!(work.preview.files[0].merged.contains("<<<<<<<"));
    let input = [Resolution { path: "file.txt".into(), content: "resolved\n".into() }];
    let author = CommitIdentity { name: "Reviewer".into(), email: "reviewer@example.com".into() };
    let id = work.commit("feature", &input, &author, Timestamp::now(), async { Ok::<_, ()>(()) }).await.unwrap();
    assert_eq!(git(&f.repo, &["rev-parse", "feature"]), id);
    assert_eq!(git(&f.repo, &["rev-parse", &format!("{id}^1")]), f.head);
    assert_eq!(git(&f.repo, &["rev-parse", &format!("{id}^2")]), f.base);
    for (path, text) in [("source-only.txt", "source"), ("base-only.txt", "base")] {
        assert_eq!(git(&f.repo, &["show", &format!("{id}:{path}")]), text);
    }
    let output = Command::new("git").current_dir(&f.repo).args(["show", &format!("{id}:file.txt")]).output().unwrap();
    assert_eq!(output.stdout, b"resolved\r\n");
    assert_eq!(git(&f.repo, &["rev-parse", "main"]), f.base, "only the source branch changes");
}

#[tokio::test]
async fn conflict_resolution_refuses_guard_stale_head_and_extra_paths() {
    let f = Fixture::new(b"source\n", b"target\n");
    let work = f.open().await.unwrap();
    let author = CommitIdentity { name: "Reviewer".into(), email: "reviewer@example.com".into() };
    let mut input = [Resolution { path: "other.txt".into(), content: "resolved\n".into() }];
    assert!(work.commit("feature", &input, &author, Timestamp::now(), async { Ok::<_, ()>(()) }).await.is_err());
    input[0].path = "file.txt".into();
    assert!(matches!(
        work.commit("feature", &input, &author, Timestamp::now(), async { Err::<(), _>("revoked") }).await,
        Err(crate::merge::guarded::Error::Guard("revoked"))
    ));
    assert_eq!(git(&f.repo, &["rev-parse", "feature"]), f.head);
    git(&f.repo, &["update-ref", "refs/heads/feature", &f.base, &f.head]);
    assert!(work.commit("feature", &input, &author, Timestamp::now(), async { Ok::<_, ()>(()) }).await.is_err());
    assert_eq!(git(&f.repo, &["rev-parse", "feature"]), f.base);
}

#[tokio::test]
async fn conflict_resolution_refuses_binary_oversized_and_modify_delete() {
    for (ours, theirs) in [(b"source\0\n".to_vec(), b"target\0\n".to_vec()), (vec![b'a'; MAX_FILE_BYTES + 1], b"target\n".to_vec())] {
        let f = Fixture::new(&ours, &theirs);
        assert!(f.open().await.is_err());
    }
    let mut f = Fixture::new(b"source\n", b"target\n");
    git(&f.repo, &["rm", "file.txt"]);
    git(&f.repo, &["commit", "-qm", "Delete"]);
    f.base = git(&f.repo, &["rev-parse", "HEAD"]);
    assert!(f.open().await.is_err());
}

#[tokio::test]
async fn conflict_resolution_add_add_keeps_executable_mode() {
    let mut f = Fixture::new(b"source\n", b"target\n");
    let ancestor = git(&f.repo, &["rev-parse", &format!("{}^", f.head)]);
    for (branch, content) in [("our-add", "ours\n"), ("their-add", "theirs\n")] {
        git(&f.repo, &["checkout", "-qb", branch, &ancestor]);
        std::fs::write(f.repo.join("added.sh"), content).unwrap();
        git(&f.repo, &["add", "added.sh"]);
        git(&f.repo, &["update-index", "--chmod=+x", "added.sh"]);
        git(&f.repo, &["commit", "-qm", "Add executable"]);
        git(&f.repo, &["reset", "--hard", "HEAD"]);
    }
    f.head = git(&f.repo, &["rev-parse", "our-add"]);
    f.base = git(&f.repo, &["rev-parse", "their-add"]);
    let workspace = f.open().await.unwrap();
    assert_eq!(workspace.preview.files[0].path, "added.sh");
    let author = CommitIdentity { name: "Reviewer".into(), email: "reviewer@example.com".into() };
    let input = [Resolution { path: "added.sh".into(), content: "#!/bin/sh\nexit 0\n".into() }];
    let id = workspace.commit("our-add", &input, &author, Timestamp::now(), async { Ok::<_, ()>(()) }).await.unwrap();
    assert!(git(&f.repo, &["ls-tree", &id, "added.sh"]).starts_with("100755 "));
}

#[test]
fn conflict_stage_records_reject_unsupported_modes_paths_and_ambiguous_stages() {
    let oid = "1".repeat(40);
    for record in [
        format!("120000 {oid} 2\tlink"),
        format!("160000 {oid} 2\tmodule"),
        format!("100644 {oid} 2\t../escape"),
        format!("100644 {oid} 2\t.git/config"),
        format!("100644 {oid} 2\tfile\0"),
    ] {
        let input = format!("{oid}\0{record}\0{record}\0");
        assert!(parsing::parse(input.as_bytes()).is_err());
    }
}
