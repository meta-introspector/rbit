//! Disabled identities remain editable without disclosing renamed resources after owner access loss.
use super::super::{AgentAccess, AgentScope, Database, Permission, Visibility, input, repo, user};

#[tokio::test]
async fn agent_management_scope_names_follow_human_visibility_even_when_agent_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("db")).await.unwrap();
    let owner = user(&db, "owner").await;
    let author = user(&db, "author").await;
    let private = repo(&db, author.id(), "private", Visibility::Private).await;
    db.collaborators().add(private, owner.id(), Permission::Write, author.id()).await.unwrap();
    let store = db.agent_accounts();
    let agent = store.create(owner.id(), &input(AgentScope::Repository { id: private.0, access: AgentAccess::Write })).await.unwrap();
    store.disable(owner.id(), agent.account_id, agent.revision).await.unwrap();
    assert_eq!(store.scope_names(owner.id(), agent.account_id).await.unwrap()[0].1.as_deref(), Some("author/private"));
    db.repo_settings().rename(private, &"renamed".parse().unwrap()).await.unwrap();
    assert_eq!(store.scope_names(owner.id(), agent.account_id).await.unwrap()[0].1.as_deref(), Some("author/renamed"));
    db.collaborators().remove(private, owner.id()).await.unwrap();
    db.repo_settings().rename(private, &"hidden-new-name".parse().unwrap()).await.unwrap();
    let hidden = store.scope_names(owner.id(), agent.account_id).await.unwrap();
    assert_eq!(hidden.len(), 1);
    assert_eq!(hidden[0].1, None, "current private names cannot survive human access revocation");
    assert!(store.scope_names(author.id(), agent.account_id).await.is_err());
    db.collaborators().add(private, owner.id(), Permission::Read, author.id()).await.unwrap();
    assert_eq!(store.scope_names(owner.id(), agent.account_id).await.unwrap()[0].1.as_deref(), Some("author/hidden-new-name"));
    assert!(store.active(agent.account_id).await.unwrap().is_none(), "metadata visibility does not reenable the agent");
}
