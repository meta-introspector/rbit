//! A refused fence never publishes; a timed-out child is reaped before authority drops.
use super::*;

#[tokio::test]
async fn security_guard_refusal_keeps_the_exact_existing_ref() {
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo.git");
    gix::init_bare(&repo).unwrap();
    let git = Git { binary: Path::new("git"), dir: &repo };
    let tree = git.command().args(["hash-object", "-w", "-t", "tree", "--stdin"]).stdin(Stdio::null()).output().await.unwrap();
    assert!(tree.status.success());
    let tree = String::from_utf8(tree.stdout).unwrap();
    let commit = git
        .run(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com", "commit-tree", tree.trim(), "-m", "base"])
        .await
        .unwrap();
    let commit = String::from_utf8(commit).unwrap().trim().to_owned();
    git.run(["update-ref", "refs/heads/main", &commit]).await.unwrap();
    let env = MergeEnvironment { git_binary: "git".into(), tmp_dir: root.path().join("tmp"), timeout: Duration::from_secs(10) };
    let proposed = git
        .run(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com", "commit-tree", tree.trim(), "-m", "proposed"])
        .await
        .unwrap();
    let proposed = String::from_utf8(proposed).unwrap().trim().to_owned();
    assert_ne!(proposed, commit);
    let result = land(&env, &repo, "main", &proposed, &commit, async { Err::<(), _>("scan changed") }).await;
    assert!(matches!(result, Err(Error::Guard("scan changed"))));
    assert_eq!(candidate::ref_target(&git, "refs/heads/main").await.unwrap().as_deref(), Some(commit.as_str()));
}

#[cfg(unix)]
#[tokio::test]
async fn security_publication_timeout_reaps_child_before_releasing_fence() {
    use std::{
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    struct Fence {
        path: PathBuf,
        reaped: Arc<AtomicBool>,
    }
    impl Drop for Fence {
        fn drop(&mut self) {
            let pid = std::fs::read_to_string(&self.path).unwrap();
            let alive = std::process::Command::new("kill")
                .args(["-0", pid.trim()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success();
            self.reaped.store(!alive, Ordering::SeqCst);
        }
    }
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo.git");
    gix::init_bare(&repo).unwrap();
    let program = root.path().join("git-fixture");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s' $$ > \"$0.pid\"\nexec sleep 60\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let reaped = Arc::new(AtomicBool::new(false));
    let fence = Fence { path: program.with_extension("pid"), reaped: reaped.clone() };
    let env = MergeEnvironment { git_binary: program, tmp_dir: root.path().join("tmp"), timeout: Duration::from_secs(10) };
    let result = land(&env, &repo, "main", &"1".repeat(40), &"2".repeat(40), async { Ok::<_, ()>(fence) }).await;
    assert!(matches!(result, Err(Error::Git(MergeError::TimedOut(_)))));
    assert!(reaped.load(Ordering::SeqCst), "authority must outlive the native update-ref child");
}
