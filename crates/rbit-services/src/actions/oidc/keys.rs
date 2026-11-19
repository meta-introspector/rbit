//! Seal rotating ES256 scalars with the existing separately backed-up Actions key.
use rbit_actions::oidc::{KEY_SECONDS, Key};
use rbit_storage::sqlite::actions_oidc::SealedKey;

use crate::{ActionsService, ServiceError};

impl ActionsService {
    pub(super) async fn oidc_keys(&self, now: i64) -> Result<Vec<SealedKey>, ServiceError> {
        if now < 0 {
            return Err(ServiceError::Invalid("OIDC clock predates the Unix epoch".to_owned()));
        }
        let _guard = self.inner.oidc_keys.lock().await;
        let period = now / KEY_SECONDS;
        let store = self.ctx().database.actions_oidc();
        let keys = store.keys(period).await?;
        if [period, period + 1].iter().all(|period| keys.iter().any(|key| key.period == *period)) {
            return Ok(keys);
        }
        let sealing = self.secrets_key().await.map_err(|error| ServiceError::Invalid(error.to_string()))?;
        for needed in [period, period + 1] {
            if !keys.iter().any(|key| key.period == needed) {
                let key = Key::generate().map_err(ServiceError::Invalid)?;
                let sealed = sealing.seal_oidc(needed, &key)?;
                store.install(&sealed, period).await?;
            }
        }
        store.keys(period).await.map_err(Into::into)
    }

    pub(super) async fn oidc_signing_key(&self, now: i64) -> Result<Key, ServiceError> {
        let keys = self.oidc_keys(now).await?;
        let key = keys
            .iter()
            .find(|key| key.period == now / KEY_SECONDS)
            .ok_or_else(|| ServiceError::Invalid("OIDC signing key is unavailable".to_owned()))?;
        self.secrets_key().await.map_err(|error| ServiceError::Invalid(error.to_string()))?.open_oidc(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn oidc_rotation_preannounces_keys_keeps_previous_and_fails_without_master() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = rbit_core::Config::default();
        config.storage.data_dir = dir.path().join("data");
        config.actions.enabled = true;
        std::fs::create_dir_all(&config.storage.data_dir).unwrap();
        let config = Arc::new(config);
        let database = rbit_storage::Database::open(&config.storage.sqlite_path()).await.unwrap();
        let kv = rbit_storage::KvStore::open(&config.storage.kv_path()).unwrap();
        let services = crate::Services::new(config.clone(), &database, &kv).await.unwrap();
        let actions = services.actions.as_ref().unwrap();
        let first = actions.oidc_keys(KEY_SECONDS * 20).await.unwrap();
        assert_eq!(first.iter().map(|key| key.period).collect::<Vec<_>>(), [20, 21]);
        let first_id = actions.oidc_signing_key(KEY_SECONDS * 20).await.unwrap().kid();
        let next = actions.oidc_keys(KEY_SECONDS * 21).await.unwrap();
        assert_eq!(next.iter().map(|key| key.period).collect::<Vec<_>>(), [20, 21, 22]);
        assert_eq!(next[0].public_jwk, first[0].public_jwk);
        let active = actions.oidc_signing_key(KEY_SECONDS * 21).await.unwrap();
        assert_eq!(active.jwk().to_string(), first[1].public_jwk);
        assert_ne!(active.kid(), first_id);
        let later = actions.oidc_keys(KEY_SECONDS * 22).await.unwrap();
        assert_eq!(later.iter().map(|key| key.period).collect::<Vec<_>>(), [21, 22, 23]);
        assert!(actions.oidc_keys(KEY_SECONDS * 19).await.is_err());
        let path = config.actions.secrets_key_path(&config.storage);
        tokio::fs::remove_file(&path).await.unwrap();
        let fresh = crate::Services::new(config, &database, &kv).await.unwrap();
        assert!(fresh.actions.as_ref().unwrap().oidc_signing_key(KEY_SECONDS * 22).await.is_err());
        assert!(!path.exists(), "missing master key must never be silently regenerated");
    }
}
