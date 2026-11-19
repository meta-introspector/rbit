//! Stale management handles never acquire a new meaning when SQLite reuses a token rowid.
use super::super::{AgentError, token};
use super::Fixture;

#[tokio::test]
async fn agent_management_stable_ids_refuse_stale_revoke_and_rotation_after_real_rowid_reuse() {
    let f = Fixture::new().await;
    let store = f.db.agent_accounts();
    let first = token(&f.db, f.agent.account_id, 1).await;
    let old = store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, first.id, None).await.unwrap();
    store.revoke_managed_credential(f.owner.id(), f.agent.account_id, old.id).await.unwrap();
    let reused = token(&f.db, f.agent.account_id, 2).await;
    assert_eq!(reused.id, first.id, "fixture must exercise actual access_tokens rowid reuse");
    let current = store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, reused.id, None).await.unwrap();
    assert!(current.id > old.id, "stable management handles must never be recycled");
    assert!(matches!(store.revoke_managed_credential(f.owner.id(), f.agent.account_id, old.id).await, Err(AgentError::NotFound)));
    let candidate = token(&f.db, f.agent.account_id, 3).await;
    assert!(matches!(
        store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, candidate.id, Some(old.id)).await,
        Err(AgentError::NotFound)
    ));
    let live = store.managed_credentials(f.owner.id(), f.agent.account_id).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!((live[0].id, live[0].token.id), (current.id, reused.id));
    assert!(store.credential_linked(f.agent.account_id, reused.id).await.unwrap());
    assert!(!store.credential_linked(f.agent.account_id, candidate.id).await.unwrap());
    assert!(f.db.access_tokens().find_by_digest(&[2; 32]).await.unwrap().is_some());
}
