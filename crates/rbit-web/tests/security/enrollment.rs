//! Authentication policy regressions.
use super::*;

#[tokio::test]
async fn required_mfa_restricts_existing_sessions_to_enrollment_and_sign_out() {
    let fixture = Fixture::configured(|config| config.security.require_two_factor = true).await;
    let user = fixture
        .services
        .accounts
        .create_user(
            SignUp { username: "mfa-user".into(), email: "mfa@example.com".into(), password: "correct horse battery".into() },
            None,
        )
        .await
        .unwrap();
    let cookie = format!("rbit_session={}", fixture.services.accounts.start_session(&user).await.unwrap().encode());
    for path in ["/", "/admin/security", "/settings/tokens", "/settings/profile"] {
        let response = fixture
            .router
            .clone()
            .oneshot(Request::builder().uri(path).header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
        assert_eq!(response.headers()[header::LOCATION], "/settings/security?enrollment=required");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for path in ["/settings/security", "/settings/security/two-factor", "/settings/account"] {
        let response = fixture
            .router
            .clone()
            .oneshot(Request::builder().uri(path).header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        if path == "/settings/security" {
            let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(String::from_utf8_lossy(&body).contains("requires two-factor authentication"));
        }
    }
    let response = fixture
        .router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/logout")
                .header(header::COOKIE, &cookie)
                .header(header::ORIGIN, "http://localhost:3000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_redirection());
}
