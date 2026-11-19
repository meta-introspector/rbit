//! Compose live workflow admission, sealed rotating keys and bounded cloud identities.

mod keys;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rbit_actions::oidc::{self, Key, TOKEN_SECONDS};
use rbit_auth::TaskTokenSecret;
use rbit_core::{Timestamp, actions::TokenPermissions};
use rbit_storage::sqlite::actions_oidc::TaskIdentity;
use rbit_workflow::spec::JobTemplate;
use serde_json::{Value, json};

use super::ActionsService;
use crate::ServiceError;

/// The issuer is derived only from configured public HTTPS identity, never Host headers.
pub fn issuer(base_url: &str) -> Result<String, ServiceError> {
    let parsed = reqwest::Url::parse(base_url).map_err(|_| ServiceError::Invalid("OIDC needs a public HTTPS URL".to_owned()))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ServiceError::Invalid("OIDC needs a public HTTPS URL without credentials, query or fragment".to_owned()));
    }
    Ok(format!("{}/api/actions/oidc", base_url.trim_end_matches('/')))
}

impl ActionsService {
    pub fn oidc_discovery(&self) -> Result<Value, ServiceError> {
        let issuer = issuer(self.ctx().config.server.base_url())?;
        Ok(json!({"issuer": issuer, "jwks_uri": format!("{issuer}/jwks"),
            "response_types_supported": ["id_token"], "subject_types_supported": ["public"],
            "id_token_signing_alg_values_supported": ["ES256"],
            "claims_supported": ["iss", "sub", "aud", "iat", "nbf", "exp", "jti", "repository_uid", "repository_owner_uid",
                "repository", "repository_id", "repository_owner", "repository_owner_id", "actor_id", "trigger_actor_id", "ref", "sha", "event_name",
                "workflow_ref", "workflow_sha", "run_id", "run_number", "run_attempt", "job_id", "task_id", "environment"]}))
    }

    /// Public verification material includes the previous, current and upcoming period.
    pub async fn oidc_jwks(&self) -> Result<Value, ServiceError> {
        issuer(self.ctx().config.server.base_url())?;
        let keys = self.oidc_keys(Timestamp::now().0).await?;
        let keys: Vec<Value> =
            keys.iter().map(|key| serde_json::from_str(&key.public_jwk).map_err(ServiceError::internal)).collect::<Result<_, _>>()?;
        Ok(json!({"keys": keys}))
    }

    /// Only a running, trusted non-PR task with explicit identity permission may mint.
    /// Already exchanged cloud sessions cannot be revoked by cancelling this job.
    pub async fn oidc_token(&self, secret: &str, audience: &str) -> Result<String, ServiceError> {
        let token = TaskTokenSecret::parse(secret).ok_or(ServiceError::NotFound)?;
        let audience = oidc::audience(audience).map_err(|error| ServiceError::Invalid(error.to_owned()))?;
        let issuer = issuer(self.ctx().config.server.base_url())?;
        // Avoid key work for malformed, revoked or unknown task credentials.
        let grant = self.authenticate_task(secret).await?.ok_or(ServiceError::NotFound)?;
        if !grant.permissions.id_token {
            return Err(ServiceError::Forbidden);
        }
        let now = Timestamp::now().0;
        let key = self.oidc_signing_key(now).await?;
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|error| ServiceError::internal(std::io::Error::other(error.to_string())))?;
        let jti = URL_SAFE_NO_PAD.encode(nonce);
        let now = Timestamp::now().0;
        let heartbeat = i64::try_from(self.config().zombie_timeout_seconds).unwrap_or(i64::MAX);
        let maximum_age = i64::try_from(self.config().endless_timeout_seconds).unwrap_or(i64::MAX);
        self.ctx()
            .database
            .actions_oidc()
            .issue(&token.digest(), now, now.saturating_sub(heartbeat), |identity| {
                sign_identity(&key, &identity, &issuer, audience, now, maximum_age, &jti)
            })
            .await?
            .ok_or(ServiceError::Forbidden)
    }
}

fn sign_identity(
    key: &Key,
    identity: &TaskIdentity,
    issuer: &str,
    audience: &str,
    now: i64,
    maximum_age: i64,
    jti: &str,
) -> Option<String> {
    let permissions: TokenPermissions = serde_json::from_str(&identity.permissions).ok()?;
    let template: JobTemplate = serde_json::from_slice(&identity.payload).ok()?;
    if !permissions.id_token
        || !template.permissions.is_some_and(|permissions| permissions.id_token)
        || template.default_ceiling
        || !environment_allows(identity)
    {
        return None;
    }
    let lifetime = i64::from(template.timeout_minutes).checked_mul(60)?.min(maximum_age);
    let expires = now.checked_add(TOKEN_SECONDS)?.min(identity.started_at.checked_add(lifetime)?);
    if expires <= now || identity.started_at > now {
        return None;
    }
    let subject =
        oidc::subject(&identity.repository_uid, &identity.owner_uid, &identity.reference, identity.environment.as_deref()).ok()?;
    let repository = format!("{}/{}", identity.owner, identity.repository);
    let mut claims = json!({"iss": issuer, "sub": subject, "aud": audience, "iat": now, "nbf": now, "exp": expires, "jti": jti,
        "repository_uid": identity.repository_uid, "repository_owner_uid": identity.owner_uid,
        "repository": repository, "repository_id": identity.repo_id.to_string(), "repository_owner": identity.owner,
        "repository_owner_id": identity.owner_id.to_string(), "ref": identity.reference, "sha": identity.sha,
        "event_name": identity.event, "workflow_ref": format!("{repository}/{}@{}", identity.workflow_path, identity.reference),
        "workflow_sha": identity.sha, "run_id": identity.run_id.to_string(), "run_number": identity.run_number.to_string(),
        "run_attempt": identity.attempt.to_string(), "job_id": identity.job_id.to_string(), "task_id": identity.task_id.to_string()});
    if let Some(environment) = &identity.environment {
        claims["environment"] = json!(environment);
    }
    if let Some(actor) = identity.actor_id {
        claims["actor_id"] = json!(actor.to_string());
    }
    if let Some(actor) = identity.trigger_actor_id {
        claims["trigger_actor_id"] = json!(actor.to_string());
    }
    key.sign(&claims).ok()
}

fn environment_allows(identity: &TaskIdentity) -> bool {
    match identity.environment_policy.as_deref() {
        None | Some("all") => true,
        Some("selected") => {
            let Ok(patterns) = serde_json::from_str::<Vec<String>>(identity.environment_patterns.as_deref().unwrap_or("")) else {
                return false;
            };
            rbit_actions::environments::selected(&patterns, &identity.reference)
        }
        Some("protected") => identity.reference.strip_prefix("refs/heads/").is_some_and(|branch| {
            identity
                .protected_patterns
                .iter()
                .any(|pattern| rbit_core::protection::BranchPattern::parse(pattern).is_ok_and(|pattern| pattern.matches(branch)))
        }),
        _ => false,
    }
}
