//! Project board service adapter; domain inputs and transactional authority live below.

use crate::{ServiceContext, ServiceError};
pub use rbit_collaboration::projects::{
    Board, BoardInput, BoardPage, Card, Filter, Iteration, IterationInput, Layout, PlanningInput, Priority, SavedView, Sort, ViewInput,
};
use rbit_core::User;
use rbit_storage::sqlite::projects::{ProjectError, ProjectStore};

#[derive(Debug, Clone)]
pub struct ProjectService {
    store: ProjectStore,
}

fn error(error: ProjectError) -> ServiceError {
    match error {
        ProjectError::NotFound => ServiceError::NotFound,
        ProjectError::Forbidden => ServiceError::Forbidden,
        ProjectError::Conflict => ServiceError::Conflict("project revision"),
        ProjectError::Invalid(message) => ServiceError::Invalid(message.to_owned()),
        ProjectError::Storage(error) => error.into(),
    }
}

impl ProjectService {
    pub(crate) fn new(ctx: &ServiceContext) -> Self {
        Self { store: ctx.database.projects() }
    }
    pub async fn list(&self, user: &User, org: i64) -> Result<Vec<Board>, ServiceError> {
        self.store.list(org, user.id().0).await.map_err(error)
    }
    pub async fn create(&self, user: &User, org: i64, input: &BoardInput) -> Result<Board, ServiceError> {
        self.store.create(org, user.id().0, input).await.map_err(error)
    }
    pub async fn page(&self, user: &User, org: i64, id: i64, filter: &Filter) -> Result<BoardPage, ServiceError> {
        self.store.page(org, user.id().0, id, filter).await.map_err(error)
    }
    pub async fn update(&self, user: &User, org: i64, id: i64, revision: i64, input: &BoardInput) -> Result<(), ServiceError> {
        self.store.update(org, user.id().0, id, revision, input).await.map_err(error)
    }
    pub async fn delete(&self, user: &User, org: i64, id: i64, revision: i64) -> Result<(), ServiceError> {
        self.store.delete(org, user.id().0, id, revision).await.map_err(error)
    }
    pub async fn add_named(&self, user: &User, org: i64, id: i64, repository: &str, number: i64) -> Result<(), ServiceError> {
        let issue = self.store.resolve_issue(org, user.id().0, repository, number).await.map_err(error)?;
        self.add(user, org, id, issue).await
    }
    pub async fn add(&self, user: &User, org: i64, id: i64, issue: i64) -> Result<(), ServiceError> {
        self.store.add(org, user.id().0, id, issue).await.map_err(error)
    }
    pub async fn move_card(
        &self,
        user: &User,
        org: i64,
        id: i64,
        issue: i64,
        revision: i64,
        column: Option<i64>,
    ) -> Result<(), ServiceError> {
        self.store.move_card(org, user.id().0, id, issue, revision, column).await.map_err(error)
    }
    pub async fn plan_card(&self, user: &User, org: i64, id: i64, issue: i64, input: &PlanningInput) -> Result<(), ServiceError> {
        self.store.plan_card(org, user.id().0, id, issue, input).await.map_err(error)
    }
    pub async fn iterations(&self, user: &User, org: i64, id: i64) -> Result<Vec<Iteration>, ServiceError> {
        self.store.iterations(org, user.id().0, id).await.map_err(error)
    }
    pub async fn save_iteration(
        &self,
        user: &User,
        org: i64,
        id: i64,
        existing: Option<(i64, i64)>,
        input: &IterationInput,
    ) -> Result<i64, ServiceError> {
        self.store.save_iteration(org, user.id().0, id, existing, input).await.map_err(error)
    }
    pub async fn delete_iteration(&self, user: &User, org: i64, id: i64, iteration: i64, revision: i64) -> Result<(), ServiceError> {
        self.store.delete_iteration(org, user.id().0, id, iteration, revision).await.map_err(error)
    }
    pub async fn views(&self, user: &User, org: i64, id: i64) -> Result<Vec<SavedView>, ServiceError> {
        self.store.views(org, user.id().0, id).await.map_err(error)
    }
    pub async fn save_view(
        &self,
        user: &User,
        org: i64,
        id: i64,
        existing: Option<(i64, i64)>,
        input: &ViewInput,
    ) -> Result<i64, ServiceError> {
        self.store.save_view(org, user.id().0, id, existing, input).await.map_err(error)
    }
    pub async fn delete_view(&self, user: &User, org: i64, id: i64, view: i64, revision: i64) -> Result<(), ServiceError> {
        self.store.delete_view(org, user.id().0, id, view, revision).await.map_err(error)
    }
}
