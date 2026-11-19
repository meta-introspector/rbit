//! Compare complete snapshots of the exact head/base pair; absence never means clean.
use super::Gate;
use crate::{ServiceContext, ServiceError};
use rbit_core::RepoId;
pub(crate) async fn evaluate(ctx: &ServiceContext, repo: RepoId, head: &str, base: Option<&str>) -> Result<Gate, ServiceError> {
    let external = external(ctx, repo, head, base).await?;
    if !external.met() {
        return Ok(external);
    }
    let store = ctx.database.security();
    let Some(config) = store.automation(repo).await? else {
        return Ok(external);
    };
    let mut required = false;
    for scanner in config.settings.channels().filter(|name| config.settings.requires(name)) {
        required = true;
        let Some(base) = base else {
            return Ok(Gate::Missing);
        };
        let channel = if scanner == rbit_collaboration::security::automation::SECRETS { "secrets" } else { "npm" };
        let (head_scan, base_scan, dismissed) = tokio::try_join!(
            store.latest_automatic(repo, channel, head, config.generation),
            store.latest_automatic(repo, channel, base, config.generation),
            store.dismissed(repo, scanner)
        )?;
        let (Some(head_scan), Some(base_scan)) = (head_scan, base_scan) else {
            return Ok(Gate::Missing);
        };
        let cutoff = rbit_core::Timestamp::now().seconds() - rbit_collaboration::security::automation::FRESH_SECONDS;
        if head_scan.state != "complete" || base_scan.state != "complete" || head_scan.created_at < cutoff || base_scan.created_at < cutoff
        {
            return Ok(Gate::Incomplete);
        }
        let (heads, bases) =
            tokio::try_join!(store.occurrences(repo, head_scan.id, 0, 1000), store.occurrences(repo, base_scan.id, 0, 1000))?;
        let heads: Vec<_> = heads.into_iter().map(|f| (f.fingerprint, f.severity)).collect();
        let bases: Vec<_> = bases.into_iter().map(|f| f.fingerprint).collect();
        let settings = rbit_collaboration::security::Settings {
            scanner: scanner.into(),
            workflow: String::new(),
            require_scan: true,
            minimum_severity: config.settings.minimum_severity,
        };
        let result = rbit_collaboration::security::evaluate(&settings, Some((true, &heads)), Some((true, &bases)), &dismissed);
        if !result.met() {
            return Ok(result);
        }
    }
    Ok(if required { Gate::Passed } else { external })
}
async fn external(ctx: &ServiceContext, repo: RepoId, head: &str, base: Option<&str>) -> Result<Gate, ServiceError> {
    let store = ctx.database.security();
    let Some(config) = store.configuration(repo).await? else { return Ok(Gate::Disabled) };
    if !config.require_scan {
        return Ok(Gate::Disabled);
    }
    let Some(base) = base else { return Ok(Gate::Missing) };
    let (head, base, dismissed) = tokio::try_join!(
        store.latest(repo, &config.scanner, head, config.generation),
        store.latest(repo, &config.scanner, base, config.generation),
        store.dismissed(repo, &config.scanner),
    )?;
    let (Some(head), Some(base)) = (head, base) else { return Ok(Gate::Missing) };
    let (head_rows, base_rows) = tokio::try_join!(store.occurrences(repo, head.id, 0, 1000), store.occurrences(repo, base.id, 0, 1000))?;
    let head_findings: Vec<_> = head_rows.into_iter().map(|row| (row.fingerprint, row.severity)).collect();
    let base_findings: Vec<_> = base_rows.into_iter().map(|row| row.fingerprint).collect();
    Ok(rbit_collaboration::security::evaluate(
        &config.settings(),
        Some((head.state == "complete", &head_findings)),
        Some((base.state == "complete", &base_findings)),
        &dismissed,
    ))
}
