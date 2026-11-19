//! Database opening and shared key handling after the relevant acceptance boundary.
use super::{Database, connection_capacity, reseal, schema, writer_compatibility};
use crate::{StorageError, sealing::SealingKey};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;
/// Open a legacy database, checking schema acceptance before migration and sealing.
pub(super) async fn open(path: &Path, key_file: &Path) -> Result<Database, StorageError> {
    let options = writer_compatibility::database_options(path)?;

    let pool = SqlitePoolOptions::new().max_connections(connection_capacity()).connect_with(options).await?;
    crate::identity_upgrade::require_legacy_sql(&pool).await?;
    schema::refuse_newer(&pool).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    crate::identity_upgrade::require_legacy_sql(&pool).await?;

    let key = key(&pool, key_file).await?;
    let database = Database {
        pool,
        key,
        #[cfg(test)]
        protocol: super::protocol::SqlProtocol::legacy(),
    };
    database.optimize().await?;
    Ok(database)
}

pub(super) async fn key(pool: &SqlitePool, key_file: &Path) -> Result<SealingKey, StorageError> {
    let key = match SealingKey::load(key_file)? {
        Some(key) => key,
        None if reseal::sample(pool).await?.is_some() => return Err(StorageError::MissingKey(key_file.display().to_string())),
        None => SealingKey::create(key_file)?,
    };
    reseal::check_key(pool, &key, key_file).await?;
    reseal::seal_plaintext(pool, &key).await?;
    Ok(key)
}
#[cfg(test)]
impl Database {
    pub(crate) async fn from_accepted(pool: crate::identity_upgrade::active::ActiveSqlPool, key_file: &Path) -> Result<Self, StorageError> {
        let (pool, protocol) = pool.into_storage_parts();
        let key = key(&pool, key_file).await?;
        let database = Self { pool, key, protocol };
        database.optimize().await?;
        Ok(database)
    }
}
