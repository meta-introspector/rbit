//! Sealed signing-key rotation and live task admission under one SQLite write fence.

use super::pool::Pool;
use crate::StorageError;

#[derive(Debug, Clone)]
pub struct OidcStore {
    pool: Pool,
}

/// Ciphertext is bound to its period by the Actions sealing adapter.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SealedKey {
    pub period: i64,
    pub key_id: Vec<u8>,
    pub nonce: Vec<u8>,
    pub ciphertext: Vec<u8>,
    pub public_jwk: String,
}

/// Current database evidence only; none of these claims comes from the runner request.
#[derive(Debug, sqlx::FromRow)]
pub struct TaskIdentity {
    pub task_id: i64,
    pub job_id: i64,
    pub run_id: i64,
    pub run_number: i64,
    pub attempt: i64,
    pub repo_id: i64,
    pub owner_id: i64,
    pub repository_uid: String,
    pub owner_uid: String,
    pub repository: String,
    pub owner: String,
    pub actor_id: Option<i64>,
    pub trigger_actor_id: Option<i64>,
    pub reference: String,
    pub sha: String,
    pub event: String,
    pub workflow_path: String,
    pub environment: Option<String>,
    pub environment_policy: Option<String>,
    pub environment_patterns: Option<String>,
    #[sqlx(skip)]
    pub protected_patterns: Vec<String>,
    pub payload: Vec<u8>,
    pub permissions: String,
    pub started_at: i64,
}

impl OidcStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub async fn has_keys(&self) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM actions_oidc_keys)").fetch_one(&self.pool).await?)
    }

    pub async fn keys(&self, period: i64) -> Result<Vec<SealedKey>, StorageError> {
        Ok(sqlx::query_as("SELECT period, key_id, nonce, ciphertext, public_jwk FROM actions_oidc_keys WHERE period BETWEEN ?1 - 1 AND ?1 + 1 ORDER BY period LIMIT 3")
            .bind(period).fetch_all(&self.pool).await?)
    }

    /// A competing creator never overwrites already published material. Clock rollback
    /// cannot replace or delete a future key; expired rows are removed at rotation.
    pub async fn install(&self, key: &SealedKey, current_period: i64) -> Result<(), StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        let latest: Option<i64> = sqlx::query_scalar("SELECT MAX(period) FROM actions_oidc_keys").fetch_one(&mut *tx).await?;
        if latest.is_some_and(|latest| latest > current_period + 1) {
            return Err(StorageError::Corrupt("OIDC clock moved behind the published key period".to_owned()));
        }
        sqlx::query("INSERT INTO actions_oidc_keys(period, key_id, nonce, ciphertext, public_jwk) VALUES(?, ?, ?, ?, ?) ON CONFLICT(period) DO NOTHING")
            .bind(key.period).bind(&key.key_id).bind(&key.nonce).bind(&key.ciphertext).bind(&key.public_jwk).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM actions_oidc_keys WHERE period < ? - 1").bind(current_period).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Sign only while live task, runner, repository, ownership and actor grants cannot
    /// change. The synchronous signer does no I/O and returns no token before commit.
    pub async fn issue<T>(
        &self,
        digest: &[u8; 32],
        now: i64,
        heartbeat_cutoff: i64,
        sign: impl FnOnce(TaskIdentity) -> Option<T> + Send,
    ) -> Result<Option<T>, StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        let row: Option<TaskIdentity> = sqlx::query_as(include_str!("actions_oidc/admit.sql"))
            .bind(digest.as_slice())
            .bind(heartbeat_cutoff)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(mut row) = row else { return Ok(None) };
        if row.environment_policy.as_deref() == Some("protected") {
            row.protected_patterns = sqlx::query_scalar("SELECT pattern FROM branch_protections WHERE repo_id = ? LIMIT 1001")
                .bind(row.repo_id)
                .fetch_all(&mut *tx)
                .await?;
            if row.protected_patterns.len() > 1000 {
                return Ok(None);
            }
        }
        let admitted: Option<i64> = sqlx::query_scalar("INSERT INTO actions_oidc_issuance(task_id, minute, count) VALUES(?1, ?2, 1) ON CONFLICT(task_id) DO UPDATE SET minute = excluded.minute, count = CASE WHEN actions_oidc_issuance.minute = excluded.minute THEN actions_oidc_issuance.count + 1 ELSE 1 END WHERE actions_oidc_issuance.minute <> excluded.minute OR actions_oidc_issuance.count < 60 RETURNING task_id")
            .bind(row.task_id).bind(now / 60).fetch_optional(&mut *tx).await?;
        if admitted.is_none() {
            return Ok(None);
        }
        let Some(token) = sign(row) else { return Ok(None) };
        tx.commit().await?;
        Ok(Some(token))
    }
}
