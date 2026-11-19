//! Reviewable, resolver-free npm patch proposals with exact-base guarded branch creation.
mod publication;
use super::{SecurityService, advisories::AdvisoryClient};
use crate::{RepoContext, ServiceError};
use rbit_collaboration::security::automation::{
    DEPENDENCIES, FRESH_SECONDS,
    dependencies::{self, Package},
};
use rbit_core::{Timestamp, User};
use rbit_storage::sqlite::security::Remediation;

pub(super) struct Proposal {
    pub package: Package,
    pub version: String,
    pub manifest: String,
    pub lock: String,
}
impl SecurityService {
    pub async fn dependency_inventory(&self, ctx: &RepoContext, sha: &str) -> Result<Vec<Package>, ServiceError> {
        Self::require_read(ctx)?;
        let lock = ctx.reader.review_blob(sha.to_owned(), "package-lock.json".into(), dependencies::MAX_LOCK_BYTES).await?;
        dependencies::inventory(lock.text().ok_or_else(|| invalid("Unsupported lockfile."))?.as_bytes()).map_err(invalid)
    }
    pub async fn remediation_history(&self, ctx: &RepoContext) -> Result<Vec<Remediation>, ServiceError> {
        Self::require_read(ctx)?;
        Ok(self.ctx.database.security().remediations(ctx.repo.repo.id).await?)
    }
    pub async fn remediate_dependency(&self, actor: &User, ctx: &RepoContext, sha: &str, name: &str) -> Result<Remediation, ServiceError> {
        if !dependencies::package_name(name) || !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("Invalid package or immutable commit id."));
        }
        let (service, actor, ctx, sha, name) = (self.clone(), actor.clone(), ctx.clone(), sha.to_owned(), name.to_owned());
        let task = {
            let closed = self.closed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if *closed {
                return Err(ServiceError::Unavailable { retry_after_seconds: 1 });
            }
            let permit = self.slots.clone().try_acquire_owned().map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
            self.tasks.spawn(async move {
                let _permit = permit;
                service.remediate_inner(&actor, &ctx, &sha, &name).await
            })
        };
        task.await.map_err(ServiceError::internal)?
    }
    async fn remediate_inner(&self, actor: &User, ctx: &RepoContext, sha: &str, name: &str) -> Result<Remediation, ServiceError> {
        let current = self.current_admin(actor, ctx).await?;
        let config = self.automation_configuration(&current).await?.ok_or(ServiceError::NotFound)?;
        if !config.settings.dependencies || !config.settings.package_network_consent {
            return Err(ServiceError::Forbidden);
        }
        let scan = self
            .ctx
            .database
            .security()
            .latest_automatic(current.repo.repo.id, "npm", sha, config.generation)
            .await?
            .ok_or_else(|| invalid("Run a complete current dependency scan first."))?;
        if scan.scanner != DEPENDENCIES || scan.state != "complete" || scan.created_at < Timestamp::now().seconds() - FRESH_SECONDS {
            return Err(invalid("Run a complete current dependency scan first."));
        }
        if current.reader.commit_id(format!("refs/heads/{}", current.repo.repo.default_branch)).await? != sha {
            return Err(ServiceError::Conflict("the default branch moved; rescan before creating an update"));
        }
        let proposal = tokio::time::timeout(std::time::Duration::from_secs(90), self.proposal(&current, sha, name))
            .await
            .map_err(|_| ServiceError::Unavailable { retry_after_seconds: 60 })??;
        self.publish_dependency(actor, &current, sha, config.generation, proposal).await
    }
    async fn proposal(&self, ctx: &RepoContext, sha: &str, name: &str) -> Result<Proposal, ServiceError> {
        let packages = self.dependency_inventory(ctx, sha).await?;
        let package =
            packages.into_iter().find(|p| p.name == name).ok_or_else(|| invalid("The package is not in the scanned lockfile."))?;
        let client = AdvisoryClient::new()?;
        let ids = client.vulnerabilities(std::slice::from_ref(&package)).await?.pop().unwrap_or_default();
        if ids.is_empty() {
            return Err(invalid("No current advisory requires an update for this package."));
        }
        let before = semver::Version::parse(&package.version).map_err(|_| invalid("Invalid package version."))?;
        let mut versions = std::collections::BTreeSet::new();
        for id in ids.iter().take(20) {
            let advisory = client.advisory(id).await?;
            if let Some(affected) = advisory["affected"].as_array() {
                for item in affected.iter().filter(|a| a["package"]["ecosystem"] == "npm" && a["package"]["name"] == package.name) {
                    for range in item["ranges"].as_array().into_iter().flatten() {
                        for event in range["events"].as_array().into_iter().flatten() {
                            if let Some(fixed) = event["fixed"].as_str().and_then(|v| semver::Version::parse(v).ok())
                                && fixed > before
                                && fixed.major == before.major
                                && fixed.minor == before.minor
                                && fixed.pre.is_empty()
                                && fixed.build.is_empty()
                            {
                                versions.insert(fixed);
                            }
                        }
                    }
                }
            }
        }
        let lock = ctx.reader.review_blob(sha.to_owned(), "package-lock.json".into(), dependencies::MAX_LOCK_BYTES).await?;
        let manifest = ctx.reader.review_blob(sha.to_owned(), "package.json".into(), dependencies::MAX_LOCK_BYTES).await?;
        for version in versions.into_iter().take(5) {
            let version = version.to_string();
            let candidate = Package { version: version.clone(), ..package.clone() };
            if client.vulnerabilities(&[candidate]).await?.iter().any(|v| !v.is_empty()) {
                continue;
            }
            let metadata = client.version(name, &version).await?;
            let (manifest, lock) = dependencies::repair(
                manifest.text().ok_or_else(|| invalid("Unsupported package manifest."))?.as_bytes(),
                lock.text().ok_or_else(|| invalid("Unsupported lockfile."))?.as_bytes(),
                &package,
                &metadata,
            )
            .map_err(invalid)?;
            return Ok(Proposal { package, version, manifest, lock });
        }
        Err(invalid(
            "No supported safe patch was found. Update this dependency manually; automatic repair supports pinned direct leaf packages only.",
        ))
    }
}
fn invalid(message: impl Into<String>) -> ServiceError {
    ServiceError::Invalid(message.into())
}
