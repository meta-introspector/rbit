//! Overlapping scope maxima and live revocation agree across keyed repository reads.
use super::*;
use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

async fn assert_access(
    db: &Database,
    pool: &SqlitePool,
    agent: AccountId,
    org: AccountId,
    repository: RepoId,
    name: &str,
    level: Option<AgentAccess>,
) {
    assert_eq!(db.agent_accounts().repository_permission(agent, repository).await.unwrap(), level);
    let listing: Option<i64> = sqlx::query_scalar("SELECT permission FROM agent_repository_grants WHERE agent_id = ? AND repo_id = ?")
        .bind(agent.0)
        .bind(repository.0)
        .fetch_optional(pool)
        .await
        .unwrap();
    assert_eq!(listing, level.map(AgentAccess::level));
    let permission = level.map(|value| match value {
        AgentAccess::Read => Permission::Read,
        AgentAccess::Write => Permission::Write,
    });
    assert_eq!(db.repositories().grants(repository, org, agent).await.unwrap().collaborator, permission);
    let page = db.repositories().find_page("team", name, Some(agent), false).await.unwrap().unwrap();
    assert_eq!(page.grants.collaborator, permission);
    let readable = db.repositories().readable_among(&[repository], agent, false).await.unwrap();
    assert_eq!(readable.contains_key(&repository), level.is_some());
}

#[tokio::test]
async fn overlapping_keyed_grants_keep_maximum_and_live_revocation_without_public_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let db = Database::open(&directory.path().join("db")).await.unwrap();
    let pool = SqlitePool::connect_with(SqliteConnectOptions::new().filename(directory.path().join("db"))).await.unwrap();
    let owner = user(&db, "owner").await;
    let other = user(&db, "other").await;
    let org = db
        .orgs()
        .create(NewOrganization { name: "team".parse().unwrap(), display_name: None, description: None, creator: owner.id() })
        .await
        .unwrap()
        .account
        .id;
    let initial = repo(&db, org, "initial", Visibility::Public).await;
    let mut settings = input(AgentScope::Organization { id: org.0, access: AgentAccess::Read });
    settings.scopes.push(AgentScope::Repository { id: initial.0, access: AgentAccess::Write });
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &settings).await.unwrap();
    assert_access(&db, &pool, agent.account_id, org, initial, "initial", Some(AgentAccess::Write)).await;
    let future = repo(&db, org, "future", Visibility::Private).await;
    assert_access(&db, &pool, agent.account_id, org, future, "future", Some(AgentAccess::Read)).await;
    let outside = repo(&db, other.id(), "public", Visibility::Public).await;
    assert_eq!(store.repository_permission(agent.account_id, outside).await.unwrap(), None);
    assert!(!db.repositories().readable_among(&[outside], agent.account_id, true).await.unwrap().contains_key(&outside));

    store.set_organization_policy(owner.id(), org, false).await.unwrap();
    assert_access(&db, &pool, agent.account_id, org, initial, "initial", None).await;
    assert_access(&db, &pool, agent.account_id, org, future, "future", None).await;
    store.set_organization_policy(owner.id(), org, true).await.unwrap();
    db.admin().set_suspended(owner.id(), true).await.unwrap();
    assert_access(&db, &pool, agent.account_id, org, initial, "initial", None).await;
    db.admin().set_suspended(owner.id(), false).await.unwrap();
    assert_access(&db, &pool, agent.account_id, org, initial, "initial", Some(AgentAccess::Write)).await;
    db.orgs().add_member(org, other.id(), OrgRole::Owner).await.unwrap();
    db.orgs().remove_member(org, owner.id()).await.unwrap();
    assert_access(&db, &pool, agent.account_id, org, initial, "initial", Some(AgentAccess::Read)).await;
    assert_access(&db, &pool, agent.account_id, org, future, "future", None).await;
}
