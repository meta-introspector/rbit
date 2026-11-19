//! Agent scope ceilings, live revocation, credential ownership and immutable identity markers.
use rbit_core::{AccountId, OrgRole, Permission, RepoId, Timestamp, TokenScope, User, Visibility};
use rbit_identity_models::agents::{AgentAccess, AgentInput, AgentScope, Capability};
use rbit_storage::{
    Database, NewAccessToken, NewRepository, NewUser,
    sqlite::{agent_accounts::AgentError, orgs::NewOrganization},
};

mod dispatch;
mod keyed_grants;
mod management;

async fn user(db: &Database, name: &str) -> User {
    db.accounts()
        .create_user(NewUser {
            name: name.parse().unwrap(),
            email: format!("{name}@example.com").parse().unwrap(),
            display_name: None,
            password_hash: "unused".into(),
            is_admin: false,
        })
        .await
        .unwrap()
}
async fn repo(db: &Database, owner: AccountId, name: &str, visibility: Visibility) -> RepoId {
    db.repositories()
        .create(NewRepository {
            owner_id: owner,
            name: name.parse().unwrap(),
            description: None,
            visibility,
            default_branch: "main".into(),
        })
        .await
        .unwrap()
        .id
}
fn input(scope: AgentScope) -> AgentInput {
    AgentInput {
        slug: "helper".into(),
        name: "Build helper".into(),
        capabilities: vec![Capability::Read, Capability::Push],
        scopes: vec![scope],
    }
}
async fn token(db: &Database, account_id: AccountId, byte: u8) -> rbit_core::AccessToken {
    db.access_tokens()
        .create(NewAccessToken {
            account_id,
            name: "worker".into(),
            digest: [byte; 32],
            hint: "test".into(),
            scope: TokenScope::Write,
            expires_at: Some(Timestamp(4_000_000_000)),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn organization_scopes_follow_current_membership_future_repositories_and_policy() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let other = user(&db, "other").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "team".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap()
        .account
        .id;
    let initial = repo(&db, org, "initial", Visibility::Private).await;
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &input(AgentScope::Organization { id: org.0, access: AgentAccess::Write })).await.unwrap();
    assert_ne!(agent.account_id, owner.id());
    assert_eq!(store.repository_permission(agent.account_id, initial).await.unwrap(), Some(AgentAccess::Write));
    let future = repo(&db, org, "future", Visibility::Private).await;
    assert_eq!(store.repository_permission(agent.account_id, future).await.unwrap(), Some(AgentAccess::Write));
    let outside = repo(&db, other.id(), "public", Visibility::Public).await;
    assert_eq!(store.repository_permission(agent.account_id, outside).await.unwrap(), None);
    store.set_organization_policy(owner.id(), org, false).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, initial).await.unwrap(), None);
    assert!(matches!(store.set_organization_policy(other.id(), org, true).await, Err(AgentError::NotFound)));
    store.set_organization_policy(owner.id(), org, true).await.unwrap();
    db.orgs().add_member(org, other.id(), OrgRole::Owner).await.unwrap();
    db.orgs().remove_member(org, owner.id()).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, initial).await.unwrap(), None);
    assert_eq!(store.repository_permission(agent.account_id, future).await.unwrap(), None);
}

#[tokio::test]
async fn repository_scopes_never_exceed_live_owner_access_or_become_administration() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let author = user(&db, "author").await;
    let private = repo(&db, author.id(), "private", Visibility::Private).await;
    let settings = input(AgentScope::Repository { id: private.0, access: AgentAccess::Write });
    let store = db.agent_accounts();
    assert!(matches!(store.create(owner.id(), &settings).await, Err(AgentError::NotFound)));
    assert!(store.list(owner.id()).await.unwrap().is_empty(), "failed scope validation rolls back identity creation");
    db.collaborators().add(private, owner.id(), Permission::Admin, author.id()).await.unwrap();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, private).await.unwrap(), Some(AgentAccess::Write));
    db.collaborators().set_permission(private, owner.id(), Permission::Read).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, private).await.unwrap(), Some(AgentAccess::Read));
    db.collaborators().remove(private, owner.id()).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, private).await.unwrap(), None);
    assert!(matches!(store.create(agent.account_id, &settings).await, Err(AgentError::Forbidden)), "agents cannot create agents");
    store.disable(owner.id(), agent.account_id, agent.revision).await.unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_none());
}

#[tokio::test]
async fn credentials_are_owner_fenced_and_disabling_permanently_revokes_them() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let other = user(&db, "other").await;
    let repository = repo(&db, owner.id(), "repo", Visibility::Private).await;
    let settings = input(AgentScope::Repository { id: repository.0, access: AgentAccess::Write });
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    let foreign = token(&db, other.id(), 1).await;
    assert!(matches!(store.link_credential(owner.id(), agent.account_id, agent.revision, foreign.id).await, Err(AgentError::NotFound)));
    let credential = token(&db, agent.account_id, 2).await;
    assert!(!store.credential_linked(agent.account_id, credential.id).await.unwrap());
    store.link_credential(owner.id(), agent.account_id, agent.revision, credential.id).await.unwrap();
    assert!(store.credential_linked(agent.account_id, credential.id).await.unwrap());
    assert!(matches!(store.credentials(other.id(), agent.account_id).await, Err(AgentError::NotFound)));
    assert_eq!(store.credentials(owner.id(), agent.account_id).await.unwrap().len(), 1);
    store.disable(owner.id(), agent.account_id, agent.revision).await.unwrap();
    assert!(db.access_tokens().find_by_digest(&[2; 32]).await.unwrap().is_none());
    let disabled = store.find(owner.id(), agent.account_id).await.unwrap();
    store.update(owner.id(), agent.account_id, disabled.revision, &settings, true).await.unwrap();
    assert!(!store.credential_linked(agent.account_id, credential.id).await.unwrap(), "reenabling cannot revive a revoked credential");
}

#[tokio::test]
async fn concurrent_updates_use_revisions_and_deleted_scopes_do_not_retarget() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let db = Database::open(&path).await.unwrap();
    let owner = user(&db, "owner").await;
    let repository = repo(&db, owner.id(), "repo", Visibility::Private).await;
    let settings = input(AgentScope::Repository { id: repository.0, access: AgentAccess::Write });
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    let (left, right) = tokio::join!(
        store.update(owner.id(), agent.account_id, agent.revision, &settings, true),
        store.update(owner.id(), agent.account_id, agent.revision, &settings, true)
    );
    assert_ne!(left.is_ok(), right.is_ok());
    let rejected = if left.is_err() { left } else { right };
    assert!(matches!(rejected, Err(AgentError::Conflict)));
    let pool = super::writer_connection::pool(&path).await;
    sqlx::query("DELETE FROM repositories WHERE id = ?").bind(repository.0).execute(&pool).await.unwrap();
    assert_eq!(store.scopes(owner.id(), agent.account_id).await.unwrap(), Vec::<AgentScope>::new());
    let recreated = repo(&db, owner.id(), "repo", Visibility::Private).await;
    assert_eq!(store.repository_permission(agent.account_id, recreated).await.unwrap(), None);
    let orphaned_token = token(&db, agent.account_id, 9).await;
    sqlx::query("DELETE FROM accounts WHERE id = ?").bind(owner.id().0).execute(&pool).await.unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_none());
    assert!(db.access_tokens().find_by_digest(&[9; 32]).await.unwrap().is_none());
    assert!(!store.credential_linked(agent.account_id, orphaned_token.id).await.unwrap());
    let marker: bool = sqlx::query_scalar("SELECT is_agent FROM user_logins WHERE account_id = ?")
        .bind(agent.account_id.0)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(marker, "orphaned identities must never turn into interactive human accounts");
}

#[tokio::test]
async fn organization_policy_and_parent_status_apply_to_explicit_repository_grants() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let db = Database::open(&path).await.unwrap();
    let owner = user(&db, "owner").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "team".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap()
        .account
        .id;
    let repository = repo(&db, org, "private", Visibility::Private).await;
    let mut settings = input(AgentScope::Repository { id: repository.0, access: AgentAccess::Write });
    settings.scopes.push(AgentScope::Organization { id: org.0, access: AgentAccess::Read });
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, repository).await.unwrap(), Some(AgentAccess::Write));
    store.set_organization_policy(owner.id(), org, false).await.unwrap();
    assert_eq!(store.repository_permission(agent.account_id, repository).await.unwrap(), None, "explicit scope cannot bypass org policy");
    store.set_organization_policy(owner.id(), org, true).await.unwrap();
    let credential = token(&db, agent.account_id, 7).await;
    store.link_credential(owner.id(), agent.account_id, agent.revision, credential.id).await.unwrap();
    let pool = super::writer_connection::pool(&path).await;
    sqlx::query("UPDATE user_logins SET suspended_at = 1 WHERE account_id = ?").bind(owner.id().0).execute(&pool).await.unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_none());
    assert_eq!(store.repository_permission(agent.account_id, repository).await.unwrap(), None);
    sqlx::query("UPDATE user_logins SET suspended_at = NULL, password_change_required = 1 WHERE account_id = ?")
        .bind(owner.id().0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_none());
    assert_eq!(store.repository_permission(agent.account_id, repository).await.unwrap(), None);
    sqlx::query("UPDATE user_logins SET password_change_required = 0, authentication_epoch = 42 WHERE account_id = ?")
        .bind(owner.id().0)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO credential_revocations(account_id, target_epoch, created_at) VALUES (?, 42, 1)")
        .bind(owner.id().0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_none());
    assert_eq!(store.repository_permission(agent.account_id, repository).await.unwrap(), None);
    assert!(matches!(store.list(owner.id()).await, Err(AgentError::Forbidden)));
    sqlx::query("UPDATE credential_revocations SET completed_at = 2, outcome = 'ended_all' WHERE account_id = ?")
        .bind(owner.id().0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store.active(agent.account_id).await.unwrap().is_some());
    assert!(db.access_tokens().find_by_digest(&[7; 32]).await.unwrap().is_none());
    assert!(!store.credential_linked(agent.account_id, credential.id).await.unwrap(), "completion never revives prior child credentials");
    let password_credential = token(&db, agent.account_id, 8).await;
    store.link_credential(owner.id(), agent.account_id, agent.revision, password_credential.id).await.unwrap();
    sqlx::query("UPDATE user_logins SET password_hash = 'changed' WHERE account_id = ?").bind(owner.id().0).execute(&pool).await.unwrap();
    assert!(db.access_tokens().find_by_digest(&[8; 32]).await.unwrap().is_none(), "legacy password changes also revoke agent credentials");
}
