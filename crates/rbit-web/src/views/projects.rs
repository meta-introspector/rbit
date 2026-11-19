//! Project projections share visible cards across responsive board and table layouts.
use crate::views::{layout::Page, org::OrgHeader};
use askama::Template;
use askama_web::WebTemplate;
use rbit_localization::settings::org::OrgMessage;
use rbit_services::projects::{Board, BoardPage, Card, Filter, Iteration, Layout, Priority};

#[derive(Debug, Template, WebTemplate)]
#[template(path = "projects/list.html")]
pub struct ProjectsPage {
    pub page: Page,
    pub header: OrgHeader,
    pub boards: Vec<Board>,
    pub base: String,
    pub can_manage: bool,
}
#[derive(Debug, Clone)]
pub struct CardView {
    pub progress: rbit_services::issue_relations::Progress,
    pub automatic: bool,
    pub title: String,
    pub label: String,
    pub url: String,
    pub action: String,
    pub revision: i64,
    pub column: i64,
    pub state: String,
    pub priority: OrgMessage,
    pub target_date: String,
    pub overdue: bool,
    pub iteration: String,
    pub column_name: String,
}
impl CardView {
    fn new(owner: &str, base: &str, data: &BoardPage, card: &Card) -> Self {
        Self {
            progress: card.progress.clone(),
            automatic: card.automatic,
            title: card.title.clone(),
            label: format!("{} #{}", card.repository, card.number),
            url: card_url(owner, &card.repository, &card.kind, card.number),
            action: format!("{base}/cards/{}", card.issue_id),
            revision: card.revision,
            column: card.column,
            state: card.state.clone(),
            priority: priority_label(card.priority),
            target_date: card.target_date.clone().unwrap_or_default(),
            overdue: card.overdue,
            iteration: data.iterations.iter().find(|it| Some(it.id) == card.iteration_id).map(|it| it.title.clone()).unwrap_or_default(),
            column_name: usize::try_from(card.column).ok().and_then(|n| data.board.columns.get(n)).cloned().unwrap_or_default(),
        }
    }
}
#[derive(Debug)]
pub struct ColumnView {
    pub index: i64,
    pub name: String,
    pub cards: Vec<CardView>,
}
#[derive(Debug)]
pub struct PriorityOption {
    pub value: &'static str,
    pub label: OrgMessage,
    pub selected: bool,
}
fn priority_label(value: Priority) -> OrgMessage {
    match value {
        Priority::None => OrgMessage::ProjectNoPriority,
        Priority::Urgent => OrgMessage::ProjectUrgent,
        Priority::High => OrgMessage::ProjectHigh,
        Priority::Normal => OrgMessage::ProjectNormal,
        Priority::Low => OrgMessage::ProjectLow,
    }
}
fn priorities(selected: Option<Priority>) -> Vec<PriorityOption> {
    [Priority::None, Priority::Urgent, Priority::High, Priority::Normal, Priority::Low]
        .into_iter()
        .map(|p| PriorityOption { value: p.as_str(), label: priority_label(p), selected: selected == Some(p) })
        .collect()
}

#[derive(Debug, Template, WebTemplate)]
#[template(path = "projects/board.html")]
pub struct ProjectPage {
    pub page: Page,
    pub header: OrgHeader,
    pub data: BoardPage,
    pub columns: Vec<ColumnView>,
    pub cards: Vec<CardView>,
    pub columns_text: String,
    pub base: String,
    pub filter: Filter,
    pub next_url: Option<String>,
    pub priorities: Vec<PriorityOption>,
    pub criteria: String,
    pub table: bool,
}
impl ProjectPage {
    pub fn new(page: Page, header: OrgHeader, data: BoardPage) -> Self {
        let base = format!("/organizations/{}/projects/{}", header.name, data.board.id);
        let filter = data.filter.clone();
        let cards: Vec<CardView> = data.cards.iter().map(|c| CardView::new(&header.name, &base, &data, c)).collect();
        let columns = data
            .board
            .columns
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let index = i64::try_from(index).unwrap_or_default();
                ColumnView { index, name: name.clone(), cards: cards.iter().filter(|c| c.column == index).cloned().collect() }
            })
            .collect();
        let columns_text = data.board.columns.join("\n");
        let next_url = data.next_cursor.as_ref().map(|cursor| {
            let mut next = if let Some(view) = filter.view { Filter { view: Some(view), ..Filter::default() } } else { filter.clone() };
            next.after = 0;
            next.cursor.clone_from(cursor);
            format!("{base}?{}", query(&next))
        });
        let mut saved = filter.clone();
        saved.after = 0;
        saved.cursor.clear();
        saved.view = None;
        let criteria = serde_json::to_string(&saved).unwrap_or_default();
        let priorities = priorities(filter.priority);
        let table = filter.layout == Layout::Table;
        Self {
            page: page.titled(&data.board.title),
            header,
            data,
            columns,
            cards,
            columns_text,
            base,
            filter,
            next_url,
            priorities,
            criteria,
            table,
        }
    }
}
fn query(filter: &Filter) -> String {
    let serde_json::Value::Object(values) = serde_json::to_value(filter).unwrap_or_default() else { return String::new() };
    values
        .into_iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| {
            let value = if let serde_json::Value::String(v) = v { v } else { v.to_string() };
            format!("{k}={}", percent_encoding::utf8_percent_encode(&value, percent_encoding::NON_ALPHANUMERIC))
        })
        .collect::<Vec<_>>()
        .join("&")
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "projects/planning.html")]
pub struct PlanningPage {
    pub page: Page,
    pub header: OrgHeader,
    pub base: String,
    pub card: CardView,
    pub priorities: Vec<PriorityOption>,
    pub iterations: Vec<Iteration>,
    pub iteration_id: Option<i64>,
}
impl PlanningPage {
    pub fn new(page: Page, header: OrgHeader, data: &BoardPage, card: &Card) -> Self {
        let base = format!("/organizations/{}/projects/{}", header.name, data.board.id);
        Self {
            page: page.titled(&card.title),
            card: CardView::new(&header.name, &base, data, card),
            priorities: priorities(Some(card.priority)),
            iterations: data.iterations.iter().filter(|it| !it.archived || Some(it.id) == card.iteration_id).cloned().collect(),
            iteration_id: card.iteration_id,
            header,
            base,
        }
    }
}
/// Issue and pull request page routes use different singular/plural sections.
fn card_url(owner: &str, repository: &str, kind: &str, number: i64) -> String {
    let section = if kind == "pull" { "pull" } else { "issues" };
    format!("/{owner}/{repository}/{section}/{number}")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_cards_link_to_canonical_issue_and_pull_pages() {
        assert_eq!(card_url("acme", "client", "issue", 12), "/acme/client/issues/12");
        assert_eq!(card_url("acme", "client", "pull", 13), "/acme/client/pull/13");
    }
    #[test]
    fn project_cursor_links_escape_and_preserve_criteria() {
        let q = query(&Filter { q: "one&two".into(), cursor: "safe_cursor".into(), layout: Layout::Table, ..Filter::default() });
        assert!(q.contains("q=one%26two"));
        assert!(q.contains("layout=table"));
        assert!(q.contains("cursor=safe%5Fcursor"));
        assert!(!q.contains("priority="));
    }
}
