//! Upload provenance is server-owned; PR and reusable workflow task grants never qualify.
use super::{Configuration, SecurityService, Uploader};
use crate::{RepoContext, ServiceError};
use rbit_core::{
    User,
    actions::{RunEvent, Status},
};
impl SecurityService {
    pub(super) async fn current_admin(&self, user: &User, ctx: &RepoContext) -> Result<RepoContext, ServiceError> {
        if !ctx.access.can_admin() {
            return Err(ServiceError::NotFound);
        }
        let actor = self.ctx.database.accounts().find_active_user(user.id()).await?.ok_or(ServiceError::NotFound)?;
        let current = crate::repositories::RepositoryService::new(&self.ctx)
            .open(Some(&actor), ctx.repo.owner.name.as_str(), ctx.repo.repo.name.as_str())
            .await?;
        if current.repo.repo.id != ctx.repo.repo.id || !current.access.can_admin() {
            return Err(ServiceError::NotFound);
        }
        current.ensure_writable()?;
        Ok(current)
    }
    pub(super) async fn authorize(&self, actor: Uploader<'_>, ctx: &RepoContext, config: &Configuration) -> Result<String, ServiceError> {
        match actor {
            Uploader::User(user) => {
                self.current_admin(user, ctx).await?;
                Ok(format!("user:{}", user.id().0))
            }
            Uploader::Task(grant) => {
                if grant.repo != ctx.repo.repo.id || !grant.contents().can_write() || config.workflow.is_empty() {
                    return Err(ServiceError::Forbidden);
                }
                ctx.ensure_writable()?;
                let store = self.ctx.database.actions_runs();
                let task = store.task(grant.task).await?.ok_or(ServiceError::Forbidden)?;
                let job = store.job(grant.job).await?.ok_or(ServiceError::Forbidden)?;
                let run = store.run(grant.run).await?.ok_or(ServiceError::Forbidden)?;
                let repo = self.ctx.database.admin().find_repository(grant.repo).await?.ok_or(ServiceError::NotFound)?;
                if repo.repo.is_archived() {
                    return Err(ServiceError::Archived);
                }
                let policies = self.ctx.database.actions_settings();
                let repository_policy = policies.policies(grant.repo).await?;
                let owner_policy = policies.owner_settings(repo.repo.owner_id).await?;
                let default = rbit_core::actions::permissions::effective(&repository_policy, &owner_policy.policies).token_permission;
                let declared = job
                    .permissions
                    .as_deref()
                    .map(serde_json::from_str::<rbit_core::actions::TokenPermissions>)
                    .transpose()
                    .map_err(|_| ServiceError::Forbidden)?;
                let permissions = rbit_core::actions::token_permissions(
                    default,
                    None,
                    declared,
                    rbit_actions::trust::RunTrust::from_stored(run.trusted).is_trusted(),
                );
                if permissions.contents != rbit_core::actions::PermissionLevel::Write {
                    return Err(ServiceError::Forbidden);
                }
                let branch = format!("refs/heads/{}", repo.repo.default_branch);
                let tip = ctx.reader.commit_id(branch.clone()).await?;
                if task.job_id != grant.job
                    || task.status != Status::Running
                    || job.run_id != grant.run
                    || job.repo_id != grant.repo
                    || job.key.contains('/')
                    || !rbit_actions::trust::RunTrust::from_stored(run.trusted).is_trusted()
                    || run.is_fork
                    || run.pull_number.is_some()
                    || run.event == RunEvent::PullRequest
                    || run.reference != branch
                    || run.sha != tip
                    || run.workflow_path != config.workflow
                    || !self.ctx.database.actions_settings().is_enabled(grant.repo).await?
                {
                    return Err(ServiceError::Forbidden);
                }
                if let Some(actor) = run.actor {
                    let user = self.ctx.database.accounts().find_active_user(actor).await?.ok_or(ServiceError::Forbidden)?;
                    let current = crate::repositories::RepositoryService::new(&self.ctx)
                        .open(Some(&user), ctx.repo.owner.name.as_str(), ctx.repo.repo.name.as_str())
                        .await?;
                    if current.repo.repo.id != grant.repo || !current.access.can_write() {
                        return Err(ServiceError::Forbidden);
                    }
                } else if run.event != RunEvent::Schedule {
                    return Err(ServiceError::Forbidden);
                }
                // The configured workflow must still exist at the exact trusted default tip.
                let workflow = ctx.reader.blob(tip, config.workflow.clone(), 256 * 1024).await?;
                if workflow.text().is_none() {
                    return Err(ServiceError::Forbidden);
                }
                Ok(format!("actions:{}:{}", grant.run.0, grant.task.0))
            }
        }
    }
}
