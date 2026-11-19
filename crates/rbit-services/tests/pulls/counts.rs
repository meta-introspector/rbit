//! Comparison-count cache scope and file-budget tests.
use super::*;

#[tokio::test]
async fn cached_counts_are_only_those_of_the_current_file_limit() {
    let f = Fixture::new().await;
    let number = f.create(&f.alice, "alice", "feature").await.unwrap();
    let ctx = f.open_pull(&f.alice, number).await;
    let pulls = &f.services.pulls;
    let (base, head) = pulls.compared_ids(&ctx).await.unwrap();
    assert_eq!(pulls.cached_counts(&base, &head).await, None);
    pulls.changes(&ctx, DiffLimits::default()).await.unwrap();
    let counted = PullCounts { ahead: 1, files: 1 };
    assert_eq!(pulls.cached_counts(&base, &head).await, Some(counted), "the Files changed tab counts for the others");

    // Counts made under another file limit (another server version) are not this answer.
    let other = CachedComparisonCounts { ahead: 1, files: 0, max_files: 0, additions: 0, deletions: 0 };
    f.kv.insights().put_comparison_counts(&base, &head, other).await.unwrap();
    assert_eq!(pulls.cached_counts(&base, &head).await, None);
    assert_eq!(pulls.counts_between(&ctx, &base, &head).await.unwrap(), counted, "recounted");
    assert_eq!(pulls.cached_counts(&base, &head).await, Some(counted), "and remembered again");
    // Only for the pull request's own pair.
    assert!(matches!(pulls.counts_between(&ctx, &head, &base).await, Err(ServiceError::NotFound)));
}
