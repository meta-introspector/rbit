//! Domain-separated sealing of OIDC private scalars with the Actions master key.
use rbit_actions::{oidc::Key, secrets::SecretAddress};
use rbit_storage::sqlite::actions_oidc::SealedKey;

use super::SecretsKey;
use crate::ServiceError;

fn address(period: i64) -> SecretAddress<'static> {
    SecretAddress { scope_kind: "oidc_signing", scope_id: 0, secret_id: period }
}

impl SecretsKey {
    pub(in crate::actions) fn seal_oidc(&self, period: i64, key: &Key) -> Result<SealedKey, ServiceError> {
        let (nonce, ciphertext) = self.key.seal(address(period), &key.encode()).map_err(ServiceError::internal)?;
        Ok(SealedKey {
            period,
            key_id: self.id.to_vec(),
            nonce: nonce.to_vec(),
            ciphertext,
            public_jwk: serde_json::to_string(&key.jwk()).map_err(ServiceError::internal)?,
        })
    }

    pub(in crate::actions) fn open_oidc(&self, sealed: &SealedKey) -> Result<Key, ServiceError> {
        let unavailable = || ServiceError::Invalid("OIDC signing key cannot be opened with the Actions secrets key".to_owned());
        let key_id = sealed.key_id.as_slice().try_into().map_err(|_| unavailable())?;
        let nonce = sealed.nonce.as_slice().try_into().map_err(|_| unavailable())?;
        let plain = self.key.open(address(sealed.period), key_id, nonce, &sealed.ciphertext).ok_or_else(unavailable)?;
        let key = Key::decode(&plain).map_err(|_| unavailable())?;
        let public: serde_json::Value = serde_json::from_str(&sealed.public_jwk).map_err(|_| unavailable())?;
        if key.jwk() != public {
            return Err(unavailable());
        }
        Ok(key)
    }
}
