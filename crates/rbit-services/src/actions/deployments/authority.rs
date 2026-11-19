//! Current writer/configuration authority and exact successful source/artifact provenance.
use super::{ActionsService, Attempt, Configuration, DeploymentRequest, PreparedDeployment, RepoContext, ServiceError, User};
use rbit_actions::deployments::{Operation, Source, check_source};
use rbit_core::{
    Timestamp,
    actions::{ArtifactId, RunEvent, Status},
};

impl ActionsService {
    pub(super) async fn deployment_writer(
        &self,
        actor: &User,
        repo: &RepoContext,
        admin: bool,
    ) -> Result<(User, RepoContext), ServiceError> {
        if repo.viewer != Some(actor.id()) || !repo.access.can_write() || (admin && !repo.access.can_admin()) {
            return Err(ServiceError::Forbidden);
        }
        let actor = self.ctx().database.accounts().find_active_user(actor.id()).await?.ok_or(ServiceError::Forbidden)?;
        let current = crate::repositories::RepositoryService::new(self.ctx())
            .open(Some(&actor), repo.repo.owner.name.as_str(), repo.repo.repo.name.as_str())
            .await?;
        if current.repo.repo.id != repo.repo.repo.id || !current.access.can_write() || (admin && !current.access.can_admin()) {
            return Err(ServiceError::Forbidden);
        }
        current.ensure_writable()?;
        self.require_pages(&current).await?;
        Ok((actor, current))
    }

    pub(super) async fn deployment_source(
        &self,
        repo: &RepoContext,
        request: &DeploymentRequest,
    ) -> Result<(Operation, Attempt, Configuration), ServiceError> {
        let invalid = |message: &str| ServiceError::Invalid(message.into());
        let operation = Operation::parse(&request.operation).ok_or_else(|| invalid("choose promote or rollback"))?;
        let store = self.ctx().database.actions_deployments();
        let configuration = store.configuration(repo.repo.repo.id, &request.environment).await?.ok_or(ServiceError::NotFound)?;
        if configuration.id != request.configuration_id || configuration.generation != request.configuration_generation {
            return Err(ServiceError::Conflict("the deployment workflow configuration changed; reload before dispatching"));
        }
        let source = store.attempt(repo.repo.repo.id, request.source_attempt).await?.ok_or(ServiceError::NotFound)?;
        let event = RunEvent::parse(&source.event).ok_or_else(|| invalid("the recorded source event is unsupported"))?;
        check_source(
            operation,
            Source {
                successful: source.status == Status::Success,
                trusted: source.trusted,
                fork: source.is_fork,
                pull_request: event == RunEvent::PullRequest,
                environment: source.environment.as_deref(),
            },
            &configuration.source_environment,
            &configuration.environment,
        )
        .map_err(invalid)?;
        if !matches!(source.sha.len(), 40 | 64) || !source.sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid("the recorded source commit is invalid"));
        }
        repo.reader.commit_id(source.sha.clone()).await.map_err(|_| invalid("the selected source commit is no longer available"))?;
        Ok((operation, source, configuration))
    }

    pub(super) async fn deployment_artifact(
        &self,
        repo: &RepoContext,
        request: &DeploymentRequest,
        source: &Attempt,
    ) -> Result<Option<(ArtifactId, String)>, ServiceError> {
        match (request.artifact_id, request.artifact_digest.as_deref()) {
            (None, None) => Ok(None),
            (Some(id), Some(digest)) if id > 0 && digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()) => {
                let origin = self
                    .ctx()
                    .database
                    .actions_deployments()
                    .attempt(repo.repo.repo.id, source.origin_attempt_id.unwrap_or(source.id))
                    .await?
                    .ok_or(ServiceError::NotFound)?;
                if origin.sha != source.sha {
                    return Err(ServiceError::Invalid("the source origin identity is inconsistent".into()));
                }
                let id = ArtifactId(id);
                let artifact = self
                    .ctx()
                    .database
                    .actions_artifacts()
                    .find(repo.repo.repo.id, id, Timestamp::now())
                    .await?
                    .ok_or(ServiceError::NotFound)?;
                let run = self.ctx().database.actions_runs().run(artifact.run_id).await?.ok_or(ServiceError::NotFound)?;
                let expected = digest.to_ascii_lowercase();
                let job = artifact.job_id.ok_or_else(|| ServiceError::Invalid("the artifact has no recorded producing job".into()))?;
                let producing = self
                    .ctx()
                    .database
                    .actions_deployments()
                    .artifact_job(repo.repo.repo.id, origin.run_number, origin.run_attempt, job)
                    .await?;
                if artifact.run_id != origin.run_id
                    || artifact.attempt != origin.run_attempt
                    || run.number != origin.run_number
                    || run.sha != origin.workflow_sha
                    || rbit_workflow::protocol::sha256_from_hex(digest) != Some(artifact.sha256)
                    || !producing
                {
                    return Err(ServiceError::Invalid(
                        "the artifact must match the selected successful source run attempt and digest".into(),
                    ));
                }
                Ok(Some((id, expected)))
            }
            _ => Err(ServiceError::Invalid("provide both a recorded artifact id and its SHA-256 digest, or neither".into())),
        }
    }

    pub(in crate::actions) async fn recheck_deployment(
        &self,
        actor: &User,
        repo: &RepoContext,
        prepared: &PreparedDeployment,
    ) -> Result<(), ServiceError> {
        let (actor, current) = self.deployment_writer(actor, repo, false).await?;
        crate::agents::require_capability(self.ctx(), &actor, current.repo.repo.id, rbit_identity_models::agents::Capability::Dispatch)
            .await?;
        self.deployment_source(&current, &prepared.request).await?;
        self.deployment_artifact(&current, &prepared.request, &prepared.source).await?;
        let reference = format!("refs/heads/{}", current.repo.repo.default_branch);
        if reference != prepared.reference || current.reader.commit_id(reference).await? != prepared.workflow_sha {
            return Err(ServiceError::Conflict(
                "the default branch changed during deployment preparation; retry to use its current workflow",
            ));
        }
        if self.ctx().database.actions_settings().is_workflow_disabled(current.repo.repo.id, &prepared.configuration.workflow_path).await? {
            return Err(ServiceError::Forbidden);
        }
        Ok(())
    }
}
