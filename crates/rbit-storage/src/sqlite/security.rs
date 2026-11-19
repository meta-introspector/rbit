//! Transactional bounded scan snapshots, findings and repository security settings.
mod automation;
pub use automation::{AutomationConfiguration, ScanJob};
mod findings;
mod remediations;
mod scans;
use super::pool::{Pool, WriteTransaction};
use crate::StorageError;
use rbit_collaboration::security::Settings;
use rbit_core::RepoId;
pub use remediations::Remediation;
use serde::Serialize;

/// Excludes every database writer until a checked Git publication has finished.
/// Keep this owned guard through the native child wait, including cancellation cleanup.
#[derive(Debug)]
pub struct SecurityWriteFence {
    transaction: WriteTransaction,
}
#[derive(Debug, Clone)]
pub struct SecurityStore {
    pub(super) pool: Pool,
}
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Configuration {
    pub scanner: String,
    pub workflow: String,
    pub require_scan: bool,
    pub minimum_severity: u8,
    pub generation: i64,
}
impl Configuration {
    pub fn settings(&self) -> Settings {
        Settings {
            scanner: self.scanner.clone(),
            workflow: self.workflow.clone(),
            require_scan: self.require_scan,
            minimum_severity: self.minimum_severity,
        }
    }
}
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Scan {
    pub channel: String,
    pub id: i64,
    pub scanner: String,
    pub commit_sha: String,
    pub generation: i64,
    pub source: String,
    pub state: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}
#[derive(Debug, Clone, sqlx::FromRow, Serialize)]
pub struct Occurrence {
    pub id: i64,
    pub fingerprint: String,
    pub rule: String,
    pub message: String,
    pub path: String,
    pub line: u32,
    pub severity: u8,
    pub reason: Option<String>,
    pub note: Option<String>,
    pub dismissed_by: Option<i64>,
    pub dismissed_at: Option<i64>,
}
impl SecurityStore {
    pub async fn publication_fence(&self) -> Result<SecurityWriteFence, StorageError> {
        Ok(SecurityWriteFence { transaction: self.pool.begin_immediate().await? })
    }
    pub(crate) fn new(pool: Pool) -> Self {
        Self { pool }
    }
    pub async fn configuration(&self, repo: RepoId) -> Result<Option<Configuration>, StorageError> {
        Ok(sqlx::query_as("SELECT scanner, workflow, require_scan, minimum_severity, generation FROM security_settings WHERE repo_id = ?")
            .bind(repo.0)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn scans(&self, repo: RepoId) -> Result<Vec<Scan>, StorageError> {
        Ok(sqlx::query_as("SELECT id, scanner, commit_sha, generation, source, state, created_at, finished_at, channel FROM security_scans WHERE repo_id = ? ORDER BY id DESC LIMIT 100").bind(repo.0).fetch_all(&self.pool).await?)
    }
    pub async fn scan(&self, repo: RepoId, id: i64) -> Result<Option<Scan>, StorageError> {
        Ok(sqlx::query_as("SELECT id, scanner, commit_sha, generation, source, state, created_at, finished_at, channel FROM security_scans WHERE repo_id = ? AND id = ?").bind(repo.0).bind(id).fetch_optional(&self.pool).await?)
    }
    pub async fn latest(&self, repo: RepoId, scanner: &str, sha: &str, generation: i64) -> Result<Option<Scan>, StorageError> {
        Ok(sqlx::query_as("SELECT id, scanner, commit_sha, generation, source, state, created_at, finished_at, channel FROM security_scans WHERE repo_id = ? AND channel = 'external' AND scanner = ? AND commit_sha = ? AND generation = ? ORDER BY id DESC LIMIT 1").bind(repo.0).bind(scanner).bind(sha).bind(generation).fetch_optional(&self.pool).await?)
    }
}

impl SecurityWriteFence {
    pub async fn configure(mut self, repo: RepoId, settings: &Settings) -> Result<(), StorageError> {
        sqlx::query("INSERT INTO security_settings (repo_id, scanner, workflow, require_scan, minimum_severity) VALUES (?, ?, ?, ?, ?) ON CONFLICT(repo_id) DO UPDATE SET scanner = excluded.scanner, workflow = excluded.workflow, require_scan = excluded.require_scan, minimum_severity = excluded.minimum_severity, generation = security_settings.generation + 1")
            .bind(repo.0).bind(&settings.scanner).bind(&settings.workflow).bind(settings.require_scan).bind(settings.minimum_severity).execute(&mut *self.transaction).await?;
        self.transaction.commit().await?;
        Ok(())
    }
}
