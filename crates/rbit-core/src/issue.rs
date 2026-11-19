//! Issues and pull requests: the shared core of milestone 2 wave B.
//!
//! A pull request is an issue with a branch attached. Both kinds share one number
//! sequence per repository ([`IssueNumber`]), comments and the timeline
//! ([`TimelineEvent`]). Who may do what with a thread is decided purely by
//! [`issue_permissions`] and [`comment_permissions`].
//!
//! The integrator owns this file (see `docs/design/milestone-2b.md`); workstreams request
//! changes through their reports.

use serde::{Deserialize, Serialize};

use crate::{AccountId, PushSource};
#[cfg(test)]
use crate::{RepoAccess, User};

// ------------------------------------------------------------------ ids

pub use rbit_collaboration::threads::IssueId;

pub use rbit_collaboration::comments::CommentId;

pub use rbit_collaboration::timeline::EventId;

pub use rbit_collaboration::labels::LabelId;

pub use rbit_collaboration::milestones::MilestoneId;

pub use rbit_collaboration::pulls::ReviewId;

// ------------------------------------------------------------------ numbers

pub use rbit_collaboration::threads::IssueNumber;

// ------------------------------------------------------------------ enums

pub use rbit_collaboration::threads::IssueKind;

pub use rbit_collaboration::IssueState;

pub use rbit_collaboration::threads::StateReason;

pub use rbit_collaboration::pulls::{MergeStrategy, ReviewState};

// ------------------------------------------------------------------ text

pub use rbit_collaboration::threads::{IssueText, IssueTextError, IssueTitle, MAX_TEXT_BYTES, MAX_TITLE_CHARS};

// ------------------------------------------------------------------ entities

pub use rbit_collaboration::threads::Issue;

pub use rbit_collaboration::comments::{Comment, CommentSource};

pub use rbit_collaboration::labels::LabelSnapshot;

// ------------------------------------------------------------------ timeline

/// Everything the timeline can show besides comments. Stored as `kind` (the variant name
/// in `snake_case`, see [`TimelineEvent::kind`]) plus `data` (the variant's fields as a
/// JSON object); serde's adjacent tagging produces exactly that shape. Unknown kinds are
/// skipped when reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum TimelineEvent {
    // --- core (T0)
    /// `commit`: closed by a pushed commit; `pull`: closed because that PR was merged.
    Closed {
        reason: StateReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        commit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pull: Option<IssueNumber>,
    },
    Reopened,
    Renamed {
        from: String,
        to: String,
    },
    Locked,
    Unlocked,
    // --- issues (I)
    Labeled {
        label: LabelSnapshot,
    },
    Unlabeled {
        label: LabelSnapshot,
    },
    Milestoned {
        milestone: MilestoneId,
        title: String,
    },
    Demilestoned {
        milestone: MilestoneId,
        title: String,
    },
    Assigned {
        assignee: AccountId,
    },
    Unassigned {
        assignee: AccountId,
    },
    /// A commit in this repository mentions the issue ("referenced this in abc1234").
    CommitReferenced {
        commit: String,
        summary: String,
    },
    /// Another issue or PR mentions this one; shown only to viewers who can read the source.
    CrossReferenced {
        source: IssueId,
    },
    // --- pull requests (P)
    HeadPushed {
        before: String,
        after: String,
        count: u32,
        forced: bool,
    },
    BaseChanged {
        from: String,
        to: String,
    },
    /// `strategy: None`: merged outside rbit (the head became part of the base).
    Merged {
        commit: String,
        strategy: Option<MergeStrategy>,
    },
    ReviewRequested {
        reviewer: AccountId,
    },
    /// An automatic review request derived from trusted CODEOWNERS rules.
    CodeOwnerReviewRequested {
        reviewer: AccountId,
    },
    ReviewRequestRemoved {
        reviewer: AccountId,
    },
    ReviewDismissed {
        review: ReviewId,
        reviewer: Option<AccountId>,
    },
    ReviewThreadResolved {
        comment: crate::pull::ReviewCommentId,
    },
    ReviewThreadReopened {
        comment: crate::pull::ReviewCommentId,
    },
    HeadRefDeleted {
        branch: String,
    },
    ConvertedToDraft,
    ReadyForReview,
}

impl TimelineEvent {
    /// The `kind` column: the variant name in `snake_case`.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Closed { .. } => "closed",
            Self::Reopened => "reopened",
            Self::Renamed { .. } => "renamed",
            Self::Locked => "locked",
            Self::Unlocked => "unlocked",
            Self::Labeled { .. } => "labeled",
            Self::Unlabeled { .. } => "unlabeled",
            Self::Milestoned { .. } => "milestoned",
            Self::Demilestoned { .. } => "demilestoned",
            Self::Assigned { .. } => "assigned",
            Self::Unassigned { .. } => "unassigned",
            Self::CommitReferenced { .. } => "commit_referenced",
            Self::CrossReferenced { .. } => "cross_referenced",
            Self::HeadPushed { .. } => "head_pushed",
            Self::BaseChanged { .. } => "base_changed",
            Self::Merged { .. } => "merged",
            Self::ReviewRequested { .. } => "review_requested",
            Self::CodeOwnerReviewRequested { .. } => "code_owner_review_requested",
            Self::ReviewRequestRemoved { .. } => "review_request_removed",
            Self::ReviewDismissed { .. } => "review_dismissed",
            Self::ReviewThreadResolved { .. } => "review_thread_resolved",
            Self::ReviewThreadReopened { .. } => "review_thread_reopened",
            Self::HeadRefDeleted { .. } => "head_ref_deleted",
            Self::ConvertedToDraft => "converted_to_draft",
            Self::ReadyForReview => "ready_for_review",
        }
    }

    /// The account the event names besides its actor (assignee, reviewer), if any.
    pub const fn subject_account(&self) -> Option<AccountId> {
        match self {
            Self::Assigned { assignee } | Self::Unassigned { assignee } => Some(*assignee),
            Self::ReviewRequested { reviewer } | Self::CodeOwnerReviewRequested { reviewer } | Self::ReviewRequestRemoved { reviewer } => {
                Some(*reviewer)
            }
            Self::ReviewDismissed { reviewer, .. } => *reviewer,
            _ => None,
        }
    }
}

// ------------------------------------------------------------------ domain event actions

/// What happened to an issue or pull request (`DomainEvent::Issue`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueAction {
    /// Published by I for issues and by P for pull requests, once their own data is in place.
    Opened,
    /// Only the fields that changed are `Some`, holding their previous value.
    Edited {
        previous_title: Option<String>,
        previous_body: Option<String>,
        previous_base: Option<String>,
    },
    /// `Merged` ⇒ the `pull_request` webhook reports `closed` with `merged: true`.
    Closed {
        reason: StateReason,
    },
    Reopened,
    Locked,
    Unlocked,
    /// The row is gone; the snapshot is what it was.
    Deleted {
        snapshot: Box<Issue>,
    },
    Labeled {
        label: LabelSnapshot,
    },
    Unlabeled {
        label: LabelSnapshot,
    },
    Assigned {
        assignee: AccountId,
    },
    Unassigned {
        assignee: AccountId,
    },
    Milestoned {
        milestone: MilestoneId,
    },
    Demilestoned {
        milestone: MilestoneId,
        title: String,
    },
    /// The head branch of a pull request moved.
    Synchronized {
        before: String,
        after: String,
        /// Where the push that moved it came from. The event's `actor` is only its
        /// account; Actions also needs to know a mirror or task push (§11.2).
        source: PushSource,
    },
    ReviewRequested {
        reviewer: AccountId,
    },
    ReviewRequestRemoved {
        reviewer: AccountId,
    },
    ConvertedToDraft,
    ReadyForReview,
}

/// What happened to a comment (`DomainEvent::IssueComment`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentAction {
    Created,
    Edited {
        previous_body: String,
    },
    /// The row is gone; the snapshot is what it was.
    Deleted {
        snapshot: Comment,
    },
}

/// What happened to a pull request review (`DomainEvent::PullRequestReview`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAction {
    Submitted,
    Edited { previous_body: String },
    Dismissed,
}

// ------------------------------------------------------------------ permissions

pub use rbit_collaboration::threads::{CommentPermissions, IssuePermissions, comment_permissions, issue_permissions};

#[cfg(test)]
mod tests;
