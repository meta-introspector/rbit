//! Native scanners read immutable data only; raw secret values never leave the Git visitor.
use super::{SecurityService, advisories::AdvisoryClient};
use crate::{RepoContext, ServiceError};
use hmac::{Hmac, KeyInit, Mac};
use rbit_collaboration::security::{
    Finding, MAX_RESULTS, Report,
    automation::{DEPENDENCIES, SECRETS, dependencies, secrets},
};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(super) fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
impl SecurityService {
    pub(super) async fn secret_report(
        &self,
        ctx: &RepoContext,
        sha: &str,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Report, ServiceError> {
        let key = Zeroizing::new(self.ctx.database.security().fingerprint_key(ctx.repo.repo.id).await?);
        let mac = Hmac::<Sha256>::new_from_slice(&key).map_err(ServiceError::internal)?;
        let (findings, complete) = ctx
            .reader
            .inspect_security_text(sha.to_owned(), Vec::<Finding>::new(), move |path, text, findings| {
                let _retained = &permit;
                let (found, complete) = secrets::detect(path, text, |scope, token| {
                    let mut mac = mac.clone();
                    mac.update(scope.as_bytes());
                    mac.update(token);
                    hex(&mac.finalize().into_bytes())
                });
                let room = MAX_RESULTS.saturating_sub(findings.len());
                let fits = found.len() <= room;
                findings.extend(found.into_iter().take(room));
                complete && fits
            })
            .await?;
        Ok(Report { scanner: SECRETS.into(), complete, findings })
    }
    pub(super) async fn dependency_report(&self, ctx: &RepoContext, sha: &str) -> Result<Report, ServiceError> {
        let Some(blob) = ctx.reader.security_root_file(sha.to_owned(), "package-lock.json".into(), dependencies::MAX_LOCK_BYTES).await?
        else {
            if ctx.reader.security_root_file(sha.to_owned(), "package.json".into(), dependencies::MAX_LOCK_BYTES).await?.is_some() {
                return Err(ServiceError::Invalid("An npm manifest without a version-3 lockfile is incomplete coverage.".into()));
            }
            return Ok(Report { scanner: DEPENDENCIES.into(), complete: true, findings: Vec::new() });
        };
        let text = blob.text().ok_or_else(|| ServiceError::Invalid("The npm lockfile is not bounded UTF-8 text.".into()))?;
        let packages = dependencies::inventory(text.as_bytes()).map_err(|e| ServiceError::Invalid(e.into()))?;
        let client = AdvisoryClient::new()?;
        let vulnerabilities = client.vulnerabilities(&packages).await?;
        let mut findings = Vec::new();
        let mut complete = true;
        for (package, ids) in packages.iter().zip(vulnerabilities) {
            for id in ids {
                if findings.len() == MAX_RESULTS {
                    complete = false;
                    break;
                }
                let identity = digest(format!("npm\0{}\0{}\0{id}", package.location, package.name).as_bytes());
                findings.push(Finding { identity, rule: id.clone(), message: format!("{} {} is affected by {id} at {}. Provider severity is not normalized; this finding conservatively uses high severity. Only public registry metadata was queried.", package.name, package.version, package.location), path: "package-lock.json".into(), line: 1, severity: 3 });
            }
        }
        Ok(Report { scanner: DEPENDENCIES.into(), complete, findings })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [char::from(b"0123456789abcdef"[(byte >> 4) as usize]), char::from(b"0123456789abcdef"[(byte & 15) as usize])])
        .collect()
}
