//! Guarded creation and bounded retry recovery of a dependency proposal's exact two-file tree.
#[cfg(test)]
mod tests;
use super::{Proposal, SecurityService, invalid};
use crate::{RepoContext, ServiceError};
use rbit_core::{
    PushSource, RefUpdate, Timestamp, User,
    file_edit::FilePath,
    pull::{CommitIdentity, noreply_domain},
};
use rbit_git::{
    DiffLimits,
    edit::{Change, EditError, EditSpec, EditWorkspace},
    merge::MergeEnvironment,
};
use rbit_storage::sqlite::{
    pulls::{PullFilter, PullStateFilter},
    security::Remediation,
};
use std::time::Duration;

impl SecurityService {
    pub(super) async fn publish_dependency(
        &self,
        actor: &User,
        ctx: &RepoContext,
        sha: &str,
        generation: i64,
        proposal: Proposal,
    ) -> Result<Remediation, ServiceError> {
        let key = super::super::scanners::digest(
            format!("{}\0{sha}\0{}\0{}", ctx.repo.repo.id.0, proposal.package.name, proposal.version).as_bytes(),
        );
        let branch = format!("rbit/dependency/{}", &key[..24]);
        let _lease = self.ctx.repositories.lease(ctx.repo.repo.id).await;
        let fence = self.ctx.database.security().publication_fence().await?;
        self.current_admin(actor, ctx).await?;
        let mut record = fence.reserve_remediation(ctx.repo.repo.id, sha, &proposal.package.name, &proposal.version, &branch).await?;
        if record.pull_number.is_some() {
            return Ok(record);
        }
        if record.base_sha != sha || record.branch != branch {
            return Err(ServiceError::Conflict("an earlier proposal for this version needs manual completion"));
        }
        let (commit, created) = self.dependency_commit(actor, ctx, sha, generation, &branch, &proposal).await?;
        if record.commit_sha.as_ref().is_some_and(|saved| saved != &commit) {
            return Err(ServiceError::Conflict("the update branch was edited; review it manually"));
        }
        if created || record.commit_sha.is_none() {
            crate::PushService::new(self.ctx.clone())
                .record(
                    ctx.repo.repo.id,
                    PushSource::User(actor.id()),
                    vec![RefUpdate { name: format!("refs/heads/{branch}"), before: None, after: Some(commit.clone()) }],
                )
                .await;
        }
        self.ctx.database.security().record_remediation(ctx.repo.repo.id, record.id, &commit, None).await?;
        let current = self.current_admin(actor, ctx).await?;
        let policy = self.ctx.database.security().automation(ctx.repo.repo.id).await?.ok_or(ServiceError::NotFound)?;
        if policy.generation != generation
            || !policy.settings.dependencies
            || !policy.settings.package_network_consent
            || current.repo.repo.visibility != ctx.repo.repo.visibility
        {
            return Err(ServiceError::Conflict("update publication settings or visibility changed; review the prepared branch"));
        }
        let number = if let Some(number) = self.recovered_dependency_pull(&current, &branch, &commit).await? {
            number
        } else {
            let result = self.pulls.create(actor, &current, &current, crate::pulls::NewPullInput { base_branch: current.repo.repo.default_branch.clone(), head_branch: branch.clone(), title: format!("Update {} to {}", proposal.package.name, proposal.version), body: "This proposed dependency update changes only package.json and package-lock.json. Review the diff and require CI before merging. No package installation or repository scripts ran while preparing it.".into(), draft: false }).await;
            match result {
                Ok((issue, _)) => i64::from(issue.number.get()),
                Err(error) => self.recovered_dependency_pull(&current, &branch, &commit).await?.ok_or(error)?,
            }
        };
        self.ctx.database.security().record_remediation(ctx.repo.repo.id, record.id, &commit, Some(number)).await?;
        record.commit_sha = Some(commit);
        record.pull_number = Some(number);
        Ok(record)
    }
    async fn dependency_commit(
        &self,
        actor: &User,
        ctx: &RepoContext,
        sha: &str,
        generation: i64,
        branch: &str,
        proposal: &Proposal,
    ) -> Result<(String, bool), ServiceError> {
        let base_tree = ctx.reader.tree(sha.into(), String::new()).await?;
        if ["package.json", "package-lock.json"]
            .iter()
            .any(|path| !base_tree.entries.iter().any(|entry| entry.name == *path && entry.kind == rbit_git::EntryKind::File))
        {
            return Err(invalid("Dependency proposals require regular non-executable package files."));
        }
        let environment = MergeEnvironment {
            git_binary: self.ctx.config.repository.git_binary.clone(),
            tmp_dir: self.ctx.config.storage.tmp_dir(),
            timeout: Duration::from_secs(30),
        };
        let workspace = EditWorkspace::open(&environment, &ctx.disk_path).await.map_err(ServiceError::internal)?;
        let mut changes = Vec::new();
        for (path, content) in [("package.json", &proposal.manifest), ("package-lock.json", &proposal.lock)] {
            let path = FilePath::parse(path).map_err(|e| invalid(e.to_string()))?;
            let blob = workspace.write_blob(content.as_bytes(), 2 * 1024 * 1024).await.map_err(ServiceError::internal)?;
            changes.push(Change::Update { path, blob });
        }
        let mut created = false;
        let commit = if let Ok(existing) = ctx.reader.commit_id(format!("refs/heads/{branch}")).await {
            self.exact_dependency_tree(ctx, sha, &existing, proposal).await?;
            existing
        } else {
            let identity = CommitIdentity::noreply(
                actor.name().as_str(),
                actor.account.display_name.as_deref(),
                &noreply_domain(&self.ctx.config.pulls.noreply_domain, &self.ctx.config.server.public_url),
            );
            let message = format!("Update {} to {}", proposal.package.name, proposal.version);
            let spec = EditSpec {
                branch: &ctx.repo.repo.default_branch,
                parent: sha,
                new_branch: Some(branch),
                changes: &changes,
                message: &message,
                author: &identity,
                at: Timestamp::now(),
            };
            let guard = async {
                let fence = self.ctx.database.security().publication_fence().await.map_err(|e| EditError::Rejected(e.to_string()))?;
                self.dependency_authority(actor, ctx, sha, generation, branch).await.map_err(|e| EditError::Rejected(e.to_string()))?;
                Ok(fence)
            };
            created = true;
            workspace
                .commit_guarded(&spec, guard)
                .await
                .map_err(|error| match error {
                    EditError::BranchMoved => ServiceError::Conflict("the update branch already exists"),
                    EditError::Rejected(_) => ServiceError::Conflict("update publication authority changed; retry after review"),
                    other => ServiceError::internal(other),
                })?
                .commit
        };
        Ok((commit, created))
    }
    async fn recovered_dependency_pull(&self, ctx: &RepoContext, branch: &str, commit: &str) -> Result<Option<i64>, ServiceError> {
        let filter = PullFilter {
            state: PullStateFilter::All,
            head_repo: Some(ctx.repo.repo.id),
            head_branch: Some(branch.into()),
            ..PullFilter::default()
        };
        let pulls = self.ctx.database.pulls().list(ctx.repo.repo.id, &filter, 2, 0).await?;
        let Some((issue, pull)) = pulls.first() else {
            return Ok(None);
        };
        if pulls.len() != 1
            || pull.repo_id != ctx.repo.repo.id
            || pull.head_repo_id != Some(ctx.repo.repo.id)
            || pull.head_branch != branch
            || pull.base_branch != ctx.repo.repo.default_branch
            || pull.head_sha != commit
        {
            return Err(ServiceError::Conflict("the reserved update branch has an ambiguous or edited pull request"));
        }
        // Include closed requests: a retry must respect the reviewer's decision to close one.
        Ok(Some(i64::from(issue.number.get())))
    }
    async fn dependency_authority(
        &self,
        actor: &User,
        ctx: &RepoContext,
        sha: &str,
        generation: i64,
        branch: &str,
    ) -> Result<(), ServiceError> {
        let mut current = self.current_admin(actor, ctx).await?;
        let config = self.ctx.database.security().automation(ctx.repo.repo.id).await?.ok_or(ServiceError::NotFound)?;
        if current.repo.repo.visibility != ctx.repo.repo.visibility
            || config.generation != generation
            || !config.settings.dependencies
            || !config.settings.package_network_consent
            || self.ctx.database.mirrors().is_mirror(ctx.repo.repo.id).await?
        {
            return Err(ServiceError::Forbidden);
        }
        if current.reader.commit_id(format!("refs/heads/{}", current.repo.repo.default_branch)).await? != sha
            || current.reader.commit_id(format!("refs/heads/{branch}")).await.is_ok()
        {
            return Err(ServiceError::Conflict("a branch changed during dependency preparation"));
        }
        current.access = rbit_core::RepoAccess::Write;
        if let Some(reason) = crate::ProtectionService::new(self.ctx.clone()).branch_update_refusal(&current, actor, branch).await? {
            return Err(invalid(reason));
        }
        Ok(())
    }
    async fn exact_dependency_tree(&self, ctx: &RepoContext, base: &str, head: &str, proposal: &Proposal) -> Result<(), ServiceError> {
        let before = ctx.reader.tree(base.into(), String::new()).await?;
        let after = ctx.reader.tree(head.into(), String::new()).await?;
        if after.commit.parent_ids.as_slice() != [base] {
            return Err(ServiceError::Conflict("the reserved update branch has unexpected ancestry"));
        }
        for path in ["package.json", "package-lock.json"] {
            let old = before.entries.iter().find(|entry| entry.name == path);
            let new = after.entries.iter().find(|entry| entry.name == path);
            if !matches!((old, new), (Some(old), Some(new)) if old.kind == rbit_git::EntryKind::File && new.kind == old.kind) {
                return Err(ServiceError::Conflict("the reserved update branch changed file modes"));
            }
        }
        let diff = ctx.reader.review_changes(base.into(), head.into(), DiffLimits { max_files: 3, ..DiffLimits::default() }).await?;
        if diff.truncated
            || diff.files.len() != 2
            || diff.files.iter().any(|f| !matches!(f.path.as_str(), "package.json" | "package-lock.json") || f.old_path.is_some())
        {
            return Err(ServiceError::Conflict("the reserved update branch contains other changes"));
        }
        for (path, expected) in [("package.json", &proposal.manifest), ("package-lock.json", &proposal.lock)] {
            let blob = ctx.reader.blob(head.into(), path.into(), 2 * 1024 * 1024).await?;
            if blob.text() != Some(expected.as_str()) {
                return Err(ServiceError::Conflict("the reserved update branch was edited"));
            }
        }
        Ok(())
    }
}
