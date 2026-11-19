//! Allocate cloud identities only for new Actions runs, never during disabled operation.
use rbit_storage::Database;
use sqlx::{Row, SqlitePool};

async fn identity_counts(raw: &SqlitePool) -> (i64, i64) {
    sqlx::query_as("SELECT (SELECT count(*) FROM actions_oidc_accounts), (SELECT count(*) FROM actions_oidc_repositories)")
        .fetch_one(raw)
        .await
        .unwrap()
}

async fn new_subject(raw: &SqlitePool) {
    sqlx::query("INSERT INTO accounts(id, kind, name, created_at, updated_at) VALUES(100, 'user', 'cloud-owner', 3, 3)")
        .execute(raw)
        .await
        .unwrap();
    sqlx::query("INSERT INTO repositories(id, owner_id, name, visibility, default_branch, created_at, updated_at) VALUES(100, 100, 'cloud', 'public', 'main', 3, 3)")
        .execute(raw).await.unwrap();
}

async fn new_run(raw: &SqlitePool, repo: i64, number: i64) -> (String, String) {
    sqlx::query("INSERT INTO actions_runs (repo_id, run_number, workflow_path, workflow_name, title, event, ref, sha, status, created_at, trusted, event_payload) VALUES(?, ?, 'deploy.yml', 'Deploy', 'New run', 'push', 'refs/heads/trunk', '0123456789abcdef0123456789abcdef01234567', 1, 2, 1, '{}')")
        .bind(repo).bind(number).execute(raw).await.unwrap();
    sqlx::query_as("SELECT oidc_owner_uid, oidc_repository_uid FROM actions_runs WHERE repo_id = ? AND run_number = ?")
        .bind(repo)
        .bind(number)
        .fetch_one(raw)
        .await
        .unwrap()
}

#[tokio::test]
async fn oidc_upgrade_allocates_identities_only_for_new_runs_and_invalidates_transfers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oidc.db");
    let raw = super::writer_connection::pool(&path).await;
    sqlx::migrate!("./migrations").run_to(95, &raw).await.unwrap();
    sqlx::raw_sql(include_str!("fixtures/milestone1.sql")).execute(&raw).await.unwrap();
    sqlx::query("INSERT INTO actions_runs (repo_id, run_number, workflow_path, workflow_name, title, event, ref, sha, actor_id, status, created_at, trusted, event_payload) VALUES(6, 1, 'deploy.yml', 'Deploy', 'Old run', 'push', 'refs/heads/trunk', '0123456789abcdef0123456789abcdef01234567', 1, 1, 1, 1, '{}')")
        .execute(&raw).await.unwrap();
    let _database = Database::open(&path).await.unwrap();
    assert_eq!(identity_counts(&raw).await, (0, 0), "upgrading must not allocate cloud identities");
    new_subject(&raw).await;
    assert_eq!(identity_counts(&raw).await, (0, 0), "ordinary account and repository creation leaves Actions untouched");
    let captured = new_run(&raw, 6, 2).await;
    let old =
        sqlx::query("SELECT oidc_owner_uid, oidc_repository_uid FROM actions_runs WHERE run_number = 1").fetch_one(&raw).await.unwrap();
    assert!(old.get::<Option<String>, _>("oidc_owner_uid").is_none());
    assert!(old.get::<Option<String>, _>("oidc_repository_uid").is_none());
    assert_eq!((captured.0.len(), captured.1.len()), (32, 32));
    assert_eq!(identity_counts(&raw).await, (1, 1), "only the run's owner and repository receive identities");
    assert_eq!(new_run(&raw, 6, 3).await, captured, "another run preserves the current ownership tenure");
    sqlx::query("UPDATE repositories SET owner_id = 1 WHERE id = 6").execute(&raw).await.unwrap();
    let moved = new_run(&raw, 6, 4).await;
    assert_ne!(moved.0, captured.0);
    assert_ne!(moved.1, captured.1);
    sqlx::query("UPDATE repositories SET owner_id = 10 WHERE id = 6").execute(&raw).await.unwrap();
    let returned = new_run(&raw, 6, 5).await;
    assert_eq!(returned.0, captured.0, "the original owner's identity is stable");
    assert_ne!(returned.1, captured.1, "returning ownership cannot resurrect previous cloud trust");
    assert_ne!(returned.1, moved.1);
    let retained: (String, String) =
        sqlx::query_as("SELECT oidc_owner_uid, oidc_repository_uid FROM actions_runs WHERE repo_id = 6 AND run_number = 2")
            .fetch_one(&raw)
            .await
            .unwrap();
    assert_eq!(retained, captured, "old runs retain their original tenure");
    assert!(sqlx::query("PRAGMA foreign_key_check").fetch_all(&raw).await.unwrap().is_empty());
}

#[tokio::test]
async fn oidc_reused_account_and_repository_ids_cannot_reclaim_cloud_identities() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("oidc.db");
    let _database = Database::open(&path).await.unwrap();
    let raw = super::writer_connection::pool(&path).await;
    new_subject(&raw).await;
    assert_eq!(identity_counts(&raw).await, (0, 0));
    let original = new_run(&raw, 100, 1).await;
    sqlx::query("DELETE FROM repositories WHERE id = 100").execute(&raw).await.unwrap();
    assert_eq!(identity_counts(&raw).await, (1, 0));
    sqlx::query("INSERT INTO repositories(id, owner_id, name, visibility, default_branch, created_at, updated_at) VALUES(100, 100, 'cloud', 'public', 'main', 3, 3)")
        .execute(&raw).await.unwrap();
    assert_eq!(identity_counts(&raw).await, (1, 0), "recreated repository remains unallocated until a run");
    let recreated_repo = new_run(&raw, 100, 1).await;
    assert_eq!(recreated_repo.0, original.0);
    assert_ne!(recreated_repo.1, original.1, "same repository ID and name is a distinct incarnation");
    sqlx::query("DELETE FROM accounts WHERE id = 100").execute(&raw).await.unwrap();
    assert_eq!(identity_counts(&raw).await, (0, 0), "identity rows follow account/repository deletion cascades");
    new_subject(&raw).await;
    assert_eq!(identity_counts(&raw).await, (0, 0));
    let recreated = new_run(&raw, 100, 1).await;
    assert_ne!(recreated.0, original.0, "same account ID and name is a distinct incarnation");
    assert_ne!(recreated.1, original.1);
    assert_ne!(recreated.1, recreated_repo.1);
    assert!(sqlx::query("PRAGMA foreign_key_check").fetch_all(&raw).await.unwrap().is_empty());
}
