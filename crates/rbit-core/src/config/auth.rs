//! `[auth]`: signing in with external identity providers (OIDC, GitHub).
//!
//! ```toml
//! [auth]
//! # Upper bound for each request to a provider (discovery, keys, token exchange, user API).
//! timeout_seconds = 10
//! # Providers on loopback or private addresses (an on-premises IdP, tests) are refused
//! # unless this is set; it also allows plain-http issuers.
//! allow_private_addresses = false
//!
//! [[auth.providers]]
//! kind = "oidc"                    # or "github"
//! id = "corp"                      # used in URLs: {public_url}/login/corp/callback
//! name = "Corp SSO"                # the button says "Sign in with Corp SSO"
//! issuer = "https://sso.example.com/realms/main"   # discovery at /.well-known/openid-configuration
//! client_id = "rbit"
//! client_secret = "…"              # or RBIT_AUTH__PROVIDERS__0__CLIENT_SECRET
//! scopes = ["openid", "email", "profile"]
//! allow_registration = true        # false: never; "always": even when sign-up is closed
//! trust_email = false              # link by verified email; sign-ups take its verified address
//! trust_mfa = false                # skip rbit's own 2FA when the IdP reports MFA (`amr`)
//! ```
//!
//! For `kind = "github"`, `issuer` is not used; `base_url` (default `https://github.com`)
//! points at a GitHub Enterprise Server instead, whose API is `{base_url}/api/v3`.
//!
//! Secrets from the environment: figment cannot index into an array of tables, so the
//! server loader keeps `RBIT_AUTH__PROVIDERS__<N>__CLIENT_ID` / `…__CLIENT_SECRET` (where
//! `<N>` is the provider's zero-based position or its `id`, upper-cased with `-` as `_`)
//! away from figment and applies them with [`AuthConfig::apply_provider_env`].
//!
//! Milestone 3 workstream O (external sign-in) owns this file.

use std::{collections::HashSet, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Longest provider id.
pub const MAX_PROVIDER_ID_LEN: usize = 32;
/// Provider ids that would collide with fixed routes under `/login/`.
const RESERVED_PROVIDER_IDS: [&str; 2] = ["two-factor", "callback"];
/// Environment keys (after `RBIT_`) that address a provider.
const PROVIDER_ENV_PREFIX: &str = "AUTH__PROVIDERS__";
/// Provider fields that may come from the environment.
const ENV_FIELDS: [&str; 2] = ["CLIENT_ID", "CLIENT_SECRET"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Upper bound for one request to a provider.
    pub timeout_seconds: u64,
    /// Allow providers whose addresses are loopback, private, link-local or unique-local,
    /// and issuers or base URLs over plain `http`.
    pub allow_private_addresses: bool,
    pub providers: Vec<ProviderConfig>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self { timeout_seconds: 10, allow_private_addresses: false, providers: Vec::new() }
    }
}

/// Which protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Any OIDC provider (Keycloak, Authentik, Google, Entra ID, …).
    #[default]
    Oidc,
    /// GitHub's OAuth web flow (not OIDC), `github.com` or Enterprise Server.
    Github,
    /// Discord `OAuth2` authorization-code flow.
    Discord,
}

impl ProviderKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Oidc => "oidc",
            Self::Github => "github",
            Self::Discord => "discord",
        }
    }
}

/// Whether a first sign-in with an unknown identity may create an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProviderRegistration {
    /// Only while the site's registration setting is open (`true` in the file).
    #[default]
    FollowSite,
    /// Never: identities must be linked to existing accounts (`false`).
    Never,
    /// Even when the site's sign-up form is closed (`"always"`): the provider decides
    /// who gets in.
    Always,
}

impl Serialize for ProviderRegistration {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::FollowSite => serializer.serialize_bool(true),
            Self::Never => serializer.serialize_bool(false),
            Self::Always => serializer.serialize_str("always"),
        }
    }
}

impl<'de> Deserialize<'de> for ProviderRegistration {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Text(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Flag(true) => Ok(Self::FollowSite),
            Raw::Flag(false) => Ok(Self::Never),
            // Environment variables arrive as text.
            Raw::Text(text) => match text.trim().to_ascii_lowercase().as_str() {
                "always" => Ok(Self::Always),
                "true" => Ok(Self::FollowSite),
                "false" => Ok(Self::Never),
                other => Err(serde::de::Error::custom(format!("allow_registration must be true, false or \"always\", not {other:?}"))),
            },
        }
    }
}

/// One `[[auth.providers]]` entry.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProviderConfig {
    pub kind: ProviderKind,
    /// Keep configuration and linked identities while preventing sign-in and linking.
    pub disabled: bool,
    /// Lower-case slug used in URLs (`/login/{id}`); never change it once people have
    /// linked identities, they are stored under it.
    pub id: String,
    /// Shown on buttons and in settings.
    pub name: String,
    /// OIDC issuer URL (`kind = "oidc"` only).
    pub issuer: Option<String>,
    /// GitHub Enterprise Server URL (`kind = "github"` only); default `https://github.com`.
    pub base_url: Option<String>,
    pub client_id: String,
    /// Never logged or shown (see the `Debug` implementation).
    pub client_secret: String,
    /// Requested scopes; the defaults are `openid email profile` (OIDC) and
    /// `read:user user:email` (GitHub). OIDC always gets `openid`.
    pub scopes: Option<Vec<String>>,
    pub allow_registration: ProviderRegistration,
    /// Link a first sign-in to the one account whose verified email equals the provider's verified email, and take
    /// that address as verified when it registers an account. Only for providers that control their users' addresses.
    pub trust_email: bool,
    /// Skip the account's own two-factor step when the ID token's `amr` claim says the
    /// provider performed multi-factor authentication (`"mfa"`). OIDC only.
    pub trust_mfa: bool,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("kind", &self.kind)
            .field("disabled", &self.disabled)
            .field("id", &self.id)
            .field("name", &self.name)
            .field("issuer", &self.issuer)
            .field("base_url", &self.base_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &if self.client_secret.is_empty() { "" } else { "<redacted>" })
            .field("scopes", &self.scopes)
            .field("allow_registration", &self.allow_registration)
            .field("trust_email", &self.trust_email)
            .field("trust_mfa", &self.trust_mfa)
            .finish()
    }
}

impl ProviderConfig {
    /// Safe setup defaults for the administrator's provider picker.
    pub fn preset(preset: &str) -> Option<Self> {
        let (kind, id, name, issuer) = match preset {
            "google" => (ProviderKind::Oidc, "google", "Google", Some("https://accounts.google.com")),
            "discord" => (ProviderKind::Discord, "discord", "Discord", None),
            "github" => (ProviderKind::Github, "github", "GitHub", None),
            "oidc" => (ProviderKind::Oidc, "sso", "Organization SSO", Some("https://sso.example.com")),
            _ => return None,
        };
        Some(Self { kind, id: id.into(), name: name.into(), issuer: issuer.map(str::to_owned), disabled: true, ..Self::default() })
    }

    /// The scopes to request.
    pub fn effective_scopes(&self) -> Vec<String> {
        let mut scopes = self.scopes.clone().unwrap_or_else(|| {
            let defaults: &[&str] = match self.kind {
                ProviderKind::Oidc => &["openid", "email", "profile"],
                ProviderKind::Github => &["read:user", "user:email"],
                ProviderKind::Discord => &["identify", "email"],
            };
            defaults.iter().map(|scope| (*scope).to_owned()).collect()
        });
        if self.kind == ProviderKind::Oidc && !scopes.iter().any(|scope| scope == "openid") {
            scopes.insert(0, "openid".to_owned());
        }
        scopes
    }

    /// The issuer without a trailing slash.
    pub fn issuer_url(&self) -> Option<&str> {
        self.issuer.as_deref().map(|issuer| issuer.trim().trim_end_matches('/'))
    }

    /// GitHub's web base (`https://github.com` or the Enterprise Server URL).
    pub fn github_base_url(&self) -> &str {
        self.base_url.as_deref().map_or("https://github.com", |url| url.trim().trim_end_matches('/'))
    }

    /// GitHub's REST API base: `https://api.github.com`, or `{base_url}/api/v3`.
    pub fn github_api_url(&self) -> String {
        match self.base_url.as_deref() {
            None => "https://api.github.com".to_owned(),
            Some(_) => format!("{}/api/v3", self.github_base_url()),
        }
    }
}

impl AuthConfig {
    /// Whether `key` (a figment environment key without the prefix, e.g.
    /// `auth__providers__0__client_secret`) addresses a provider field. The loader hides
    /// these from figment (an indexed key would replace the whole providers array) and
    /// applies them with [`Self::apply_provider_env`].
    pub fn is_provider_env_key(key: &str) -> bool {
        key.len() > PROVIDER_ENV_PREFIX.len()
            && key.get(..PROVIDER_ENV_PREFIX.len()).is_some_and(|start| start.eq_ignore_ascii_case(PROVIDER_ENV_PREFIX))
    }

    /// Applies `RBIT_AUTH__PROVIDERS__<N>__<FIELD>` variables, where `vars` yields the
    /// keys without the `RBIT_` prefix. `<N>` is a zero-based position or a provider id
    /// (upper-cased, `-` written as `_`); `<FIELD>` is `CLIENT_ID` or `CLIENT_SECRET`.
    pub fn apply_provider_env<I, K, V>(&mut self, vars: I) -> Result<(), String>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        for (key, value) in vars {
            let key = key.as_ref();
            if !Self::is_provider_env_key(key) {
                continue;
            }
            let rest = &key[PROVIDER_ENV_PREFIX.len()..];
            let Some((which, field)) = rest.split_once("__") else {
                return Err(format!("RBIT_{key}: expected RBIT_AUTH__PROVIDERS__<N>__CLIENT_SECRET"));
            };
            let index = match which.parse::<usize>() {
                Ok(index) => Some(index),
                Err(_) => self.providers.iter().position(|provider| provider.id.replace('-', "_").eq_ignore_ascii_case(which)),
            };
            let Some(provider) = index.and_then(|index| self.providers.get_mut(index)) else {
                return Err(format!("RBIT_{key}: there is no [[auth.providers]] entry {which} (configure providers in the file)"));
            };
            let field = field.to_ascii_uppercase();
            match field.as_str() {
                "CLIENT_ID" => provider.client_id = value.into(),
                "CLIENT_SECRET" => provider.client_secret = value.into(),
                _ => return Err(format!("RBIT_{key}: only {} can be set from the environment", ENV_FIELDS.join(" and "))),
            }
        }
        Ok(())
    }

    /// Checks the providers, returning every problem found (empty when valid).
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut ids = HashSet::new();
        for (index, provider) in self.providers.iter().enumerate() {
            let label =
                if provider.id.is_empty() { format!("auth.providers[{index}]") } else { format!("auth provider {:?}", provider.id) };
            if let Err(problem) = validate_id(&provider.id) {
                problems.push(format!("{label}: {problem}"));
            } else if !ids.insert(provider.id.as_str()) {
                problems.push(format!("{label}: the id is used by another provider"));
            }
            if provider.name.trim().is_empty() {
                problems.push(format!("{label}: `name` is required (it is shown on the sign-in button)"));
            }
            if !provider.disabled && provider.client_id.trim().is_empty() {
                problems.push(format!("{label}: `client_id` is required"));
            }
            if !provider.disabled && provider.client_secret.is_empty() {
                problems.push(format!("{label}: `client_secret` is required (or set RBIT_AUTH__PROVIDERS__{index}__CLIENT_SECRET)"));
            }
            if provider
                .scopes
                .as_ref()
                .is_some_and(|scopes| scopes.iter().any(|scope| scope.is_empty() || scope.contains(char::is_whitespace)))
            {
                problems.push(format!("{label}: scopes must be single words"));
            }
            match provider.kind {
                ProviderKind::Oidc => {
                    match provider.issuer_url() {
                        None | Some("") => problems.push(format!("{label}: `issuer` is required for kind = \"oidc\"")),
                        Some(issuer) => {
                            if let Err(problem) = self.validate_url(issuer) {
                                problems.push(format!("{label}: issuer {problem}"));
                            }
                        }
                    }
                    if provider.base_url.is_some() {
                        problems.push(format!("{label}: `base_url` is for kind = \"github\"; OIDC providers use `issuer`"));
                    }
                }
                ProviderKind::Discord => {
                    if provider.issuer.is_some() || provider.base_url.is_some() || provider.trust_mfa {
                        problems.push(format!("{label}: Discord uses fixed endpoints and cannot attest MFA"));
                    }
                }
                ProviderKind::Github => {
                    if provider.issuer.is_some() {
                        problems.push(format!("{label}: `issuer` is for kind = \"oidc\"; GitHub Enterprise uses `base_url`"));
                    }
                    if provider.trust_mfa {
                        problems.push(format!("{label}: `trust_mfa` needs an OIDC provider (GitHub does not report how users signed in)"));
                    }
                    if provider.base_url.is_some()
                        && let Err(problem) = self.validate_url(provider.github_base_url())
                    {
                        problems.push(format!("{label}: base_url {problem}"));
                    }
                }
            }
        }
        problems
    }

    /// Fails with every problem joined, for start-up.
    pub fn validate(&self) -> Result<(), String> {
        let problems = self.problems();
        if problems.is_empty() { Ok(()) } else { Err(problems.join("; ")) }
    }

    fn validate_url(&self, url: &str) -> Result<(), String> {
        let rest = if let Some(rest) = url.strip_prefix("https://") {
            rest
        } else if let Some(rest) = url.strip_prefix("http://") {
            if !self.allow_private_addresses {
                return Err("must use https (plain http needs [auth] allow_private_addresses = true)".to_owned());
            }
            rest
        } else {
            return Err("must be an absolute https URL".to_owned());
        };
        if rest.is_empty() || rest.starts_with('/') || url.chars().any(|c| c.is_whitespace() || c.is_control()) || url.contains(['?', '#'])
        {
            return Err("must be an absolute URL without query or fragment".to_owned());
        }
        Ok(())
    }
}

/// `[a-z0-9-]`, starting with a letter or digit, at most [`MAX_PROVIDER_ID_LEN`].
fn validate_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("`id` is required (it is used in URLs, e.g. \"github\")");
    }
    if id.len() > MAX_PROVIDER_ID_LEN {
        return Err("`id` can be at most 32 characters");
    }
    if !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') || id.starts_with('-') {
        return Err("`id` may only contain lower-case letters, digits and '-' (and not start with '-')");
    }
    if RESERVED_PROVIDER_IDS.contains(&id) {
        return Err("this `id` is reserved");
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod preset_tests;
