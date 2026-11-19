//! Administrator workflow mapping and exact-source dispatch, without executing historical workflows.
use super::{ActionsService, Configuration, DeploymentRequest, PreparedDeployment, RepoContext, ServiceError, User};
use crate::{
    actions::dispatch::DispatchRequest,
    audit::{AuditAction, AuditEntry},
};
use rbit_core::{Timestamp, actions::Run};
use rbit_workflow::model::InputKind;
use std::collections::BTreeMap;

const REQUIRED_INPUTS: [&str; 3] = ["deployment_sha", "deployment_environment", "deployment_operation"];

impl ActionsService {
    /// Configure one target's trusted current-default-branch workflow; both environments must already exist.
    pub async fn configure_deployment_workflow(
        &self,
        actor: &User,
        repo: &RepoContext,
        environment: &str,
        source_environment: &str,
        workflow: &str,
    ) -> Result<Configuration, ServiceError> {
        let (actor, current) = self.deployment_writer(actor, repo, true).await?;
        for name in [environment, source_environment] {
            rbit_actions::environments::check_name_and_policy(name, rbit_actions::environments::RefPolicy::All)
                .map_err(ServiceError::Invalid)?;
            self.environment(&current, name).await?;
        }
        if environment.eq_ignore_ascii_case(source_environment) {
            return Err(ServiceError::Invalid("promotion needs distinct source and target environments".into()));
        }
        let form = self
            .dispatch_form(&current, workflow, "")
            .await?
            .ok_or_else(|| ServiceError::Invalid("the current workflow must support workflow_dispatch".into()))?;
        check_inputs(&form.inputs, false)?;
        let fence = self.ctx().database.actions_deployments().publication_fence().await?;
        let (_, fresh) = self.deployment_writer(&actor, &current, true).await?;
        for name in [environment, source_environment] {
            self.environment(&fresh, name).await?;
        }
        let configured = fence.put_configuration(fresh.repo.repo.id, environment, source_environment, &form.path, Timestamp::now()).await?;
        self.audit()
            .record(AuditEntry::new(AuditAction::ActionsDeploymentWorkflowChanged).by(&actor).on(fresh.repo.full_name()).detail(format!(
                "{} from {} via {} configuration {} generation {}",
                configured.environment, configured.source_environment, configured.workflow_path, configured.id, configured.generation
            )))
            .await;
        Ok(configured)
    }

    pub(super) async fn dispatch_deployment(
        &self,
        actor: &User,
        repo: &RepoContext,
        request: DeploymentRequest,
    ) -> Result<Run, ServiceError> {
        let (actor, current) = self.deployment_writer(actor, repo, false).await?;
        let (operation, source, configuration) = self.deployment_source(&current, &request).await?;
        let artifact = self.deployment_artifact(&current, &request, &source).await?;
        let reference = format!("refs/heads/{}", current.repo.repo.default_branch);
        let workflow_sha = current.reader.commit_id(reference.clone()).await?;
        let form = self
            .dispatch_form(&current, &configuration.workflow_path, &reference)
            .await?
            .ok_or_else(|| ServiceError::Invalid("the current configured workflow must support workflow_dispatch".into()))?;
        check_inputs(&form.inputs, artifact.is_some())?;
        let mut inputs = BTreeMap::from([
            ("deployment_sha".into(), source.sha.clone()),
            ("deployment_environment".into(), configuration.environment.clone()),
            ("deployment_operation".into(), operation.as_str().into()),
        ]);
        if let Some((id, digest)) = &artifact {
            inputs.insert("artifact_id".into(), id.0.to_string());
            inputs.insert("artifact_digest".into(), digest.clone());
        }
        let dispatch = DispatchRequest { workflow: configuration.workflow_path.clone(), reference: reference.clone(), inputs };
        let prepared = PreparedDeployment { request, operation, source, configuration, workflow_sha, reference, artifact };
        let run = self.dispatch_prepared(&actor, &current, &dispatch, Some(&prepared)).await?;
        let action = match operation {
            rbit_actions::deployments::Operation::Promote => AuditAction::ActionsDeploymentPromoted,
            rbit_actions::deployments::Operation::Rollback => AuditAction::ActionsDeploymentRolledBack,
        };
        self.audit()
            .record(AuditEntry::new(action).by(&actor).on(current.repo.full_name()).detail(format!(
                "{} source attempt {} SHA {} workflow {} at {} new run #{}",
                prepared.configuration.environment,
                prepared.source.id,
                prepared.source.sha,
                prepared.configuration.workflow_path,
                prepared.workflow_sha,
                run.number
            )))
            .await;
        Ok(run)
    }
}

fn check_inputs(inputs: &[rbit_workflow::model::DispatchInput], artifact: bool) -> Result<(), ServiceError> {
    for name in REQUIRED_INPUTS.into_iter().chain(artifact.then_some("artifact_id")).chain(artifact.then_some("artifact_digest")) {
        if !inputs.iter().any(|input| input.name == name && input.kind == InputKind::String) {
            return Err(ServiceError::Invalid(format!("the deployment workflow must declare string input {name}")));
        }
    }
    Ok(())
}
