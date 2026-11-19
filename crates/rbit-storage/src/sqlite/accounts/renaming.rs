//! Atomic account renames preserve old-name reservations and verified-password fences.

use crate::{StorageError, sqlite::pool::Pool};
use rbit_core::{AccountId, AccountName, Timestamp};

pub(super) async fn rename(pool: &Pool, id: AccountId, name: &AccountName) -> Result<(), StorageError> {
    let renamed = sqlx::query(
        "UPDATE accounts SET name = ?1, updated_at = ?2 WHERE id = ?3 AND NOT EXISTS \
        (SELECT 1 FROM account_redirects WHERE name = ?1 AND account_id <> ?3)",
    )
    .bind(name.as_str())
    .bind(Timestamp::now().seconds())
    .bind(id.0)
    .execute(pool)
    .await
    .map_err(|error| StorageError::from_insert(error, "username"))?
    .rows_affected()
        != 0;
    if !renamed && reserved_for_another(pool, id, name).await? {
        return Err(StorageError::Conflict("username"));
    }
    Ok(())
}

pub(super) async fn rename_if_password(pool: &Pool, id: AccountId, name: &AccountName, expected: &str) -> Result<bool, StorageError> {
    let renamed = sqlx::query(
        "UPDATE accounts SET name = ?1, updated_at = ?2 WHERE id = ?3 AND EXISTS \
        (SELECT 1 FROM user_logins WHERE account_id = ?3 AND password_hash = ?4 AND suspended_at IS NULL) \
        AND NOT EXISTS (SELECT 1 FROM account_redirects WHERE name = ?1 AND account_id <> ?3)",
    )
    .bind(name.as_str())
    .bind(Timestamp::now().seconds())
    .bind(id.0)
    .bind(expected)
    .execute(pool)
    .await
    .map_err(|error| StorageError::from_insert(error, "username"))?
    .rows_affected()
        != 0;
    if !renamed && reserved_for_another(pool, id, name).await? {
        return Err(StorageError::Conflict("username"));
    }
    Ok(renamed)
}

/// Whether `name` is a former name of an account other than `id`.
async fn reserved_for_another(pool: &Pool, id: AccountId, name: &AccountName) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM account_redirects WHERE name = ? AND account_id <> ?)")
        .bind(name.as_str())
        .bind(id.0)
        .fetch_one(pool)
        .await?)
}
