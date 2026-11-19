//! Authentication policy regressions.
use super::*;

#[tokio::test]
async fn disabled_provider_cannot_start_sign_in_link_or_complete_verified_callback() {
    let fixture = Fixture::with(|config| config.auth.providers[0].disabled = true).await;
    assert!(!fixture.services.external_auth.providers().iter().any(|provider| provider.id == "corp"));
    assert!(matches!(fixture.services.external_auth.begin("corp", "/", None).await, Err(ServiceError::NotFound)));
    let user = fixture.user("ada").await;
    assert!(matches!(fixture.complete(identity("corp", "disabled", None, false), None).await, Err(ServiceError::NotFound)));
    assert!(fixture.services.external_auth.methods(&user).await.unwrap().linkable.iter().all(|provider| provider.id != "corp"));
    assert!(matches!(fixture.services.external_auth.begin("corp", "/", Some(user.id())).await, Err(ServiceError::NotFound)));
}
