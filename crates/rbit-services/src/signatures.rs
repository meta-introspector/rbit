//! Repository signing trust and bounded, cancellation-safe signature verification.
mod process;
mod public_material;
mod verify;
use crate::{RepoContext, ServiceContext, ServiceError};
use rbit_core::User;
pub use rbit_storage::sqlite::signing_keys::SigningKey;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tokio::sync::Semaphore;
use tokio_util::task::TaskTracker;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureState {
    Unsigned,
    Verified,
    Unknown,
    Invalid,
    Unavailable,
}
impl SignatureState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned => "unsigned",
            Self::Verified => "verified",
            Self::Unknown => "unknown",
            Self::Invalid => "invalid",
            Self::Unavailable => "unavailable",
        }
    }
}
#[derive(Debug, Serialize)]
pub struct SignatureVerification {
    pub object_id: String,
    pub kind: String,
    pub state: SignatureState,
}

#[derive(Debug, Clone)]
pub struct SignatureService {
    ctx: ServiceContext,
    slots: Arc<Semaphore>,
    closed: Arc<Mutex<bool>>,
    tasks: TaskTracker,
}
impl SignatureService {
    pub(crate) fn new(ctx: ServiceContext) -> Self {
        Self { ctx, slots: Arc::new(Semaphore::new(2)), closed: Arc::new(Mutex::new(false)), tasks: TaskTracker::new() }
    }
    fn submit<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
    ) -> Result<tokio::task::JoinHandle<T>, ServiceError> {
        let closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if *closed {
            return Err(ServiceError::Unavailable { retry_after_seconds: 1 });
        }
        let permit = self.slots.clone().try_acquire_owned().map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
        Ok(self.tasks.spawn(async move {
            let _permit = permit;
            work.await
        }))
    }
    pub async fn shutdown(&self) {
        {
            let mut closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *closed = true;
            self.slots.close();
            self.tasks.close();
        }
        self.tasks.wait().await;
    }
    pub async fn verify(&self, ctx: &RepoContext, object_id: String) -> Result<SignatureVerification, ServiceError> {
        if !matches!(object_id.len(), 40 | 64) || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ServiceError::NotFound);
        }
        let requested = object_id.clone();
        let repo = ctx.clone();
        let store = self.ctx.database.signing_keys();
        let work = async move {
            let object = repo.reader.signed_object(object_id).await?;
            let keys = store.list(repo.repo.repo.id).await?;
            let mut state = verify::check(&object, &keys).await;
            // Trust removal while the verifier runs must never publish a stale positive.
            if state == SignatureState::Verified && store.list(repo.repo.repo.id).await? != keys {
                state = SignatureState::Unknown;
            }
            Ok(SignatureVerification { object_id: object.object_id, kind: object.kind.into(), state })
        };
        match self.submit(work) {
            Ok(job) => job.await.map_err(ServiceError::internal)?,
            Err(ServiceError::Unavailable { .. }) => {
                Ok(SignatureVerification { object_id: requested, kind: String::new(), state: SignatureState::Unavailable })
            }
            Err(error) => Err(error),
        }
    }
    pub async fn list_keys(&self, ctx: &RepoContext) -> Result<Vec<SigningKey>, ServiceError> {
        if !ctx.access.can_admin() {
            return Err(ServiceError::NotFound);
        }
        Ok(self.ctx.database.signing_keys().list(ctx.repo.repo.id).await?)
    }
    pub async fn add_key(&self, user: &User, ctx: &RepoContext, name: String, kind: String, material: String) -> Result<(), ServiceError> {
        require_admin(user, ctx)?;
        if name.trim().is_empty() || name.len() > 100 || material.len() > 65536 || name.chars().any(char::is_control) {
            return Err(ServiceError::Invalid("Signing key name or material is invalid or too long.".into()));
        }
        let key = match kind.as_str() {
            "ssh" => {
                let parsed = ssh_key::PublicKey::from_openssh(material.trim())
                    .map_err(|_| ServiceError::Invalid("Invalid SSH public key.".into()))?;
                let encoded = parsed.to_openssh().map_err(ServiceError::internal)?;
                encoded.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
            }
            "openpgp"
                if material.trim().starts_with("-----BEGIN PGP PUBLIC KEY BLOCK-----")
                    && material.trim().ends_with("-----END PGP PUBLIC KEY BLOCK-----")
                    && !material.contains("PRIVATE KEY")
                    && public_material::public_packets(&material) =>
            {
                material.trim().to_owned()
            }
            _ => return Err(ServiceError::Invalid("Supply an SSH or armored OpenPGP public key.".into())),
        };
        if kind == "openpgp" {
            let valid = self
                .submit(verify::valid_pgp_key(key.clone()))?
                .await
                .map_err(ServiceError::internal)?
                .map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
            if !valid {
                return Err(ServiceError::Invalid("Invalid OpenPGP public key.".into()));
            }
        }
        let current_user = self.ctx.database.accounts().find_active_user(user.id()).await?.ok_or(ServiceError::NotFound)?;
        let current = crate::repositories::RepositoryService::new(&self.ctx)
            .open(Some(&current_user), ctx.repo.owner.name.as_str(), ctx.repo.repo.name.as_str())
            .await?;
        require_admin(&current_user, &current)?;
        self.ctx.database.signing_keys().create(current.repo.repo.id, name.trim(), &kind, &key).await?;
        crate::AuditService::new(self.ctx.clone())
            .record(
                crate::audit::AuditEntry::new(crate::audit::AuditAction::SigningKeyAdded).by(user).on(ctx.repo.full_name()).detail(name),
            )
            .await;
        Ok(())
    }
    pub async fn delete_key(&self, user: &User, ctx: &RepoContext, id: i64) -> Result<(), ServiceError> {
        require_admin(user, ctx)?;
        self.ctx.database.signing_keys().delete(ctx.repo.repo.id, id).await?;
        crate::AuditService::new(self.ctx.clone())
            .record(
                crate::audit::AuditEntry::new(crate::audit::AuditAction::SigningKeyRemoved)
                    .by(user)
                    .on(ctx.repo.full_name())
                    .detail(id.to_string()),
            )
            .await;
        Ok(())
    }
}
fn require_admin(user: &User, ctx: &RepoContext) -> Result<(), ServiceError> {
    if ctx.viewer != Some(user.id()) || !ctx.access.can_admin() {
        return Err(ServiceError::NotFound);
    }
    ctx.ensure_writable()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn signatures_cancelled_waiter_retains_child_admission_and_shutdown_drains() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = rbit_core::Config::default();
        config.storage.data_dir = dir.path().into();
        let db = rbit_storage::Database::open(&config.storage.sqlite_path()).await.unwrap();
        let kv = rbit_storage::KvStore::open(&config.storage.kv_path()).unwrap();
        let services = crate::Services::new(Arc::new(config), &db, &kv).await.unwrap();
        let service = services.signatures.clone();
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, resume) = tokio::sync::oneshot::channel();
        let worker = service.clone();
        let waiter = tokio::spawn(async move {
            worker
                .submit(async move {
                    let _ = started.send(());
                    let _ = resume.await;
                    process::execute(tokio::process::Command::new("sh").args(["-c", "exec sleep 0.2"])).await
                })
                .unwrap()
                .await
        });
        observed.await.unwrap();
        waiter.abort();
        let _ = waiter.await;
        assert_eq!(service.slots.available_permits(), 1);
        let (release_second, second_resume) = tokio::sync::oneshot::channel::<()>();
        let second = service.submit(second_resume).unwrap();
        assert_eq!(service.slots.available_permits(), 0);
        assert!(service.submit(async {}).is_err(), "saturated verification refuses without queueing");
        release_second.send(()).unwrap();
        second.await.unwrap().unwrap();
        assert!(tokio::time::timeout(std::time::Duration::from_millis(10), service.shutdown()).await.is_err());
        release.send(()).unwrap();
        service.shutdown().await;
        assert_eq!(service.slots.available_permits(), 2);
        assert!(service.submit(async {}).is_err());
    }
}
