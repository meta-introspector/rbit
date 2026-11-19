//! Session requests against the shared real delegation fixture.
use crate::agents::fixture::Fixture;
use axum::{
    body::Body,
    http::{HeaderMap, Request, StatusCode, header},
};
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery";
pub fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={}", percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC)))
        .collect::<Vec<_>>()
        .join("&")
}
pub async fn request(
    fixture: &Fixture,
    cookie: &str,
    path: &str,
    body: Option<&str>,
    origin: &str,
    language: &str,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder()
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::COOKIE, cookie)
        .header(header::ACCEPT_LANGUAGE, language);
    if body.is_some() {
        request = request.method("POST").header(header::ORIGIN, origin).header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let response = fixture
        .app
        .clone()
        .oneshot(request.body(body.map_or_else(Body::empty, |body| Body::from(body.to_owned()))).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let body = axum::body::to_bytes(response.into_body(), 2_000_000).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}
pub async fn cookie(fixture: &Fixture, user: &rbit_core::User) -> String {
    format!("rbit_session={}", fixture.services.accounts.start_session(user).await.unwrap().encode())
}
pub fn agent_form(revision: &str, name: &str, password: &str) -> String {
    form(&[
        ("revision", revision),
        ("slug", "helper"),
        ("name", name),
        ("write_repositories", "agent-team/scoped"),
        ("issues", "true"),
        ("enabled", "true"),
        ("current_password", password),
    ])
}
pub async fn create(fixture: &Fixture, cookie: &str) -> rbit_identity_models::agents::Agent {
    let body = agent_form("0", "Helper", PASSWORD);
    assert_eq!(request(fixture, cookie, "/settings/agents", Some(&body), "http://localhost", "en").await.0, StatusCode::SEE_OTHER);
    fixture.services.agents.list(&fixture.owner).await.unwrap().remove(0)
}
pub fn secret(html: &str) -> String {
    html.split("id=\"agent-secret\"").nth(1).unwrap().split("value=\"").nth(1).unwrap().split('"').next().unwrap().to_owned()
}
