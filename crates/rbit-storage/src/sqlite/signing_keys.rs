//! Repository-scoped public signing trust material, bounded transactionally.
use super::pool::Pool;
use crate::StorageError;
use rbit_core::{RepoId, Timestamp};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SigningKey {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub public_key: String,
}
#[derive(Debug, Clone)]
pub struct SigningKeyStore {
    pool: Pool,
}
impl SigningKeyStore {
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
    pub async fn list(&self, repo: RepoId) -> Result<Vec<SigningKey>, StorageError> {
        Ok(sqlx::query_as("SELECT id, name, kind, public_key FROM repository_signing_keys WHERE repo_id = ? ORDER BY id")
            .bind(repo.0)
            .fetch_all(&self.pool)
            .await?)
    }
    pub async fn create(&self, repo: RepoId, name: &str, kind: &str, key: &str) -> Result<(), StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM repository_signing_keys WHERE repo_id = ?").bind(repo.0).fetch_one(&mut *tx).await?;
        if count >= 16 {
            return Err(StorageError::Conflict("signing key limit"));
        }
        sqlx::query("INSERT INTO repository_signing_keys (repo_id, name, kind, public_key, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(repo.0)
            .bind(name)
            .bind(kind)
            .bind(key)
            .bind(Timestamp::now().seconds())
            .execute(&mut *tx)
            .await
            .map_err(|error| StorageError::from_insert(error, "signing key name"))?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn delete(&self, repo: RepoId, id: i64) -> Result<(), StorageError> {
        sqlx::query("DELETE FROM repository_signing_keys WHERE repo_id = ? AND id = ?").bind(repo.0).bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::{Database, NewRepository, NewUser};
    #[tokio::test]
    async fn signatures_store_caps_concurrent_enrollment_and_scopes_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("rbit.db")).await.unwrap();
        let user = db
            .accounts()
            .create_user(NewUser {
                name: "owner".parse().unwrap(),
                email: "owner@example.com".parse().unwrap(),
                display_name: None,
                password_hash: "hash".into(),
                is_admin: false,
            })
            .await
            .unwrap();
        let mut repos = Vec::new();
        for name in ["one", "two"] {
            repos.push(
                db.repositories()
                    .create(NewRepository {
                        owner_id: user.id(),
                        name: name.parse().unwrap(),
                        description: None,
                        visibility: rbit_core::Visibility::Private,
                        default_branch: "main".into(),
                    })
                    .await
                    .unwrap()
                    .id,
            );
        }
        let store = db.signing_keys();
        for index in 0..15 {
            store.create(repos[0], &format!("key-{index}"), "ssh", "public").await.unwrap();
        }
        let (first, second) =
            tokio::join!(store.create(repos[0], "last-a", "ssh", "public"), store.create(repos[0], "last-b", "ssh", "public"));
        assert_ne!(first.is_ok(), second.is_ok());
        let keys = store.list(repos[0]).await.unwrap();
        assert_eq!(keys.len(), 16);
        store.delete(repos[1], keys[0].id).await.unwrap();
        assert_eq!(store.list(repos[0]).await.unwrap().len(), 16);
        store.delete(repos[0], keys[0].id).await.unwrap();
        assert_eq!(store.list(repos[0]).await.unwrap().len(), 15);
        store.create(repos[0], "replacement", "ssh", "public").await.unwrap();
    }
}
