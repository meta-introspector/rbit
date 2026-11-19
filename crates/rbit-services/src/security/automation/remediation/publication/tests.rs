//! Real-Git retry recovery must reconnect exact proposals and refuse human branch edits.
use super::*;
use crate::{CreateRepository, Services};
use rbit_core::{Config, Visibility};
use rbit_storage::{Database, KvStore, NewUser};
use std::{path::Path, process::Command, sync::Arc};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.name=Chase", "-c", "user.email=chase@example.com", "-c", "init.defaultBranch=main"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn proposal() -> Proposal {
    Proposal {
        package: super::super::super::Package { location: "node_modules/leaf".into(), name: "leaf".into(), version: "1.0.0".into() },
        version: "1.0.1".into(),
        manifest: "updated manifest\n".into(),
        lock: "updated lock\n".into(),
    }
}

fn push_count(events: &mut tokio::sync::broadcast::Receiver<crate::DomainEvent>) -> usize {
    let mut count = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, crate::DomainEvent::RepositoryPushed { .. }) {
            count += 1;
        }
    }
    count
}

struct Fixture {
    _dir: tempfile::TempDir,
    config: Config,
    database: Database,
    services: Services,
    actor: User,
    repo: RepoContext,
    work: std::path::PathBuf,
    base: String,
    generation: i64,
}
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.storage.data_dir = dir.path().join("data");
    config.server.public_url = "https://git.example.com".into();
    std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
    let database = Database::open(&config.storage.sqlite_path()).await.unwrap();
    let kv = KvStore::open(&config.storage.kv_path()).unwrap();
    let services = Services::new(Arc::new(config.clone()), &database, &kv).await.unwrap();
    let actor = database
        .accounts()
        .create_user(NewUser {
            name: "chase".parse().unwrap(),
            email: "chase@example.com".parse().unwrap(),
            display_name: None,
            password_hash: "test".into(),
            is_admin: false,
        })
        .await
        .unwrap();
    services
        .repositories
        .create(&actor, CreateRepository { name: "updates".into(), description: String::new(), visibility: Visibility::Public })
        .await
        .unwrap();
    let repo = services.repositories.open(Some(&actor), "chase", "updates").await.unwrap();
    let work = dir.path().join("work");
    std::fs::create_dir(&work).unwrap();
    git(&work, &["init", "-q"]);
    std::fs::write(work.join("package.json"), "original manifest\n").unwrap();
    std::fs::write(work.join("package-lock.json"), "original lock\n").unwrap();
    git(&work, &["add", "."]);
    git(&work, &["commit", "-qm", "Base"]);
    let base = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:main"]);
    let repo = services.repositories.open(Some(&actor), "chase", "updates").await.unwrap();
    services
        .security
        .configure_automation(
            &actor,
            &repo,
            super::super::super::AutomationSettings {
                dependencies: true,
                package_network_consent: true,
                minimum_severity: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let generation = database.security().automation(repo.repo.repo.id).await.unwrap().unwrap().generation;
    Fixture { _dir: dir, config, database, services, actor, repo, work, base, generation }
}

#[tokio::test]
async fn security_automation_recovers_created_pull_and_rejects_edited_branch() {
    let Fixture { _dir, config, database, services, actor, repo, work, base, generation } = fixture().await;
    let mut events = services.events.subscribe();
    let first = services.security.publish_dependency(&actor, &repo, &base, generation, proposal()).await.unwrap();
    assert_eq!(push_count(&mut events), 1);
    let connection =
        sqlx::SqlitePool::connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(config.storage.sqlite_path())).await.unwrap();
    sqlx::query("UPDATE security_remediations SET commit_sha = NULL, pull_number = NULL WHERE id = ?")
        .bind(first.id)
        .execute(&connection)
        .await
        .unwrap();
    let branch_recovered = services.security.publish_dependency(&actor, &repo, &base, generation, proposal()).await.unwrap();
    assert_eq!(branch_recovered.commit_sha, first.commit_sha);
    assert_eq!(branch_recovered.pull_number, first.pull_number);
    assert_eq!(push_count(&mut events), 1, "a persisted branch with no saved commit replays its push");
    // The persisted state immediately after successful PR creation but before recording its number.
    sqlx::query("UPDATE security_remediations SET pull_number = NULL WHERE id = ?").bind(first.id).execute(&connection).await.unwrap();
    let recovered = services.security.publish_dependency(&actor, &repo, &base, generation, proposal()).await.unwrap();
    assert_eq!(recovered.pull_number, first.pull_number);
    assert_eq!(push_count(&mut events), 0, "saved commits do not repeat their push during PR recovery");
    assert_eq!(
        database
            .pulls()
            .list(repo.repo.repo.id, &PullFilter { state: PullStateFilter::All, ..Default::default() }, 10, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    sqlx::query("UPDATE issues SET state = 'closed', state_reason = 'not_planned', closed_at = 1 WHERE repo_id = ? AND number = ?")
        .bind(repo.repo.repo.id.0)
        .bind(first.pull_number)
        .execute(&connection)
        .await
        .unwrap();
    sqlx::query("UPDATE security_remediations SET pull_number = NULL WHERE id = ?").bind(first.id).execute(&connection).await.unwrap();
    let closed = services.security.publish_dependency(&actor, &repo, &base, generation, proposal()).await.unwrap();
    assert_eq!(closed.pull_number, first.pull_number, "retry respects a closed proposal");
    sqlx::query("UPDATE pull_requests SET head_sha = ? WHERE issue_id = (SELECT id FROM issues WHERE repo_id = ? AND number = ?)")
        .bind(&base)
        .bind(repo.repo.repo.id.0)
        .bind(first.pull_number)
        .execute(&connection)
        .await
        .unwrap();
    sqlx::query("UPDATE security_remediations SET pull_number = NULL WHERE id = ?").bind(first.id).execute(&connection).await.unwrap();
    assert!(
        services.security.publish_dependency(&actor, &repo, &base, generation, proposal()).await.is_err(),
        "edited PR metadata cannot be reconnected"
    );
    let head = first.commit_sha.unwrap();
    services.security.exact_dependency_tree(&repo, &base, &head, &proposal()).await.unwrap();
    git(&work, &["fetch", "-q", repo.disk_path.to_str().unwrap(), &first.branch]);
    git(&work, &["checkout", "-q", "FETCH_HEAD"]);
    git(&work, &["update-index", "--chmod=+x", "package.json"]);
    git(&work, &["commit", "--amend", "-qm", "Change mode"]);
    let changed_mode = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:refs/heads/mode-test"]);
    assert!(services.security.exact_dependency_tree(&repo, &base, &changed_mode, &proposal()).await.is_err());
    // Same exact files and modes, but a second commit instead of one direct parent.
    git(&work, &["reset", "--hard", "HEAD"]);
    git(&work, &["checkout", "-q", &head]);
    git(&work, &["commit", "-q", "--allow-empty", "-m", "Extra ancestor"]);
    let changed_ancestry = git(&work, &["rev-parse", "HEAD"]);
    git(&work, &["push", "-q", repo.disk_path.to_str().unwrap(), "HEAD:refs/heads/ancestry-test"]);
    assert!(services.security.exact_dependency_tree(&repo, &base, &changed_ancestry, &proposal()).await.is_err());
    connection.close().await;
    services.shutdown().await;
}
