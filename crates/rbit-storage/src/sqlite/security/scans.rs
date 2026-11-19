//! Ordered scan initiation and one-shot completion; late completion cannot supersede newer starts.
use super::{Scan, SecurityWriteFence};
use crate::StorageError;
use rbit_collaboration::security::{MAX_FINDINGS, MAX_SCANS, Report};
use rbit_core::{RepoId, Timestamp};
impl SecurityWriteFence {
    pub async fn begin_scan(self, repo: RepoId, sha: &str, source: &str, generation: i64) -> Result<i64, StorageError> {
        let mut tx = self.transaction;
        let id: Option<i64> = sqlx::query_scalar("INSERT INTO security_scans (repo_id, scanner, commit_sha, generation, source, created_at) SELECT repo_id, scanner, ?, generation, ?, ? FROM security_settings WHERE repo_id = ? AND generation = ? RETURNING id")
            .bind(sha).bind(source).bind(Timestamp::now().seconds()).bind(repo.0).bind(generation).fetch_optional(&mut *tx).await?;
        let id = id.ok_or(StorageError::Conflict("security configuration changed"))?;
        sqlx::query("DELETE FROM security_scans WHERE repo_id = ? AND channel = 'external' AND id NOT IN (SELECT id FROM security_scans WHERE repo_id = ? AND channel = 'external' ORDER BY id DESC LIMIT ?)")
            .bind(repo.0).bind(repo.0).bind(MAX_SCANS).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    /// `Report` identities are SHA-256 digests before crossing this persistence boundary.
    pub async fn finish_scan(self, repo: RepoId, scan: &Scan, report: &Report) -> Result<(), StorageError> {
        let mut tx = self.transaction;
        let current: Option<i64> = sqlx::query_scalar("SELECT id FROM security_scans WHERE id = ? AND repo_id = ? AND state = 'pending' AND scanner = ? AND generation = CASE WHEN channel = 'external' THEN (SELECT generation FROM security_settings WHERE repo_id = ?) ELSE (SELECT generation FROM security_automation WHERE repo_id = ?) END")
            .bind(scan.id).bind(repo.0).bind(&report.scanner).bind(repo.0).bind(repo.0).fetch_optional(&mut *tx).await?;
        if current.is_none() {
            return Err(StorageError::Conflict("scan is finished or configuration changed"));
        }
        for finding in &report.findings {
            sqlx::query("INSERT INTO security_findings (repo_id, scanner, fingerprint) VALUES (?, ?, ?) ON CONFLICT(repo_id, scanner, fingerprint) DO NOTHING")
                .bind(repo.0).bind(&report.scanner).bind(&finding.identity).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO security_occurrences (scan_id, finding_id, rule, message, path, line, severity) SELECT ?, id, ?, ?, ?, ?, ? FROM security_findings WHERE repo_id = ? AND scanner = ? AND fingerprint = ?")
                .bind(scan.id).bind(&finding.rule).bind(&finding.message).bind(&finding.path).bind(finding.line).bind(finding.severity)
                .bind(repo.0).bind(&report.scanner).bind(&finding.identity).execute(&mut *tx).await?;
        }
        sqlx::query("DELETE FROM security_findings WHERE repo_id = ? AND reason IS NULL AND NOT EXISTS (SELECT 1 FROM security_occurrences WHERE finding_id = security_findings.id)")
            .bind(repo.0).execute(&mut *tx).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM security_findings WHERE repo_id = ?").bind(repo.0).fetch_one(&mut *tx).await?;
        if count > MAX_FINDINGS {
            return Err(StorageError::Conflict("security finding limit reached"));
        }
        sqlx::query("UPDATE security_scans SET state = ?, finished_at = ? WHERE id = ?")
            .bind(if report.complete { "complete" } else { "incomplete" })
            .bind(Timestamp::now().seconds())
            .bind(scan.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM security_scan_jobs WHERE scan_id = ?").bind(scan.id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
