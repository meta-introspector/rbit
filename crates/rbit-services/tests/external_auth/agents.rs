//! Delegated identities cannot acquire or use interactive provider credentials.
use super::{Fixture, ServiceError, identity};

#[tokio::test]
async fn agent_provider_credentials_and_link_attempts_are_refused() {
    let fx = Fixture::new().await;
    let user = fx.user("candidate").await;
    let claimed = identity("corp", "agent-subject", None, false);
    fx.complete(claimed.clone(), Some(user.id())).await.unwrap();
    let pool = sqlx::SqlitePool::connect_with(rbit_storage::sqlite::writer_compatibility::register(
        sqlx::sqlite::SqliteConnectOptions::new().filename(fx.dir.path().join("rbit.db")),
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE user_logins SET is_agent=1,is_admin=0 WHERE account_id=?").bind(user.id().0).execute(&pool).await.unwrap();
    assert!(matches!(fx.complete(claimed, None).await, Err(ServiceError::Forbidden)));
    assert!(matches!(
        fx.complete(identity("github", "new-agent-subject", None, false), Some(user.id())).await,
        Err(ServiceError::Forbidden)
    ));
    pool.close().await;
}
