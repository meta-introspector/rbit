//! Real JWT signatures and unambiguous, bounded cloud trust subjects.
use super::*;
use p256::ecdsa::{VerifyingKey, signature::Verifier as _};

#[test]
fn published_jwk_verifies_signature_and_detects_tampering() {
    let key = Key::generate().unwrap();
    let decoded = Key::decode(&key.encode()).unwrap();
    assert_eq!(decoded.jwk(), key.jwk());
    let token = key.sign(&json!({"iss": "https://forge.example/oidc", "aud": "sts.amazonaws.com", "exp": 300})).unwrap();
    assert_eq!(crate::logs::Masker::none().mask(&format!("identity={token}")), "identity=***");
    let (signed, signature) = token.rsplit_once('.').unwrap();
    let signature = Signature::from_slice(&URL_SAFE_NO_PAD.decode(signature).unwrap()).unwrap();
    let jwk = key.jwk();
    let mut point = vec![4];
    point.extend(URL_SAFE_NO_PAD.decode(jwk["x"].as_str().unwrap()).unwrap());
    point.extend(URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap());
    let verifier = VerifyingKey::from_sec1_bytes(&point).unwrap();
    verifier.verify(signed.as_bytes(), &signature).unwrap();
    assert!(verifier.verify(format!("{signed}x").as_bytes(), &signature).is_err());
    assert!(jwk.get("d").is_none());
    assert!(!format!("{key:?}").contains(key.encode().as_str()));
    assert_ne!(key.kid(), Key::generate().unwrap().kid());
}

#[test]
fn subjects_cannot_alias_or_follow_recreated_names() {
    let repository = "a".repeat(32);
    let owner = "b".repeat(32);
    let plain = subject(&repository, &owner, "refs/heads/main", None).unwrap();
    assert!(plain.ends_with(":ref:refs/heads/main"));
    assert_ne!(plain, subject(&"c".repeat(32), &owner, "refs/heads/main", None).unwrap());
    assert_ne!(plain, subject(&repository, &"c".repeat(32), "refs/heads/main", None).unwrap());
    assert_ne!(
        subject(&repository, &owner, "", Some("prod:west")).unwrap(),
        subject(&repository, &owner, "", Some("prod%3Awest")).unwrap()
    );
    assert!(subject(&repository, &owner, &"x".repeat(256), None).is_err());
    assert!(subject("1", &owner, "refs/heads/main", None).is_err());
    for invalid in ["", "aud\nvalue", " leading", "trailing ", &"x".repeat(513)] {
        assert!(audience(invalid).is_err());
    }
    assert!(audience("https://iam.googleapis.com/projects/123/locations/global/workloadIdentityPools/p/providers/r").is_ok());
}
