//! Permanent agent markers deny preexisting human credentials and survive owner deletion.
use super::{Fixture, PASSWORD};
use rbit_core::{Timestamp, User};
use rbit_services::{ServiceError, ssh_keys::AddSshKey};
use rbit_storage::sqlite::recovery::{NewEmailToken, TokenPurpose};
use sqlx::sqlite::SqliteConnectOptions;

const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAU5O8WQTG+ptksHlmxF+ieJyp0kwsQ2tQsz0JAuTWme test";
const FINGERPRINT: &str = "SHA256:6yxaAtSseRpGJPcID+R+91fKBqIqIfu/nAYq0PCjYyo";

async fn assert_noninteractive(fixture: &Fixture, agent: &User) {
    assert!(agent.is_agent);
    assert!(matches!(
        fixture.services.accounts.authenticate(agent.name().as_str(), PASSWORD.into()).await,
        Err(ServiceError::InvalidCredentials)
    ));
    assert!(fixture.services.accounts.start_session(agent).await.is_err());
    assert!(fixture.services.two_factor.creation_options(agent).await.is_err());
    assert!(fixture.services.ssh_keys.add_user_key(agent, AddSshKey { title: "new".into(), key: KEY.into() }).await.is_err());
    assert!(fixture.services.ssh_keys.find_by_fingerprint(FINGERPRINT).await.unwrap().is_none());
    assert!(fixture.database.recovery().find_account(agent.id()).await.unwrap().is_none());
    assert!(fixture.database.recovery().find_account_by_email(&agent.email).await.unwrap().is_none());
    assert!(fixture.database.recovery().find_valid(&[12; 32], TokenPurpose::Reset, Timestamp::now()).await.unwrap().is_none());
    assert!(fixture.database.recovery().reset_password(&[12; 32], "replacement", Timestamp::now()).await.unwrap().is_none());
    assert!(fixture.database.profiles().password_hash(agent.id()).await.unwrap().is_none());
}

#[tokio::test]
async fn agent_credentials_and_orphaned_identities_remain_noninteractive() {
    let fixture = Fixture::new().await;
    fixture.user("root").await;
    let owner = fixture.user("owner").await;
    let candidate = fixture.user("candidate").await;
    let token = fixture.session(&candidate).await;
    fixture.services.ssh_keys.add_user_key(&candidate, AddSshKey { title: "existing".into(), key: KEY.into() }).await.unwrap();
    fixture
        .database
        .recovery()
        .insert(&NewEmailToken {
            digest: [12; 32],
            account_id: candidate.id(),
            purpose: TokenPurpose::Reset,
            email: candidate.email.clone(),
            expires_at: Timestamp(Timestamp::now().0 + 3600),
        })
        .await
        .unwrap();
    let pool = sqlx::SqlitePool::connect_with(rbit_storage::sqlite::writer_compatibility::register(
        SqliteConnectOptions::new().filename(fixture.dir.path().join("rbit.db")).foreign_keys(true),
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE user_logins SET is_agent=1 WHERE account_id=?").bind(candidate.id().0).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO agent_accounts(account_id,owner_id,slug,name,capabilities_json,created_at,updated_at) VALUES (?,?,'helper','Helper','[]',0,0)")
        .bind(candidate.id().0).bind(owner.id().0).execute(&pool).await.unwrap();
    let agent = fixture.database.accounts().find_user(candidate.id()).await.unwrap().unwrap();
    assert_eq!(agent.agent_owner_id, Some(owner.id()));
    assert_noninteractive(&fixture, &agent).await;
    assert!(!fixture.is_live(&token).await);
    assert!(fixture.kv.sessions().get(token.digest()).await.unwrap().is_none());
    fixture.database.admin().delete_user(owner.id()).await.unwrap();
    let orphan = fixture.database.accounts().find_user(candidate.id()).await.unwrap().unwrap();
    assert_eq!(orphan.agent_owner_id, None);
    assert_noninteractive(&fixture, &orphan).await;
    let snapshot = fixture.database.accounts().find_login_snapshot(candidate.name().as_str()).await.unwrap().unwrap();
    assert!(snapshot.login.user.is_agent);
    assert_eq!(snapshot.login.user.agent_owner_id, None);
    pool.close().await;
}
