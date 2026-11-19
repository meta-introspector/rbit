//! Resolve ownership from the trusted base commit and current repository grants.

use std::collections::{BTreeMap, BTreeSet};

use rbit_collaboration::pulls::{
    codeowners::{CodeOwners, MAX_BYTES},
    tally,
};
use rbit_core::{AccountId, Issue, TimelineEvent, Timestamp, issue::IssueAction, pull::PullRequest};
use rbit_git::GitError;

use super::{Blocked, MergeFailure, PullContext, PullService};
use crate::{RepoContext, ServiceError};
use rbit_core::User;

/// One group per matching rule; each group needs one eligible approving owner.
#[derive(Default)]
struct Ownership {
    groups: Vec<BTreeSet<AccountId>>,
    invalid: bool,
}

impl PullService {
    /// Reevaluate ownership against the exact base before each ref update attempt.
    pub(super) async fn recheck_owner_merge(
        &self,
        actor: &User,
        ctx: &PullContext,
        base_tip: &str,
        bypass: bool,
    ) -> Result<(), MergeFailure> {
        let readiness = self.merge_readiness(Some(actor), ctx, None).await?;
        if readiness.base_tip.as_deref() != Some(base_tip) {
            return Err(MergeFailure::Blocked(Blocked::base_changed()));
        }
        if readiness.rules.require_code_owner_reviews && !(bypass && readiness.rules.merger_may_bypass) {
            let missing = readiness
                .evaluation
                .checks
                .iter()
                .find(|check| matches!(check.requirement, rbit_core::pull::Requirement::CodeOwners { .. }) && !check.met);
            if let Some(check) = missing {
                return Err(MergeFailure::Blocked(Blocked::unmet(Some(check.requirement), &readiness.rules)));
            }
        }
        let current = self.ctx().database.pulls().find(ctx.issue().id).await?.ok_or(ServiceError::NotFound)?;
        if current.head_sha != ctx.pull.head_sha {
            return Err(MergeFailure::HeadChanged);
        }
        if current.base_branch != ctx.pull.base_branch {
            return Err(MergeFailure::BaseChanged);
        }
        Ok(())
    }

    async fn ownership(&self, repo: &RepoContext, issue: &Issue, pull: &PullRequest, base: &str) -> Result<Ownership, ServiceError> {
        let mut source = None;
        for path in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
            match repo.reader.blob(base.to_owned(), path.to_owned(), MAX_BYTES).await {
                Ok(blob) => {
                    source = blob.text().map(str::to_owned);
                    if source.is_none() {
                        return Ok(Ownership { invalid: true, ..Ownership::default() });
                    }
                    break;
                }
                Err(GitError::NotFound(_)) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let Some(source) = source else { return Ok(Ownership::default()) };
        let Ok(owners) = CodeOwners::parse(&source) else { return Ok(Ownership { invalid: true, ..Ownership::default() }) };
        let (paths, incomplete) = repo.reader.changed_paths(base.to_owned(), pull.head_sha.clone(), 1000).await?;
        if incomplete {
            return Ok(Ownership { invalid: true, ..Ownership::default() });
        }
        let mut names = BTreeSet::new();
        let mut work = 0_usize;
        for path in &paths {
            work = work.saturating_add(source.len().saturating_add(6).saturating_mul(path.len()));
            if path.len() > 4096 || work > 16_000_000 {
                return Ok(Ownership { invalid: true, ..Ownership::default() });
            }
            if let Some(group) = owners.owners(path).filter(|group| !group.is_empty()) {
                names.insert(group.to_vec());
            }
        }

        if names.iter().flatten().collect::<BTreeSet<_>>().len() > 100 {
            return Ok(Ownership { invalid: true, ..Ownership::default() });
        }
        let mut resolved = BTreeMap::new();
        let mut result = Ownership::default();
        let mut candidate_count = 0;
        for group in names {
            let mut eligible = BTreeSet::new();
            for name in group {
                if let std::collections::btree_map::Entry::Vacant(entry) = resolved.entry(name.clone()) {
                    let candidates = self.owner_candidates(repo, &name).await?;
                    candidate_count += candidates.len();
                    if candidates.len() > 100 || candidate_count > 500 {
                        return Ok(Ownership { invalid: true, ..Ownership::default() });
                    }
                    let mut current = BTreeSet::new();
                    for id in candidates {
                        if Some(id) != issue.author
                            && self.ctx().database.accounts().find_active_user(id).await?.is_some()
                            && self.access_of(id, &repo.repo).await?.can_write()
                        {
                            current.insert(id);
                        }
                    }
                    entry.insert(current);
                }
                if let Some(ids) = resolved.get(&name) {
                    eligible.extend(ids);
                }
            }
            // An unresolved owner must never turn a protected path into an unowned path.
            result.groups.push(eligible);
        }
        Ok(result)
    }

    async fn owner_candidates(&self, repo: &RepoContext, name: &str) -> Result<Vec<AccountId>, ServiceError> {
        let name = name.strip_prefix('@').unwrap_or(name);
        if let Some((organization, team)) = name.split_once('/') {
            if !organization.eq_ignore_ascii_case(repo.repo.owner.name.as_str()) {
                return Ok(Vec::new());
            }
            let teams = self.ctx().database.teams();
            let Some(team) = teams.find_by_name(repo.repo.repo.owner_id, team).await? else { return Ok(Vec::new()) };
            let members = teams.members_bounded(team.id, 101).await?;
            Ok(members.into_iter().map(|account| account.id).collect())
        } else {
            Ok(self.ctx().database.accounts().find_account_by_name(name).await?.map(|account| account.id).into_iter().collect())
        }
    }

    pub(super) async fn owner_approval_missing(
        &self,
        repo: &RepoContext,
        issue: &Issue,
        pull: &PullRequest,
        base: &str,
        approvals: &[AccountId],
    ) -> Result<usize, ServiceError> {
        let ownership = self.ownership(repo, issue, pull, base).await?;
        Ok(usize::from(ownership.invalid) + ownership.groups.iter().filter(|group| !approvals.iter().any(|id| group.contains(id))).count())
    }

    /// Best-effort delivery must not turn a committed pull request into a failed request.
    pub(super) async fn route_code_owners(&self, repo: &RepoContext, issue: &Issue, pull: &PullRequest) {
        if let Err(error) = self.request_code_owners(repo, issue, pull).await {
            tracing::warn!(%error, pull = issue.id.0, "could not request code owners");
        }
    }

    /// System-owned requests use the existing author as attribution and existing dedupe.
    pub(super) async fn request_code_owners(&self, repo: &RepoContext, issue: &Issue, pull: &PullRequest) -> Result<(), ServiceError> {
        if pull.is_draft || !issue.is_open() || repo.repo.repo.is_archived() {
            return Ok(());
        }
        let Some(author) = issue.author else { return Ok(()) };
        let base = repo.reader.commit_id(format!("refs/heads/{}", pull.base_branch)).await?;
        let ownership = self.ownership(repo, issue, pull, &base).await?;
        if ownership.invalid {
            return Ok(());
        }
        let reviewers: BTreeSet<_> = ownership.groups.into_iter().flatten().collect();
        if reviewers.len() > 100 {
            return Ok(());
        }
        let reviews = self.ctx().database.reviews().reviews(issue.id).await?;
        let approved = tally(&reviews, issue.author, &pull.head_sha, true, |id| reviewers.contains(&id)).approvals;
        for reviewer in reviewers {
            if approved.contains(&reviewer) {
                continue;
            }
            if self.ctx().database.reviews().request(issue.id, reviewer, author, Timestamp::now()).await? {
                self.inner.timeline.writer().record(issue.id, None, TimelineEvent::CodeOwnerReviewRequested { reviewer }).await?;
                self.publish(issue, None, IssueAction::ReviewRequested { reviewer });
            }
        }
        Ok(())
    }
}
