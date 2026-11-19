//! Exact review diffs and a rejected pre-publication guard cannot move a branch.
use super::*;
#[tokio::test]
async fn review_guard_refuses_before_publishing_prepared_objects() {
    let f = Fixture::new();
    let parent = f.tip("main");
    let workspace = EditWorkspace::open(&f.env, &f.bare).await.unwrap();
    let blob = workspace.write_blob(&b"reviewed\n"[..], 100).await.unwrap();
    let changes = [Change::Update { path: path("README.md"), blob }];
    let spec = EditSpec {
        branch: "main",
        parent: &parent,
        new_branch: None,
        changes: &changes,
        message: "Review",
        author: &f.author,
        at: Timestamp::now(),
    };
    let denied = workspace.commit_guarded(&spec, async { Err::<(), _>(EditError::Rejected("grant revoked".into())) }).await;
    assert!(matches!(denied, Err(EditError::Rejected(_))));
    assert_eq!(f.tip("main"), parent);
    let result = workspace.commit(&spec).await.unwrap();
    let reader = rbit_git::RepositoryReader::new(f.bare.clone());
    let diff = reader.review_changes(parent.clone(), result.commit.clone(), rbit_git::DiffLimits::default()).await.unwrap();
    assert_eq!(diff.files.len(), 1);
    let fingerprint = reader.review_fingerprints(parent.clone(), result.commit.clone(), &diff.files).await.unwrap();
    let reversed = reader.review_fingerprints(result.commit, parent, &diff.files).await.unwrap();
    assert_ne!(fingerprint, reversed);
    assert_eq!(fingerprint[0].1.len(), 64);
    assert!(fingerprint[0].1.bytes().all(|byte| byte.is_ascii_hexdigit()));
}
