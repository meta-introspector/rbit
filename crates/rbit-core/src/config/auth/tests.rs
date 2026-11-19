//! Provider configuration validation regressions.

use super::*;

fn oidc(id: &str) -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::Oidc,
        disabled: false,
        id: id.to_owned(),
        name: "Corp".to_owned(),
        issuer: Some("https://sso.example.com/realms/main/".to_owned()),
        client_id: "rbit".to_owned(),
        client_secret: "s3cret".to_owned(),
        ..ProviderConfig::default()
    }
}

fn github() -> ProviderConfig {
    ProviderConfig {
        kind: ProviderKind::Github,
        disabled: false,
        id: "github".to_owned(),
        name: "GitHub".to_owned(),
        client_id: "Iv1.abc".to_owned(),
        client_secret: "s3cret".to_owned(),
        ..ProviderConfig::default()
    }
}

fn config(providers: Vec<ProviderConfig>) -> AuthConfig {
    AuthConfig { providers, ..AuthConfig::default() }
}

#[test]
fn parses_providers_and_registration_modes() {
    let config: AuthConfig = serde_json::from_str(
            r#"{"providers": [
                {"kind": "oidc", "id": "corp", "name": "Corp", "issuer": "https://sso.example.com", "client_id": "a", "client_secret": "b", "trust_email": true},
                {"kind": "github", "id": "github", "name": "GitHub", "client_id": "c", "client_secret": "d", "allow_registration": "always"},
                {"kind": "github", "id": "ghe", "name": "GHE", "client_id": "c", "client_secret": "d", "allow_registration": false, "base_url": "https://ghe.example.com/"}
            ]}"#,
        )
        .unwrap();
    assert_eq!(config.timeout_seconds, 10);
    assert!(!config.allow_private_addresses);
    assert_eq!(config.providers[0].allow_registration, ProviderRegistration::FollowSite);
    assert!(config.providers[0].trust_email);
    assert_eq!(config.providers[1].allow_registration, ProviderRegistration::Always);
    assert_eq!(config.providers[2].allow_registration, ProviderRegistration::Never);
    assert_eq!(config.providers[1].github_api_url(), "https://api.github.com");
    assert_eq!(config.providers[2].github_api_url(), "https://ghe.example.com/api/v3");
    assert_eq!(config.providers[2].github_base_url(), "https://ghe.example.com");
    assert!(config.validate().is_ok(), "{:?}", config.problems());
    let invalid: Result<AuthConfig, _> = serde_json::from_str(r#"{"providers": [{"allow_registration": "sometimes"}]}"#);
    assert!(invalid.is_err());
}

#[test]
fn scopes_default_per_kind_and_oidc_always_asks_for_openid() {
    assert_eq!(oidc("corp").effective_scopes(), ["openid", "email", "profile"]);
    assert_eq!(github().effective_scopes(), ["read:user", "user:email"]);
    let custom = ProviderConfig { scopes: Some(vec!["email".to_owned()]), ..oidc("corp") };
    assert_eq!(custom.effective_scopes(), ["openid", "email"]);
    assert_eq!(oidc("corp").issuer_url(), Some("https://sso.example.com/realms/main"));
}

#[test]
fn validation_reports_every_problem() {
    assert!(config(vec![oidc("corp"), github()]).validate().is_ok());
    let problems = config(vec![
        oidc("corp"),
        oidc("corp"),
        ProviderConfig { issuer: None, ..oidc("no-issuer") },
        ProviderConfig { client_secret: String::new(), ..oidc("Bad_Id") },
        ProviderConfig { issuer: Some("http://sso.internal".to_owned()), ..oidc("plain") },
        ProviderConfig { issuer: Some("https://sso.example.com/?x=1".to_owned()), ..oidc("query") },
        ProviderConfig { trust_mfa: true, issuer: Some("https://x.example".to_owned()), ..github() },
        oidc("two-factor"),
    ])
    .problems();
    let all = problems.join("\n");
    assert!(all.contains("\"corp\": the id is used by another provider"), "{all}");
    assert!(all.contains("`issuer` is required"), "{all}");
    assert!(all.contains("lower-case letters"), "{all}");
    assert!(all.contains("must use https"), "{all}");
    assert!(all.contains("without query"), "{all}");
    assert!(all.contains("`issuer` is for kind = \"oidc\""), "{all}");
    assert!(all.contains("`trust_mfa` needs an OIDC provider"), "{all}");
    assert!(all.contains("reserved"), "{all}");

    let mut local = config(vec![ProviderConfig { issuer: Some("http://127.0.0.1:8080".to_owned()), ..oidc("local") }]);
    assert!(local.validate().is_err());
    local.allow_private_addresses = true;
    assert!(local.validate().is_ok());
    let unnamed = config(vec![ProviderConfig { id: String::new(), name: String::new(), client_id: String::new(), ..oidc("x") }]);
    let all = unnamed.problems().join("\n");
    assert!(all.contains("auth.providers[0]: `id` is required"), "{all}");
    assert!(all.contains("`name` is required") && all.contains("`client_id` is required"), "{all}");
}

#[test]
fn secrets_come_from_the_environment_by_position_or_id() {
    let mut config = config(vec![ProviderConfig { client_secret: String::new(), ..oidc("corp-sso") }, github()]);
    assert!(AuthConfig::is_provider_env_key("auth__providers__0__client_secret"));
    assert!(AuthConfig::is_provider_env_key("AUTH__PROVIDERS__CORP_SSO__CLIENT_SECRET"));
    assert!(!AuthConfig::is_provider_env_key("AUTH__TIMEOUT_SECONDS"));
    config
        .apply_provider_env([
            ("SERVER__PUBLIC_URL", "ignored"),
            ("AUTH__PROVIDERS__CORP_SSO__CLIENT_SECRET", "from-env"),
            ("auth__providers__1__client_id", "other-id"),
        ])
        .unwrap();
    assert_eq!(config.providers[0].client_secret, "from-env");
    assert_eq!(config.providers[1].client_id, "other-id");
    assert!(config.validate().is_ok());
    assert!(config.apply_provider_env([("AUTH__PROVIDERS__7__CLIENT_SECRET", "x")]).unwrap_err().contains("no [[auth.providers]] entry 7"));
    assert!(config.apply_provider_env([("AUTH__PROVIDERS__0__ISSUER", "x")]).unwrap_err().contains("only CLIENT_ID and CLIENT_SECRET"));
}

#[test]
fn debug_output_never_shows_the_secret() {
    let debug = format!("{:?}", oidc("corp"));
    assert!(!debug.contains("s3cret"));
    assert!(debug.contains("<redacted>"));
}
