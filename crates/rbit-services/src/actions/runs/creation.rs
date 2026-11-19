//! Planned run creation shares limits and concurrency, then atomically binds guarded deployment intent.
use super::{
    ActionsService, Allowance, Candidate, ErrorKind, JobRows, MAX_TEXT_CHARS, NewRun, Run, RunStart, RunTrust, ServiceError, Timestamp,
    Trigger, members, namespaced_group, new_run, scheduling, truncate_chars,
};

impl ActionsService {
    /// Creates the run `candidate` asks for, within `allowance`; `None` once the event started as many runs as it may.
    pub async fn start_run(
        &self,
        trigger: &Trigger<'_>,
        candidate: Candidate,
        allowance: &mut Allowance,
    ) -> Result<Option<Run>, ServiceError> {
        self.start_run_prepared(trigger, candidate, allowance, None).await
    }

    pub(in crate::actions) async fn start_run_prepared(
        &self,
        trigger: &Trigger<'_>,
        candidate: Candidate,
        allowance: &mut Allowance,
        deployment: Option<(&rbit_core::User, &crate::RepoContext, &crate::actions::deployments::PreparedDeployment)>,
    ) -> Result<Option<Run>, ServiceError> {
        // An archived repository is read-only: nothing starts there, whatever the trigger.
        if trigger.repo.repo.is_archived() {
            tracing::debug!(repo = %trigger.repo.full_name(), workflow = %candidate.path, "the repository is archived; no run");
            return Ok(None);
        }
        // Actions: disabled workflows. A disabled workflow starts nothing, for any event.
        if self.ctx().database.actions_settings().is_workflow_disabled(trigger.repo.repo.id, &candidate.path).await? {
            tracing::debug!(repo = %trigger.repo.full_name(), workflow = %candidate.path, "the workflow is disabled; no run");
            return Ok(None);
        }
        if !allowance.take_run() {
            tracing::warn!(repo = %trigger.repo.full_name(), workflow = %candidate.path, "an event may start at most {} runs; skipped", self.config().max_runs_per_event);
            return Ok(None);
        }
        let plan = match candidate.outcome {
            Ok(plan) => plan,
            Err(problem) => {
                if deployment.is_some() {
                    return Err(ServiceError::Invalid(problem.message));
                }
                return self.record_error(trigger, &candidate.path, &candidate.name, problem.reason, &problem.message).await.map(Some);
            }
        };
        if deployment.is_some() && plan.jobs.iter().any(|job| job.deferred) {
            return Err(ServiceError::Invalid(
                "deployment jobs must be fully planned from current workflow dispatch inputs; needs-dependent job plans are unsupported"
                    .into(),
            ));
        }
        let admitted = scheduling::check_jobs(&candidate.path, plan.jobs.len(), allowance, self.config().max_jobs_per_event)
            .and_then(|()| scheduling::check_labels(&candidate.path, plan.jobs.iter().flat_map(|job| &job.runs_on).map(String::as_str)));
        if let Err(error) = admitted {
            if deployment.is_some() {
                return Err(ServiceError::Invalid(error));
            }
            return self.record_error(trigger, &candidate.path, &candidate.name, ErrorKind::Limit, &error).await.map(Some);
        }
        if let Some((actor, repo, prepared)) = deployment {
            self.recheck_deployment(actor, repo, prepared).await?;
        }
        let group = match plan.concurrency.as_ref() {
            Some((group, cancel)) => Some((namespaced_group(trigger, group), *cancel)),
            None => self.supersede_group(trigger, &candidate.path).await?,
        };
        let awaiting = matches!(trigger.trust, RunTrust::Untrusted { awaits_approval: true });
        let decisions = self.inner.groups.lock().await;
        let start = if awaiting {
            RunStart::AwaitingApproval
        } else {
            let waiting = self.ctx().database.actions_runs().count_waiting(trigger.repo.repo.id).await?;
            let starting = u64::try_from(plan.jobs.iter().filter(|job| job.needs.is_empty()).count()).unwrap_or(u64::MAX);
            let max = u64::from(self.config().max_waiting_jobs_per_repository);
            if let Err(error) = scheduling::check_waiting(&candidate.path, waiting, starting, max) {
                if deployment.is_some() {
                    return Err(ServiceError::Invalid(error));
                }
                return self.record_error(trigger, &candidate.path, &candidate.name, ErrorKind::Limit, &error).await.map(Some);
            }
            match &group {
                Some(_) if deployment.is_some() => RunStart::HeldForConcurrency,
                Some((group, cancel)) => self.enter_group(trigger.repo.repo.id, group, *cancel, None, Some(trigger.sha)).await?,
                None => RunStart::Runnable,
            }
        };
        let rows = JobRows::new(&plan.jobs);
        let jobs = rows.jobs(&plan.jobs);
        let (path, name, title) = (
            truncate_chars(&candidate.path, MAX_TEXT_CHARS),
            truncate_chars(&candidate.name, MAX_TEXT_CHARS),
            truncate_chars(trigger.title, MAX_TEXT_CHARS),
        );
        let mut new = NewRun {
            concurrency_group: group.as_ref().map(|(group, _)| group.as_str()),
            cancel_in_progress: group.as_ref().is_some_and(|(_, cancel)| *cancel),
            start,
            jobs: &jobs,
            error: None,
            ..new_run(trigger, &path, &name, &title)
        };
        let store = self.ctx().database.actions_runs();
        let run = if let Some((actor, repo, prepared)) = deployment {
            self.create_checked_deployment_run(&mut new, actor, repo, prepared, group.as_ref()).await?
        } else {
            store.create_run(&new).await?
        };
        drop(decisions);
        tracing::info!(repo = %trigger.repo.full_name(), run = run.id.0, number = run.number, workflow = %run.workflow_path, event = %run.event, status = %run.status, "workflow run created");
        // Actions: join (J). Following up the creation decides the jobs that start with their
        // own `if:`, moves the scheduler's version and reports the statuses; a run those
        // decisions finish (every job skipped) releases its group's next run, which may have
        // been held behind it meanwhile.
        let settled = self.ctx().database.actions_runs().resettle(run.id, Timestamp::now()).await?;
        self.settled(&settled).await?;
        let run = self.ctx().database.actions_runs().run(run.id).await?.unwrap_or(run);
        Ok(Some(run))
    }
    async fn create_checked_deployment_run(
        &self,
        new: &mut NewRun<'_>,
        actor: &rbit_core::User,
        repo: &crate::RepoContext,
        prepared: &crate::actions::deployments::PreparedDeployment,
        group: Option<&(String, bool)>,
    ) -> Result<Run, ServiceError> {
        let store = self.ctx().database.actions_runs();
        let fence = self.ctx().database.actions_deployments().publication_fence().await?;
        self.recheck_deployment(actor, repo, prepared).await?;
        if let Some((group, cancel)) = group {
            let others = members(&store.unfinished_in_group(repo.repo.repo.id, group).await?);
            let entry = scheduling::enter_group(&others, None, *cancel);
            if !entry.cancel.is_empty() {
                return Err(ServiceError::Conflict(
                    "this deployment would cancel another run; wait for it or cancel it explicitly before submitting a new request",
                ));
            }
            new.start = match entry.start {
                scheduling::Start::Runnable => RunStart::Runnable,
                scheduling::Start::HeldForConcurrency => RunStart::HeldForConcurrency,
            };
        }
        Ok(store.create_deployment_run(new, fence, &prepared.intent(actor)).await?)
    }
}
