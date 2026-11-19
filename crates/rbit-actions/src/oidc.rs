//! Bounded workflow identity claims and ES256 signing, independent of HTTP and storage.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

/// Tokens outlive neither five minutes nor their admitted task lifetime.
pub const TOKEN_SECONDS: i64 = 300;
/// Rotate every thirty UTC days, publishing the following period's key in advance.
pub const KEY_SECONDS: i64 = 30 * 24 * 60 * 60;
/// Bound token requests before parsing or signing anything.
pub const MAX_AUDIENCE_BYTES: usize = 512;

/// Private signing material. Debug output never contains its scalar.
pub struct Key(SigningKey);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OidcKey([REDACTED])")
    }
}

impl Key {
    pub fn generate() -> Result<Self, String> {
        for _ in 0..16 {
            let mut bytes = Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut()).map_err(|error| error.to_string())?;
            if let Ok(key) = SigningKey::from_slice(bytes.as_ref()) {
                return Ok(Self(key));
            }
        }
        Err("could not generate an OIDC signing scalar".to_owned())
    }

    pub fn decode(encoded: &str) -> Result<Self, String> {
        if encoded.len() != 43 {
            return Err("invalid OIDC key length".to_owned());
        }
        let bytes = Zeroizing::new(URL_SAFE_NO_PAD.decode(encoded).map_err(|_| "invalid OIDC key encoding")?);
        SigningKey::from_slice(&bytes).map(Self).map_err(|_| "invalid OIDC key scalar".to_owned())
    }

    pub fn encode(&self) -> Zeroizing<String> {
        Zeroizing::new(URL_SAFE_NO_PAD.encode(self.0.to_bytes()))
    }

    pub fn kid(&self) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(self.0.verifying_key().to_sec1_point(false).as_bytes()))
    }

    pub fn jwk(&self) -> Value {
        let point = self.0.verifying_key().to_sec1_point(false);
        let bytes = point.as_bytes();
        json!({"kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig", "kid": self.kid(),
            "x": URL_SAFE_NO_PAD.encode(&bytes[1..33]), "y": URL_SAFE_NO_PAD.encode(&bytes[33..65])})
    }

    pub fn sign(&self, claims: &Value) -> Result<String, String> {
        let header = json!({"typ": "JWT", "alg": "ES256", "kid": self.kid()});
        let header = serde_json::to_vec(&header).map_err(|error| error.to_string())?;
        let claims = serde_json::to_vec(claims).map_err(|error| error.to_string())?;
        if claims.len() > 12 * 1024 {
            return Err("OIDC claims exceed their size limit".to_owned());
        }
        let payload = format!("{}.{}", URL_SAFE_NO_PAD.encode(header), URL_SAFE_NO_PAD.encode(claims));
        let signature: Signature = self.0.sign(payload.as_bytes());
        Ok(format!("{payload}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes())))
    }
}

pub fn audience(value: &str) -> Result<&str, &'static str> {
    if value.is_empty() || value.len() > MAX_AUDIENCE_BYTES || value.chars().any(char::is_control) || value.trim() != value {
        return Err("audience must be 1–512 bytes without controls or surrounding whitespace");
    }
    Ok(value)
}

/// Never trust reusable numeric database IDs or reclaimable owner/repository names.
pub fn subject(repository: &str, owner: &str, reference: &str, environment: Option<&str>) -> Result<String, &'static str> {
    let valid_id = |id: &str| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !valid_id(repository) || !valid_id(owner) {
        return Err("OIDC immutable identity is unavailable");
    }
    let (kind, scope) = environment.map_or(("ref", reference), |name| ("environment", name));
    let scope = escape(scope);
    let subject = format!("repo:{repository}:owner:{owner}:{kind}:{scope}");
    if subject.len() > 255 {
        return Err("OIDC subject exceeds 255 bytes; use a shorter ref or environment name");
    }
    Ok(subject)
}

fn escape(value: &str) -> String {
    use std::fmt::Write as _;
    value.bytes().fold(String::new(), |mut result, byte| {
        if byte.is_ascii_alphanumeric() || b"-_.~/".contains(&byte) {
            result.push(char::from(byte));
        } else {
            let _ = write!(result, "%{byte:02X}");
        }
        result
    })
}

#[cfg(test)]
mod tests;
