//! Session planning forms share transactional mutations with the REST adapter.
use super::{
    AppState, Deserialize, Filter, Form, OrgHeader, OrgTab, Page, Path, Redirect, RevisionForm, Router, SignedIn, State, WebResult, get,
    post,
};
use crate::views::projects::PlanningPage;
use rbit_services::{
    ServiceError,
    projects::{IterationInput, PlanningInput, Priority, ViewInput},
};
use std::collections::HashMap;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/organizations/{owner}/projects/{id}/cards/{issue}/planning", get(edit).post(plan))
        .route("/organizations/{owner}/projects/{id}/iterations", post(create_iteration))
        .route("/organizations/{owner}/projects/{id}/iterations/{iteration}", post(update_iteration))
        .route("/organizations/{owner}/projects/{id}/iterations/{iteration}/delete", post(delete_iteration))
        .route("/organizations/{owner}/projects/{id}/views", post(create_view))
        .route("/organizations/{owner}/projects/{id}/views/{view}", post(update_view))
        .route("/organizations/{owner}/projects/{id}/views/{view}/delete", post(delete_view))
}
fn invalid() -> ServiceError {
    ServiceError::Invalid("invalid project form".into())
}
/// Empty select options mean no filter; typed REST criteria retain their normal schema.
pub(super) fn filter(values: HashMap<String, String>) -> Result<Filter, ServiceError> {
    let mut map = serde_json::Map::new();
    for (key, value) in values {
        if value.is_empty() {
            continue;
        }
        let value = match key.as_str() {
            "column" | "iteration" | "view" | "after" => serde_json::Value::from(value.parse::<i64>().map_err(|_| invalid())?),
            "overdue" => serde_json::Value::from(value.parse::<bool>().map_err(|_| invalid())?),
            _ => serde_json::Value::String(value),
        };
        map.insert(key, value);
    }
    serde_json::from_value(serde_json::Value::Object(map)).map_err(|_| invalid())
}
async fn edit(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    page: Page,
    Path((owner, id, issue)): Path<(String, i64, i64)>,
) -> WebResult<PlanningPage> {
    if issue < 1 {
        return Err(ServiceError::NotFound.into());
    }
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    // A bounded seek reuses the same snapshot, visibility policy and progress projection.
    let data = state.services.projects.page(&user, ctx.org.account.id.0, id, &Filter { after: issue - 1, ..Filter::default() }).await?;
    let card = data.cards.iter().find(|c| c.issue_id == issue).ok_or(ServiceError::NotFound)?;
    Ok(PlanningPage::new(page, OrgHeader::new(&ctx, OrgTab::Projects), &data, card))
}
#[derive(Deserialize)]
struct PlanningForm {
    revision: i64,
    priority: Priority,
    #[serde(default)]
    iteration_id: String,
    #[serde(default)]
    target_date: String,
}
async fn plan(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, issue)): Path<(String, i64, i64)>,
    Form(form): Form<PlanningForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    let iteration_id = if form.iteration_id.is_empty() { None } else { Some(form.iteration_id.parse().map_err(|_| invalid())?) };
    let input = PlanningInput {
        revision: form.revision,
        priority: form.priority,
        iteration_id,
        target_date: if form.target_date.is_empty() { None } else { Some(form.target_date) },
    };
    state.services.projects.plan_card(&user, ctx.org.account.id.0, id, issue, &input).await?;
    Ok(redirect(&owner, id))
}
fn redirect(owner: &str, id: i64) -> Redirect {
    Redirect::to(&format!("/organizations/{owner}/projects/{id}"))
}
#[derive(Deserialize)]
struct IterationForm {
    title: String,
    starts_on: String,
    ends_on: String,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    revision: i64,
}
impl IterationForm {
    fn input(self) -> IterationInput {
        IterationInput { title: self.title, starts_on: self.starts_on, ends_on: self.ends_on, archived: self.archived }
    }
}
async fn create_iteration(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id)): Path<(String, i64)>,
    Form(form): Form<IterationForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.save_iteration(&user, ctx.org.account.id.0, id, None, &form.input()).await?;
    Ok(redirect(&owner, id))
}
async fn update_iteration(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, iteration)): Path<(String, i64, i64)>,
    Form(form): Form<IterationForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.save_iteration(&user, ctx.org.account.id.0, id, Some((iteration, form.revision)), &form.input()).await?;
    Ok(redirect(&owner, id))
}
async fn delete_iteration(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, iteration)): Path<(String, i64, i64)>,
    Form(form): Form<RevisionForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.delete_iteration(&user, ctx.org.account.id.0, id, iteration, form.revision).await?;
    Ok(redirect(&owner, id))
}
#[derive(Deserialize)]
struct ViewForm {
    title: String,
    #[serde(default)]
    shared: bool,
    criteria: String,
    #[serde(default)]
    revision: i64,
}
impl ViewForm {
    fn input(self) -> Result<ViewInput, ServiceError> {
        if self.criteria.len() > 4096 {
            return Err(invalid());
        }
        Ok(ViewInput { title: self.title, shared: self.shared, filter: serde_json::from_str(&self.criteria).map_err(|_| invalid())? })
    }
}
async fn create_view(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id)): Path<(String, i64)>,
    Form(form): Form<ViewForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    let view = state.services.projects.save_view(&user, ctx.org.account.id.0, id, None, &form.input()?).await?;
    Ok(Redirect::to(&format!("/organizations/{owner}/projects/{id}?view={view}")))
}
async fn update_view(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, view)): Path<(String, i64, i64)>,
    Form(form): Form<ViewForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.save_view(&user, ctx.org.account.id.0, id, Some((view, form.revision)), &form.input()?).await?;
    Ok(Redirect::to(&format!("/organizations/{owner}/projects/{id}?view={view}")))
}
async fn delete_view(
    State(state): State<AppState>,
    SignedIn(user): SignedIn,
    Path((owner, id, view)): Path<(String, i64, i64)>,
    Form(form): Form<RevisionForm>,
) -> WebResult<Redirect> {
    let ctx = state.services.orgs.open(Some(&user), &owner).await?;
    state.services.projects.delete_view(&user, ctx.org.account.id.0, id, view, form.revision).await?;
    Ok(redirect(&owner, id))
}
#[cfg(test)]
mod tests {
    use super::{HashMap, filter};
    #[test]
    fn project_form_empty_options_and_typed_values() {
        let values = HashMap::from([
            ("priority".into(), String::new()),
            ("column".into(), "0".into()),
            ("iteration".into(), "0".into()),
            ("overdue".into(), "true".into()),
        ]);
        let parsed = filter(values).unwrap();
        assert_eq!(parsed.column, Some(0));
        assert_eq!(parsed.iteration, Some(0));
        assert_eq!(parsed.priority, None);
        assert!(parsed.overdue);
        assert!(filter(HashMap::from([("iteration".into(), "bad".into())])).is_err());
    }
}
