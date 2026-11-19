//! Enrollment policy reads for browser sessions and noninteractive credentials.
use super::pool::Pool;
use crate::StorageError;
use rbit_core::AccountId;

#[derive(Debug, Clone)]
pub struct AuthenticationPolicyStore {
    pool: Pool,
}

impl AuthenticationPolicyStore {
    /// Policy storage uses the database's normal instrumented pool and protocol guard.
    pub fn new(database: &super::Database) -> Self {
        Self { pool: database.store("authentication_policy") }
    }

    /// Missing overrides use the deployment default.
    pub async fn site(&self) -> Result<Option<bool>, StorageError> {
        let value: Option<String> =
            sqlx::query_scalar("SELECT value FROM instance_settings WHERE key = 'require_two_factor'").fetch_optional(&self.pool).await?;
        value.map(|value| value.parse().map_err(|_| StorageError::Corrupt("require_two_factor policy".into()))).transpose()
    }

    pub async fn any_organization_required(&self) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM org_authentication_policies WHERE require_two_factor = 1)")
            .fetch_one(&self.pool)
            .await?)
    }

    pub async fn organization(&self, org: AccountId) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar::<_, bool>("SELECT require_two_factor FROM org_authentication_policies WHERE org_id = ?")
            .bind(org.0)
            .fetch_optional(&self.pool)
            .await?
            .unwrap_or(false))
    }

    pub async fn set_organization(&self, org: AccountId, required: bool) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO org_authentication_policies (org_id, require_two_factor) VALUES (?, ?) ON CONFLICT (org_id) DO UPDATE SET require_two_factor = excluded.require_two_factor")
            .bind(org.0).bind(required).execute(&self.pool).await?;
        Ok(())
    }

    /// Organization policy applies to every account that is a member, including owners.
    pub async fn required_by_membership(&self, user: AccountId) -> Result<bool, StorageError> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM org_members m JOIN org_authentication_policies p ON p.org_id = m.org_id WHERE m.user_id = ? AND p.require_two_factor = 1)")
            .bind(user.0).fetch_one(&self.pool).await?)
    }
}
