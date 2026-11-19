//! Pause real preparation before the final fence; cancellation must retain owned work.
use super::*;
use std::{os::unix::fs::PermissionsExt, time::Duration};

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_str().unwrap().replace('\'', "'\\''"))
}

fn gated_git(root: &Path) -> std::path::PathBuf {
    let output = Command::new("which").arg("git").output().unwrap();
    assert!(output.status.success());
    let git = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    let binary = root.join("git-gate");
    let script = format!(
        "#!/bin/sh\n{} \"$@\"\nresult=$?\nfor arg do\nif [ \"$arg\" = refs/heads/rbit-conflict-result ]; then\n: > {}\nwhile [ ! -f {} ]; do sleep 0.01; done\nfi\ndone\nexit \"$result\"\n",
        quote(&git),
        quote(&root.join("ready")),
        quote(&root.join("release")),
    );
    std::fs::write(&binary, script).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    binary
}

#[tokio::test]
async fn browser_conflicts_cancelled_waiter_retains_work_and_rechecks_revocation_after_preparation() {
    for revoke in [false, true] {
        let gate = tempfile::tempdir().unwrap();
        let f = Fixture::with_git(Some(gated_git(gate.path()))).await;
        let n = conflicted(&f).await;
        let ctx = f.open_pull(&f.carol, n).await;
        let head = f.repo(&f.carol, "alice").await;
        let preview = f.services.pulls.preview_conflicts(&f.carol, &ctx, &head).await.unwrap();
        let input = ResolveConflicts {
            expected_head: preview.head_sha,
            expected_base: preview.base_sha,
            resolutions: vec![Resolution { path: "README.md".into(), content: "resolved\n".into() }],
        };
        let (service, actor, context, source) = (f.services.pulls.clone(), f.carol.clone(), ctx.clone(), head.clone());
        let waiter = tokio::spawn(async move { service.resolve_conflicts(&actor, &context, &source, input).await });
        tokio::time::timeout(Duration::from_secs(10), async {
            while !gate.path().join("ready").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        waiter.abort();
        let _ = waiter.await;
        if revoke {
            let owner = f.repo(&f.alice, "alice").await;
            f.services.collaborators.remove(&f.alice, &owner, "carol").await.unwrap();
        }
        std::fs::write(gate.path().join("release"), "go").unwrap();
        tokio::time::timeout(Duration::from_secs(10), f.services.pulls.shutdown()).await.unwrap();
        let actual = head.reader.commit_id("refs/heads/feature".into()).await.unwrap();
        assert_eq!(
            actual == ctx.pull.head_sha,
            revoke,
            "cancelled HTTP waiters must neither abandon admitted work nor skip current authority"
        );
        assert_eq!(f.open_pull(&f.alice, n).await.pull.head_sha, actual);
    }
}
