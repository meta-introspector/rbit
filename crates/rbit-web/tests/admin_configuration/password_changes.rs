//! Required replacement of administrator-provided passwords.
use super::*;

async fn initially_blocked_credentials(fixture: &Fixture, user: &User, initial: &str) -> (String, String) {
    // A credential cannot bypass the browser restriction through Git or the API.
    assert!(matches!(
        fixture
            .services
            .authenticate_git_password(user.name().as_str(), initial.into(), &rbit_identity_models::sessions::ClientInfo::default())
            .await,
        Err(rbit_services::ServiceError::Forbidden)
    ));
    let (_, token) = fixture
        .services
        .tokens
        .create(
            user,
            rbit_services::CreateAccessToken { name: "test credential".into(), scope: rbit_core::TokenScope::Write, expires_in_days: None },
        )
        .await
        .unwrap();
    assert!(fixture.services.tokens.authenticate(&token).await.unwrap().is_none());
    let key = fixture
        .services
        .ssh_keys
        .add_user_key(
            user,
            rbit_services::ssh_keys::AddSshKey {
                title: "test key".into(),
                key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAU5O8WQTG+ptksHlmxF+ieJyp0kwsQ2tQsz0JAuTWme".into(),
            },
        )
        .await
        .unwrap();
    assert!(fixture.services.ssh_keys.find_by_fingerprint(&key.fingerprint).await.unwrap().is_none());

    (token, key.fingerprint)
}

fn session_cookie(response: &Response) -> String {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("rbit_session="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn administrator_created_users_must_replace_their_initial_password_before_using_rbit() {
    Box::pin(require_personal_password(false)).await;
}

#[tokio::test]
async fn administrator_reset_passwords_must_be_replaced_before_using_rbit() {
    Box::pin(require_personal_password(true)).await;
}

async fn require_personal_password(reset_existing: bool) {
    let fixture = Fixture::new().await;
    let initial = "initial administrator password";
    let replacement = "my own replacement password";
    let user = Box::pin(temporary_user(&fixture, initial, reset_existing)).await;
    assert!(fixture.services.accounts.password_change_required(user.id()).await.unwrap());
    assert!(!fixture.services.accounts.password_change_required(fixture.admin.id()).await.unwrap());

    let response = fixture
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("login=newmember&password=initial+administrator+password&next=%2Fnew"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers()[header::LOCATION], "/settings/account");
    let cookie = session_cookie(&response);

    for (method, path) in
        [("GET", "/"), ("GET", "/new"), ("GET", "/settings/tokens"), ("POST", "/settings/account/email"), ("POST", "/new")]
    {
        let response = fixture
            .router
            .clone()
            .oneshot(Request::builder().method(method).uri(path).header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{method} {path}");
        assert_eq!(response.headers()[header::LOCATION], "/settings/account");
    }
    let response = fixture
        .router
        .clone()
        .oneshot(Request::builder().uri("/settings/account").header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let content = html(response).await;
    assert!(content.contains("Update your password"));
    assert!(content.contains("before continuing"));
    assert!(!content.contains("action=\"/settings/account/email\""));

    let (token, fingerprint) = initially_blocked_credentials(&fixture, &user, initial).await;

    for (new, confirmation, succeeds) in
        [(initial, initial, false), (replacement, "does not match", false), (replacement, replacement, true)]
    {
        let body = [("current_password", initial), ("new_password", new), ("confirm_password", confirmation)]
            .into_iter()
            .map(|(key, value)| format!("{key}={}", utf8_percent_encode(value, NON_ALPHANUMERIC)))
            .collect::<Vec<_>>()
            .join("&");
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/settings/account/password")
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), if succeeds { StatusCode::SEE_OTHER } else { StatusCode::UNPROCESSABLE_ENTITY });
        assert_eq!(fixture.services.accounts.password_change_required(user.id()).await.unwrap(), !succeeds);
    }
    let response = fixture
        .router
        .clone()
        .oneshot(Request::builder().uri("/new").header(header::COOKIE, &cookie).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "the initiating session survives and can use rbit");
    assert!(fixture.services.accounts.authenticate("newmember", initial.into()).await.is_err());
    assert!(fixture.services.accounts.authenticate("newmember", replacement.into()).await.is_ok());
    assert!(fixture.services.tokens.authenticate(&token).await.unwrap().is_some());
    assert!(fixture.services.ssh_keys.find_by_fingerprint(&fingerprint).await.unwrap().is_some());
}

async fn temporary_user(fixture: &Fixture, initial: &str, reset_existing: bool) -> User {
    let user = fixture
        .services
        .admin
        .create_user(
            &fixture.admin,
            SignUp { username: "newmember".into(), email: "newmember@example.com".into(), password: initial.into() },
            false,
        )
        .await
        .unwrap();
    if reset_existing {
        // Start with an ordinary account, then exercise the administrator's HTTP reset.
        fixture.services.profiles.change_password(&user, initial.into(), "previous personal password".into()).await.unwrap();
        assert!(!fixture.services.accounts.password_change_required(user.id()).await.unwrap());
        let response = fixture
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/admin/users/{}/password", user.id().0))
                    .header(header::COOKIE, &fixture.cookie)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from("new_password=initial+administrator+password&confirm_password=initial+administrator+password"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
    }
    user
}
