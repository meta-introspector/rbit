//! Issue relationship adapter; the store rechecks both endpoints atomically.
use crate::{ServiceContext, ServiceError};
pub use rbit_collaboration::issue_relations::{Progress, RelationInput, RelationKind, Relations};
use rbit_core::{IssueId, User};
use rbit_storage::sqlite::issue_relations::{RelationError, RelationStore};
#[derive(Debug, Clone)]
pub struct RelationService {
    store: RelationStore,
}
fn error(error: RelationError) -> ServiceError {
    match error {
        RelationError::NotFound => ServiceError::NotFound,
        RelationError::Forbidden => ServiceError::Forbidden,
        RelationError::Conflict => ServiceError::Conflict("relationship"),
        RelationError::Invalid(message) => ServiceError::Invalid(message.to_owned()),
        RelationError::Storage(error) => error.into(),
    }
}
impl RelationService {
    pub(crate) fn new(ctx: &ServiceContext) -> Self {
        Self { store: ctx.database.issue_relations() }
    }
    pub async fn get(&self, viewer: Option<&User>, issue: IssueId) -> Result<Relations, ServiceError> {
        self.store.get(viewer.map(|user| user.id().0), issue.0).await.map_err(error)
    }
    pub async fn add(&self, user: &User, issue: IssueId, input: &RelationInput) -> Result<i64, ServiceError> {
        self.store.add(user.id().0, issue.0, input).await.map_err(error)
    }
    pub async fn remove(&self, user: &User, issue: IssueId, edge: i64) -> Result<(), ServiceError> {
        self.store.remove(user.id().0, issue.0, edge).await.map_err(error)
    }
}
