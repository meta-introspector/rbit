//! Opt-in MFA enrollment policy across membership, sessions and existing credentials.
use rbit_auth::{TotpSecret, totp::time_step};
use rbit_core::{Config, OrgRole, Timestamp, User, Visibility};
use rbit_identity_models::sessions::ClientInfo;
use rbit_services::{CreateAccessToken, CreateRepository, ServiceError, Services, SignUp, orgs::CreateOrganization};
use rbit_storage::{Database, KvStore};
use std::sync::Arc;

async fn user(services: &Services, name: &str) -> User {
    services
        .accounts
        .create_user(SignUp { username: name.into(), email: format!("{name}@example.com"), password: "correct horse battery".into() }, None)
        .await
        .unwrap()
}

async fn enroll(services: &Services, database: &Database, user: &User) {
    services.two_factor.begin_enrolment(user, "Tests").await.unwrap();
    let record = database.two_factor().find(user.id()).await.unwrap().unwrap();
    let secret = TotpSecret::from_bytes(&record.secret).unwrap();
    let code = format!("{:06}", secret.code_at_step(time_step(Timestamp::now().seconds())));
    services.confirm_two_factor(user, &code, &ClientInfo::default()).await.unwrap().unwrap();
}

#[tokio::test]
async fn opt_in_policies_recheck_existing_tokens_and_preserve_enrollment_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.storage.data_dir = dir.path().into();
    std::fs::create_dir_all(config.storage.repositories_dir()).unwrap();
    let database = Database::open(&config.storage.sqlite_path()).await.unwrap();
    let kv = KvStore::open(&config.storage.kv_path()).unwrap();
    let services = Services::new(Arc::new(config.clone()), &database, &kv).await.unwrap();
    let admin = user(&services, "site-owner").await;
    let member = user(&services, "member").await;
    let outsider = user(&services, "outsider").await;
    let policy = &services.authentication_policy;
    assert!(!policy.site_required().await.unwrap());
    let (needed, work) = rbit_storage::count_work(policy.needs_enrollment(member.id())).await;
    assert!(!needed.unwrap());
    assert_eq!(work.statements, 0, "optional policies preserve request query budgets");
    assert!(matches!(policy.set_site(&member, true).await, Err(ServiceError::Forbidden)));
    assert!(matches!(policy.set_site(&admin, true).await, Err(ServiceError::Invalid(_))));
    enroll(&services, &database, &admin).await;
    let (_, token) = services.tokens.create(&member, CreateAccessToken { name: "existing".into(), ..Default::default() }).await.unwrap();
    services
        .repositories
        .create(&member, CreateRepository { name: "demo".into(), description: String::new(), visibility: Visibility::Public })
        .await
        .unwrap();
    let session = services.accounts.start_session(&member).await.unwrap();
    assert!(services.tokens.authenticate(&token).await.unwrap().is_some());
    policy.set_site(&admin, true).await.unwrap();
    assert!(matches!(
        services.disable_two_factor(&admin, "correct horse battery".into(), "000000", &ClientInfo::default()).await,
        Err(ServiceError::Invalid(_))
    ));
    assert!(policy.needs_enrollment(member.id()).await.unwrap());
    assert!(matches!(services.repositories.open(Some(&member), "member", "demo").await, Err(ServiceError::Forbidden)));
    assert!(services.tokens.authenticate(&token).await.unwrap().is_none());
    assert!(services.tokens.authenticate_limited(&token).await.unwrap().is_none());
    assert!(services.accounts.resume_session(&session).await.unwrap().is_some(), "session survives for enrollment");
    let reloaded = Services::new(Arc::new(config), &database, &kv).await.unwrap();
    assert!(reloaded.authentication_policy.site_required().await.unwrap(), "policy survives restart");
    policy.set_site(&admin, false).await.unwrap();
    assert!(services.tokens.authenticate(&token).await.unwrap().is_some());
    let org = services
        .orgs
        .create(&admin, CreateOrganization { name: "acme".into(), display_name: String::new(), description: String::new() })
        .await
        .unwrap();
    assert!(!policy.organization_required(org.account.id).await.unwrap());
    assert!(matches!(policy.set_organization(&member, &org, true).await, Err(ServiceError::NotFound)));
    services.orgs.add_member(&admin, &org, "member", OrgRole::Member).await.unwrap();
    policy.set_organization(&admin, &org, true).await.unwrap();
    assert!(policy.needs_enrollment(member.id()).await.unwrap());
    assert!(!policy.needs_enrollment(outsider.id()).await.unwrap());
    services.orgs.add_member(&admin, &org, "outsider", OrgRole::Member).await.unwrap();
    assert!(policy.needs_enrollment(outsider.id()).await.unwrap(), "new membership takes effect immediately");
    services.orgs.remove_member(&admin, &org, "outsider").await.unwrap();
    assert!(!policy.needs_enrollment(outsider.id()).await.unwrap(), "removed membership cannot leave a cached requirement");
    assert!(!policy.needs_enrollment(admin.id()).await.unwrap());
    assert!(services.tokens.authenticate(&token).await.unwrap().is_none());
    enroll(&services, &database, &member).await;
    assert!(!policy.needs_enrollment(member.id()).await.unwrap());
    assert!(services.tokens.authenticate(&token).await.unwrap().is_some());
    policy.set_organization(&admin, &org, false).await.unwrap();
    assert!(!policy.required(member.id()).await.unwrap());
}
