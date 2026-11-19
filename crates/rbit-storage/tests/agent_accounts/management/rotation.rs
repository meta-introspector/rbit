//! Replacement is all-or-nothing across quotas, authority fences and candidate validation.
use super::super::{Timestamp, TokenScope, token};
use super::Fixture;
use rbit_core::AccessTokenId;
use rbit_storage::NewAccessToken;

#[tokio::test]
async fn agent_management_rotation_replaces_one_credential_atomically_at_the_ten_slot_cap() {
    let f = Fixture::new().await;
    let store = f.db.agent_accounts();
    let mut credentials = Vec::new();
    for byte in 1..=10 {
        let candidate = token(&f.db, f.agent.account_id, byte).await;
        credentials
            .push(store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, candidate.id, None).await.unwrap());
    }
    let replacement = token(&f.db, f.agent.account_id, 11).await;
    assert!(store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, replacement.id, None).await.is_err());
    assert_eq!(store.managed_credentials(f.owner.id(), f.agent.account_id).await.unwrap().len(), 10);
    let old = &credentials[0];
    let rotated =
        store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, replacement.id, Some(old.id)).await.unwrap();
    let live = store.managed_credentials(f.owner.id(), f.agent.account_id).await.unwrap();
    assert_eq!(live.len(), 10);
    assert!(live.iter().all(|credential| credential.id != old.id));
    assert!(live.iter().any(|credential| credential.id == rotated.id && credential.token.id == replacement.id));
    assert!(f.db.access_tokens().find_by_digest(&[1; 32]).await.unwrap().is_none());
    assert!(f.db.access_tokens().find_by_digest(&[11; 32]).await.unwrap().is_some());
    assert!(!store.credential_linked(f.agent.account_id, old.token.id).await.unwrap());
}

#[tokio::test]
async fn agent_management_failed_replacements_roll_back_the_old_digest_and_every_link() {
    let f = Fixture::new().await;
    let store = f.db.agent_accounts();
    let first = token(&f.db, f.agent.account_id, 1).await;
    let old = store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, first.id, None).await.unwrap();
    let linked = token(&f.db, f.agent.account_id, 2).await;
    let second = store.link_managed_credential(f.owner.id(), f.agent.account_id, f.agent.revision, linked.id, None).await.unwrap();
    let candidate = token(&f.db, f.agent.account_id, 3).await;
    let foreign = token(&f.db, f.other.id(), 4).await;
    let admin =
        f.db.access_tokens()
            .create(NewAccessToken {
                account_id: f.agent.account_id,
                name: "invalid admin candidate".into(),
                digest: [5; 32],
                hint: "test".into(),
                scope: TokenScope::Admin,
                expires_at: Some(Timestamp(4_000_000_000)),
            })
            .await
            .unwrap();
    for (actor, revision, replacement, target) in [
        (f.other.id(), f.agent.revision, candidate.id, old.id),
        (f.owner.id(), f.agent.revision + 1, candidate.id, old.id),
        (f.owner.id(), f.agent.revision, candidate.id, i64::MAX),
        (f.owner.id(), f.agent.revision, foreign.id, old.id),
        (f.owner.id(), f.agent.revision, admin.id, old.id),
        (f.owner.id(), f.agent.revision, AccessTokenId(i64::MAX), old.id),
        (f.owner.id(), f.agent.revision, linked.id, old.id),
        (f.owner.id(), f.agent.revision, first.id, old.id),
    ] {
        assert!(store.link_managed_credential(actor, f.agent.account_id, revision, replacement, Some(target)).await.is_err());
        let live = store.managed_credentials(f.owner.id(), f.agent.account_id).await.unwrap();
        assert_eq!(live.iter().map(|credential| credential.id).collect::<Vec<_>>(), [old.id, second.id]);
        assert!(f.db.access_tokens().find_by_digest(&[1; 32]).await.unwrap().is_some());
        assert!(store.credential_linked(f.agent.account_id, first.id).await.unwrap());
        assert!(!store.credential_linked(f.agent.account_id, candidate.id).await.unwrap());
    }
}
