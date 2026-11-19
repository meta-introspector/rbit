//! Exact migration SQL preserves existing links and makes new public handles monotonic.
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
#[tokio::test]
async fn agent_management_identity_upgrade_backfills_links_and_never_reuses_deleted_ids() {
    let dir = tempfile::tempdir().unwrap();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::new().filename(dir.path().join("db")).create_if_missing(true).foreign_keys(true))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE accounts(id INTEGER PRIMARY KEY) STRICT;
        CREATE TABLE access_tokens(id INTEGER PRIMARY KEY,account_id INTEGER NOT NULL REFERENCES accounts(id),scope TEXT NOT NULL) STRICT;
        CREATE TABLE agent_accounts(account_id INTEGER PRIMARY KEY REFERENCES accounts(id)) STRICT;
        CREATE TABLE agent_credentials(token_id INTEGER PRIMARY KEY REFERENCES access_tokens(id) ON DELETE CASCADE,agent_id INTEGER NOT NULL REFERENCES agent_accounts(account_id) ON DELETE CASCADE,created_at INTEGER NOT NULL) STRICT;
        CREATE INDEX agent_credentials_identity ON agent_credentials(agent_id);
        INSERT INTO accounts VALUES(1),(2);
        INSERT INTO agent_accounts VALUES(2);
        INSERT INTO access_tokens VALUES(3,2,'write'),(9,2,'read');
        INSERT INTO agent_credentials VALUES(3,2,100),(9,2,101);")
        .execute(&pool).await.unwrap();
    sqlx::raw_sql(include_str!("../../../migrations/0105_agent_credential_identity.sql")).execute(&pool).await.unwrap();
    let rows: Vec<(i64, i64, i64, i64)> =
        sqlx::query_as("SELECT id,token_id,agent_id,created_at FROM agent_credentials ORDER BY token_id").fetch_all(&pool).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!((rows[0].1, rows[0].2, rows[0].3), (3, 2, 100));
    assert_eq!((rows[1].1, rows[1].2, rows[1].3), (9, 2, 101));
    sqlx::query("DELETE FROM access_tokens WHERE id=9").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO access_tokens VALUES(9,2,'write')").execute(&pool).await.unwrap();
    let next: i64 = sqlx::query_scalar("INSERT INTO agent_credentials(token_id,agent_id,created_at) VALUES(9,2,102) RETURNING id")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(next > rows[1].0);
    let retained: i64 = sqlx::query_scalar("SELECT id FROM agent_credentials WHERE token_id=3").fetch_one(&pool).await.unwrap();
    assert_eq!(retained, rows[0].0);
    pool.close().await;
}
