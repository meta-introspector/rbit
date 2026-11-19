//! One owned automatic worker per service; durable claims recover after process interruption.
use super::{DEPENDENCIES, SECRETS, SecurityService};
use crate::{DomainEvent, ServiceError};
use rbit_core::{RepoId, Timestamp};
use std::{sync::atomic::Ordering, time::Duration};

impl SecurityService {
    pub fn spawn_background(&self) {
        if self.background.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let service = self.clone();
        let events = self.ctx.events.subscribe();
        self.tasks.spawn(async move {
            service.run_automation(events).await;
        });
    }
    async fn run_automation(&self, mut events: tokio::sync::broadcast::Receiver<DomainEvent>) {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        let mut cursor = 0;
        loop {
            let repo = tokio::select! {
                () = self.background.stop.cancelled() => break,
                event = events.recv() => match event {
                    Ok(DomainEvent::RepositoryPushed { repo, .. } | DomainEvent::Issue { repo, .. }) => Some(repo),
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    _ => continue,
                },
                _ = interval.tick() => None,
            };
            let now = Timestamp::now();
            if let Some(repo) = repo {
                let _ = self.enqueue_current(repo, now).await;
            } else if let Ok(repos) = self.ctx.database.security().automatic_repositories(cursor).await {
                cursor = repos.last().map_or(0, |id| id.0);
                for repo in repos {
                    let _ = self.enqueue_current(repo, now).await;
                }
            }
            for _ in 0..8 {
                if self.background.stop.is_cancelled() {
                    return;
                }
                match self.run_one_automatic_scan().await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(%error, "automatic security worker unavailable");
                        break;
                    }
                }
            }
        }
    }
    /// Public for deterministic worker/lease tests without timers or an event listener.
    pub async fn run_one_automatic_scan(&self) -> Result<bool, ServiceError> {
        let Some(job) = self.ctx.database.security().claim_scan(Timestamp::now()).await? else { return Ok(false) };
        let outcome = tokio::time::timeout(Duration::from_secs(90), self.scan_claimed(RepoId(job.repo_id), job.scan_id)).await;
        if !matches!(outcome, Ok(Ok(()))) {
            // Provider, unsupported coverage and revoked authority never become clean evidence.
            self.ctx.database.security().abandon_scan(job.scan_id, Timestamp::now()).await?;
        }
        Ok(true)
    }
    async fn scan_claimed(&self, repo: RepoId, id: i64) -> Result<(), ServiceError> {
        let store = self.ctx.database.security();
        let config = store.automation(repo).await?.ok_or(ServiceError::NotFound)?;
        let (actor, ctx) = self.automation_actor(repo, &config).await?;
        let scan = store.scan(repo, id).await?.ok_or(ServiceError::NotFound)?;
        if scan.generation != config.generation || !config.settings.channels().any(|name| name == scan.scanner) {
            return Err(ServiceError::Forbidden);
        }
        let permit = self.slots.clone().acquire_owned().await.map_err(|_| ServiceError::Unavailable { retry_after_seconds: 1 })?;
        let report = match scan.scanner.as_str() {
            SECRETS => self.secret_report(&ctx, &scan.commit_sha, permit).await?,
            DEPENDENCIES if config.settings.package_network_consent => {
                let _permit = permit;
                self.dependency_report(&ctx, &scan.commit_sha).await?
            }
            _ => return Err(ServiceError::Forbidden),
        };
        let lease = self.ctx.repositories.lease(repo).await;
        let fence = store.publication_fence().await?;
        self.automation_actor(repo, &config).await?;
        fence.finish_scan(repo, &scan, &report).await?;
        drop(lease);
        if report.complete
            && !report.findings.is_empty()
            && scan.scanner == DEPENDENCIES
            && config.settings.automatic_pull_requests
            && ctx.reader.commit_id(format!("refs/heads/{}", ctx.repo.repo.default_branch)).await.ok().as_deref() == Some(&scan.commit_sha)
        {
            let existing = store.remediations(repo).await?;
            let packages = self.dependency_inventory(&ctx, &scan.commit_sha).await?;
            for package in packages.into_iter().filter(|p| p.location == format!("node_modules/{}", p.name)).take(5) {
                if existing.iter().any(|r| r.base_sha == scan.commit_sha && r.package == package.name) {
                    continue;
                }
                // Unsupported repair is explicit on the manual endpoint; automatic proposals
                // never weaken the scan result or attempt a broader dependency resolution.
                let _ = self.remediate_dependency(&actor, &ctx, &scan.commit_sha, &package.name).await;
            }
        }
        Ok(())
    }
}
