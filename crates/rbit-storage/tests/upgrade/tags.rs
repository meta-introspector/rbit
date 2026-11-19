//! Tag protection regression coverage.

use super::*;

#[tokio::test]
async fn tag_protection_upgrade_is_empty_and_rules_are_repository_scoped() {
    let fixture = upgraded().await;
    assert!(fixture.exists("table", "tag_protections").await);
    let store = fixture.database.tag_protection();
    assert_eq!(store.list(ROCKET).await.unwrap(), []);
    let pattern = "v*".parse().unwrap();
    let id = store.create(ROCKET, &pattern).await.unwrap().unwrap();
    assert!(matches!(store.create(ROCKET, &pattern).await, Err(rbit_storage::StorageError::Conflict(_))));
    assert!(!store.delete(RepoId(ROCKET.0 + 1), id).await.unwrap());
    assert!(store.delete(ROCKET, id).await.unwrap());
}
