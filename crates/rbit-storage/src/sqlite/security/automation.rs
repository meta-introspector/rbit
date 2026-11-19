//! Durable bounded automatic scan admission, expiring claims and current configuration.
use super::{Scan, SecurityStore, SecurityWriteFence};
use crate::StorageError;
use rbit_collaboration::security::automation::{AutomationSettings, DEPENDENCIES, SECRETS};
use rbit_core::{AccountId, RepoId, Timestamp};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AutomationConfiguration {
    pub settings: AutomationSettings,
    pub generation: i64,
    pub enabled_by: Option<AccountId>,
}
#[derive(Debug, sqlx::FromRow)]
pub struct ScanJob {
    pub scan_id: i64,
    pub repo_id: i64,
    pub attempts: i64,
}
impl SecurityStore {
    pub async fn automation(&self, repo: RepoId) -> Result<Option<AutomationConfiguration>, StorageError> {
        let row: Option<(String, i64, Option<i64>)> =
            sqlx::query_as("SELECT settings, generation, enabled_by FROM security_automation WHERE repo_id = ?")
                .bind(repo.0)
                .fetch_optional(&self.pool)
                .await?;
        row.map(|(settings, generation, actor)| {
            Ok(AutomationConfiguration {
                settings: serde_json::from_str(&settings)
                    .map_err(|_| StorageError::Conflict("invalid security automation configuration"))?,
                generation,
                enabled_by: actor.map(AccountId),
            })
        })
        .transpose()
    }
    pub async fn fingerprint_key(&self, repo: RepoId) -> Result<Vec<u8>, StorageError> {
        Ok(sqlx::query_scalar("SELECT fingerprint_key FROM security_automation WHERE repo_id = ?")
            .bind(repo.0)
            .fetch_one(&self.pool)
            .await?)
    }
    pub async fn automatic_repositories(&self, after: i64) -> Result<Vec<RepoId>, StorageError> {
        let ids: Vec<i64> = sqlx::query_scalar("SELECT a.repo_id FROM security_automation a JOIN repositories r ON r.id = a.repo_id WHERE a.repo_id > ? AND (json_extract(a.settings, '$.secrets') = 1 OR json_extract(a.settings, '$.dependencies') = 1) ORDER BY a.repo_id LIMIT 100").bind(after).fetch_all(&self.pool).await?;
        Ok(ids.into_iter().map(RepoId).collect())
    }
    pub async fn latest_automatic(&self, repo: RepoId, channel: &str, sha: &str, generation: i64) -> Result<Option<Scan>, StorageError> {
        Ok(sqlx::query_as("SELECT id, scanner, commit_sha, generation, source, state, created_at, finished_at, channel FROM security_scans WHERE repo_id = ? AND channel = ? AND commit_sha = ? AND generation = ? ORDER BY id DESC LIMIT 1").bind(repo.0).bind(channel).bind(sha).bind(generation).fetch_optional(&self.pool).await?)
    }
    pub async fn claim_scan(&self, now: Timestamp) -> Result<Option<ScanJob>, StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        let row: Option<ScanJob> = sqlx::query_as("UPDATE security_scan_jobs SET claimed_until = ?, attempts = attempts + 1 WHERE scan_id = (SELECT j.scan_id FROM security_scan_jobs j JOIN security_scans s ON s.id = j.scan_id WHERE j.claimed_until <= ? AND s.state = 'pending' ORDER BY j.scan_id LIMIT 1) RETURNING scan_id, (SELECT repo_id FROM security_scans WHERE id = scan_id) AS repo_id, attempts").bind(now.seconds() + 300).bind(now.seconds()).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        Ok(row)
    }
    /// Complete an interrupted/invalidated attempt without publishing any clean evidence.
    pub async fn abandon_scan(&self, id: i64, now: Timestamp) -> Result<(), StorageError> {
        let mut tx = self.pool.begin_immediate().await?;
        sqlx::query(
            "UPDATE security_scans SET state = 'incomplete', finished_at = ? WHERE id = ? AND state = 'pending' AND channel <> 'external'",
        )
        .bind(now.seconds())
        .bind(id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM security_scan_jobs WHERE scan_id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
impl SecurityWriteFence {
    pub async fn configure_automation(mut self, repo: RepoId, actor: AccountId, settings: &AutomationSettings) -> Result<(), StorageError> {
        let json = serde_json::to_string(settings).map_err(|_| StorageError::Conflict("invalid security automation configuration"))?;
        sqlx::query("INSERT INTO security_automation(repo_id, settings, enabled_by) VALUES (?, ?, ?) ON CONFLICT(repo_id) DO UPDATE SET settings = excluded.settings, enabled_by = excluded.enabled_by, generation = security_automation.generation + 1").bind(repo.0).bind(json).bind(actor.0).execute(&mut *self.transaction).await?;
        self.transaction.commit().await?;
        Ok(())
    }
    pub async fn enqueue_scan(
        mut self,
        repo: RepoId,
        scanner: &str,
        sha: &str,
        generation: i64,
        now: Timestamp,
    ) -> Result<i64, StorageError> {
        let channel = match scanner {
            SECRETS => "secrets",
            DEPENDENCIES => "npm",
            _ => return Err(StorageError::Conflict("unknown automatic scanner")),
        };
        let existing: Option<i64> = sqlx::query_scalar("SELECT s.id FROM security_scans s JOIN security_scan_jobs j ON j.scan_id = s.id WHERE s.repo_id = ? AND s.channel = ? AND s.commit_sha = ? AND s.generation = ? AND s.state = 'pending'").bind(repo.0).bind(channel).bind(sha).bind(generation).fetch_optional(&mut *self.transaction).await?;
        if let Some(id) = existing {
            self.transaction.commit().await?;
            return Ok(id);
        }
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT count(*), coalesce(sum(s.repo_id = ?), 0) FROM security_scan_jobs j JOIN security_scans s ON s.id = j.scan_id",
        )
        .bind(repo.0)
        .fetch_one(&mut *self.transaction)
        .await?;
        if counts.0 >= 4096 || counts.1 >= 256 {
            return Err(StorageError::Conflict("automatic scan queue is full"));
        }
        let field = if channel == "secrets" { "$.secrets" } else { "$.dependencies" };
        let id: Option<i64> = sqlx::query_scalar("INSERT INTO security_scans(repo_id, scanner, commit_sha, generation, source, created_at, channel) SELECT repo_id, ?, ?, generation, 'rbit:automatic', ?, ? FROM security_automation WHERE repo_id = ? AND generation = ? AND json_extract(settings, ?) = 1 RETURNING id").bind(scanner).bind(sha).bind(now.seconds()).bind(channel).bind(repo.0).bind(generation).bind(field).fetch_optional(&mut *self.transaction).await?;
        let id = id.ok_or(StorageError::Conflict("automatic scan configuration changed"))?;
        sqlx::query("INSERT INTO security_scan_jobs(scan_id) VALUES (?)").bind(id).execute(&mut *self.transaction).await?;
        sqlx::query("DELETE FROM security_scans WHERE repo_id = ? AND channel = ? AND id NOT IN (SELECT id FROM security_scans WHERE repo_id = ? AND channel = ? ORDER BY id DESC LIMIT 100) AND id NOT IN (SELECT scan_id FROM security_scan_jobs)").bind(repo.0).bind(channel).bind(repo.0).bind(channel).execute(&mut *self.transaction).await?;
        self.transaction.commit().await?;
        Ok(id)
    }
}
