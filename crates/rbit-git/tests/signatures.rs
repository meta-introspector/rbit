//! Immutable signature extraction preserves nested headers and rejects ambiguity/oversize.
use crate::hostile_support::Repo;
use rbit_git::RepositoryReader;

#[tokio::test]
async fn signatures_extract_exact_bytes_and_reject_ambiguous_or_oversized_objects() {
    let repo = Repo::new(&[("README.md", b"signed\n")]);
    let reader = RepositoryReader::new(repo.bare.clone());
    let tree = repo.bare(&["rev-parse", "main^{tree}"]);
    let prefix = format!(
        "tree {tree}\nauthor Signer <signer@example.com> 1700000000 +0000\ncommitter Signer <signer@example.com> 1700000000 +0000\nmergetag object {}\n type commit\n tag nested\n \n gpgsig nested-is-not-the-commit-signature\n",
        repo.tip("main")
    );
    let message = "\nmessage\ngpgsig this is only message text\n";
    let raw = format!("{prefix}gpgsig first\n second\n{message}");
    let id = repo.bare_with_input(&["hash-object", "-w", "-t", "commit", "--stdin"], raw.as_bytes());
    let extracted = reader.signed_object(id).await.unwrap();
    assert_eq!(extracted.payload, format!("{prefix}{message}").as_bytes());
    assert!(extracted.signature.unwrap().starts_with(b"first\nsecond"));
    let duplicate = format!("{prefix}gpgsig first\ngpgsig second\n{message}");
    let id = repo.bare_with_input(&["hash-object", "-w", "-t", "commit", "--stdin"], duplicate.as_bytes());
    assert!(reader.signed_object(id).await.is_err());
    let oversized = format!("{prefix}\n{}", "x".repeat(1024 * 1024));
    let id = repo.bare_with_input(&["hash-object", "-w", "-t", "commit", "--stdin"], oversized.as_bytes());
    assert!(reader.signed_object(id).await.is_err());
    assert!(reader.signed_object(tree).await.is_err());
    assert!(reader.signed_object("main".into()).await.is_err());
    assert!(reader.signed_object(repo.tip("main")).await.unwrap().signature.is_none());
}
