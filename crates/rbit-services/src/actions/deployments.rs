//! Deployments select immutable source identity and dispatch only the current configured workflow.
mod authority;
mod publication;

use super::ActionsService;
use crate::{RepoContext, ServiceError};
use rbit_actions::deployments::Operation;
use rbit_core::{
    User,
    actions::{ArtifactId, Run},
};
use rbit_storage::sqlite::actions_deployments::{ActiveAttempt, Attempt, Configuration, Intent, NewIntent};
use serde::Deserialize;

/// A writer's exact source selection; administrator configuration is bound by identity and generation.
#[derive(Debug, Clone, Deserialize)]
pub struct DeploymentRequest {
    pub environment: String,
    pub operation: String,
    pub source_attempt: i64,
    pub configuration_id: i64,
    pub configuration_generation: i64,
    pub artifact_id: Option<i64>,
    pub artifact_digest: Option<String>,
}

/// A bounded dashboard snapshot. Completed attempts survive Actions run retention.
#[derive(Debug)]
pub struct Dashboard {
    pub attempts: Vec<Attempt>,
    pub active: Vec<ActiveAttempt>,
    pub configurations: Vec<Configuration>,
    pub intents: Vec<Intent>,
}

#[derive(Debug)]
pub(super) struct PreparedDeployment {
    request: DeploymentRequest,
    operation: Operation,
    source: Attempt,
    configuration: Configuration,
    workflow_sha: String,
    reference: String,
    artifact: Option<(ArtifactId, String)>,
}

impl PreparedDeployment {
    pub(super) fn intent(&self, actor: &User) -> NewIntent<'_> {
        NewIntent {
            actor: actor.id(),
            operation: self.operation.as_str(),
            environment: &self.configuration.environment,
            source_attempt: self.source.id,
            source_sha: &self.source.sha,
            artifact: self.artifact.as_ref().map(|(id, digest)| (id.0, digest.as_str())),
            configuration_id: self.configuration.id,
            configuration_generation: self.configuration.generation,
            workflow_sha: &self.workflow_sha,
        }
    }
}

impl ActionsService {
    pub async fn deployment_dashboard(&self, repo: &RepoContext, before: Option<i64>) -> Result<Dashboard, ServiceError> {
        self.require_pages(repo).await?;
        let store = self.ctx().database.actions_deployments();
        Ok(Dashboard {
            attempts: store.attempts(repo.repo.repo.id, before, 50).await?,
            active: store.active_attempts(repo.repo.repo.id).await?,
            configurations: store.configurations(repo.repo.repo.id).await?,
            intents: store.intents(repo.repo.repo.id, 50).await?,
        })
    }

    /// Whether a run is bound to deployment intent and must be restarted using a new current-workflow request.
    pub async fn is_deployment_run(&self, repo: &RepoContext, run: rbit_core::actions::RunId) -> Result<bool, ServiceError> {
        self.require_pages(repo).await?;
        Ok(self.ctx().database.actions_deployments().is_deployment_run(repo.repo.repo.id, run).await?)
    }

    /// Dispatch a promotion/rollback with exact selected identity and current administrator configuration.
    pub async fn deploy(&self, actor: &User, repo: &RepoContext, request: DeploymentRequest) -> Result<Run, ServiceError> {
        self.dispatch_deployment(actor, repo, request).await
    }
}
