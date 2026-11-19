//! Reviewer isolation and publication fences against concurrent SQLite mutations.
use super::*;
#[tokio::test]
async fn review_progress_is_private_and_publication_fence_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let db = Database::open(&dir.path().join("rbit.db")).await.unwrap();
    let alice = user(&db, "alice").await;
    let bob = user(&db, "bob").await;
    let repo = repo(&db, &alice, "demo", Visibility::Public).await;
    let (pull, _) = created(&db, &new_pull(repo, repo, &alice, "feature", 1)).await;
    let store = db.reviews();
    store.set_progress(pull.id, alice.id(), "a.rs", Some("before:after")).await.unwrap();
    assert_eq!(store.progress(pull.id, bob.id()).await.unwrap(), Vec::<(String, String)>::new());
    assert_eq!(store.progress(pull.id, alice.id()).await.unwrap(), [("a.rs".into(), "before:after".into())]);
    assert!(store.last_reviewed_head(pull.id, alice.id()).await.unwrap().is_none());
    let fence = store.publication_fence().await.unwrap();
    let writer = store.clone();
    let mut mutation = tokio::spawn(async move { writer.set_progress(pull.id, alice.id(), "a.rs", None).await });
    assert!(tokio::time::timeout(std::time::Duration::from_millis(20), &mut mutation).await.is_err());
    // Readers continue while policy writers cannot pass the publication boundary.
    assert_eq!(store.progress(pull.id, bob.id()).await.unwrap(), Vec::<(String, String)>::new());
    drop(fence);
    tokio::time::timeout(std::time::Duration::from_secs(5), mutation).await.unwrap().unwrap().unwrap();
    db.close().await;
}
