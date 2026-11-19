//! Tag protection regression coverage.

use super::*;

#[tokio::test]
async fn tag_rules_protect_pushes_and_release_writes() {
    use rbit_services::releases::ReleaseInput;
    let fixture = Box::pin(Fixture::new()).await;
    fixture.branches().await;
    let protection = &fixture.services.protection;
    let admin = fixture.repo(&fixture.olga).await;
    let writer = fixture.repo(&fixture.bob).await;
    assert!(matches!(protection.create_tag_rule(&writer, &fixture.bob, "v*").await, Err(ServiceError::Forbidden)));
    let id = protection.create_tag_rule(&admin, &fixture.olga, "v*").await.unwrap();
    let admin_actor = PushActor { user: Some(&fixture.olga), deploy_key: false, access: admin.access };
    let writer_actor = PushActor { user: Some(&fixture.bob), deploy_key: false, access: writer.access };
    let creation = update("refs/tags/v1", None, Some(ID));
    assert!(!protection.check_push(&admin, &admin_actor, std::slice::from_ref(&creation)).await.unwrap().is_refused());
    assert!(protection.check_push(&writer, &writer_actor, &[creation]).await.unwrap().is_refused());
    for proposed in [update("refs/tags/v1", Some(ID), None), update("refs/tags/v1", Some(ID), Some(ID))] {
        assert_eq!(protection.check_push(&admin, &admin_actor, &[proposed]).await.unwrap().refusals[0].code, "protected_tag_immutable");
    }
    let release = ReleaseInput {
        tag_name: "v1".into(),
        name: "Release".into(),
        notes: String::new(),
        is_draft: true,
        is_prerelease: false,
        target: "main".into(),
    };
    assert!(matches!(fixture.services.releases.create(&writer, &fixture.bob, release.clone()).await, Err(ServiceError::Forbidden)));
    let spaced = ReleaseInput { tag_name: " v2 ".into(), ..release.clone() };
    assert!(matches!(fixture.services.releases.create(&writer, &fixture.bob, spaced.clone()).await, Err(ServiceError::Forbidden)));
    let created = fixture.services.releases.create(&admin, &fixture.olga, release).await.unwrap();
    assert!(matches!(fixture.services.releases.update(&writer, &fixture.bob, created.id, spaced).await, Err(ServiceError::Forbidden)));
    assert!(matches!(fixture.services.releases.delete(&writer, created.id).await, Err(ServiceError::Forbidden)));
    assert!(matches!(
        fixture.services.releases.upload_asset(&writer, &fixture.bob, created.id, "file", "text/plain", b"x".as_slice()).await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(protection.delete_tag_rule(&writer, &fixture.bob, id).await, Err(ServiceError::Forbidden)));
    protection.delete_tag_rule(&admin, &fixture.olga, id).await.unwrap();
    assert_eq!(protection.tag_rules(&admin).await.unwrap(), []);
}
