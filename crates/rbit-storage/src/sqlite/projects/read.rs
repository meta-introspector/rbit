//! One bounded visible-card query and relation batch serve both board and table layouts.
use super::{ProjectError, ProjectStore, Result, StorageError, authority, board, iterations, readable_by, views};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rbit_collaboration::projects::{BoardPage, Card, Filter, PAGE_SIZE, Priority, Sort};
use serde::{Deserialize, Serialize};

#[derive(sqlx::FromRow)]
struct Row {
    automatic: bool,
    issue_id: i64,
    number: i64,
    repository: String,
    title: String,
    kind: String,
    state: String,
    column: i64,
    revision: i64,
    priority: i64,
    target_date: Option<String>,
    iteration_id: Option<i64>,
    overdue: bool,
    sort_key: String,
}
impl TryFrom<Row> for Card {
    type Error = ProjectError;
    fn try_from(r: Row) -> Result<Self> {
        Ok(Self {
            progress: rbit_collaboration::issue_relations::Progress::default(),
            automatic: r.automatic,
            issue_id: r.issue_id,
            number: r.number,
            repository: r.repository,
            title: r.title,
            kind: r.kind,
            state: r.state,
            column: r.column,
            revision: r.revision,
            priority: Priority::from_code(r.priority).ok_or_else(|| StorageError::Corrupt("invalid project priority".into()))?,
            target_date: r.target_date,
            iteration_id: r.iteration_id,
            overdue: r.overdue,
        })
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    board: i64,
    criteria: Filter,
    issue: i64,
    key: String,
}
fn criteria(filter: &Filter) -> Filter {
    let mut value = filter.clone();
    value.after = 0;
    value.cursor.clear();
    value
}
fn cursor(id: i64, filter: &Filter) -> Result<(String, i64)> {
    if filter.cursor.is_empty() {
        return Ok((String::new(), filter.after));
    }
    let invalid = || ProjectError::Invalid("invalid project cursor; restart from the first page");
    let bytes = URL_SAFE_NO_PAD.decode(&filter.cursor).map_err(|_| invalid())?;
    let value: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let valid_key = match filter.sort {
        Sort::Id => value.key.is_empty(),
        Sort::Priority => matches!(value.key.as_str(), "1" | "2" | "3" | "4" | "5"),
        Sort::TargetDate => value.key == "9999-99-99" || rbit_collaboration::milestones::is_calendar_day(&value.key),
    };
    if value.board != id || value.criteria != criteria(filter) || value.issue < 1 || !valid_key {
        return Err(invalid());
    }
    Ok((value.key, value.issue))
}
impl ProjectStore {
    pub async fn page(&self, org: i64, actor: i64, id: i64, request: &Filter) -> Result<BoardPage> {
        request.validate().map_err(ProjectError::Invalid)?;
        let mut tx = self.pool.begin().await?;
        let can_manage = authority(&mut tx, org, actor, false).await?;
        let board = board(&mut tx, org, id).await?;
        let iterations = iterations::list(&mut tx, id).await?;
        let views = views::list(&mut tx, org, actor, id, can_manage).await?;
        let mut filter = request.clone();
        if let Some(view) = request.view {
            // A named view owns its criteria; pagination is the only permitted override.
            let expected = Filter { view: Some(view), after: request.after, cursor: request.cursor.clone(), ..Filter::default() };
            if *request != expected {
                return Err(ProjectError::Invalid("a saved view accepts only pagination overrides"));
            }
            filter = views.iter().find(|v| v.id == view).ok_or(ProjectError::NotFound)?.filter.clone();
            filter.view = Some(view);
            filter.after = request.after;
            filter.cursor.clone_from(&request.cursor);
        }
        filter.validate().map_err(ProjectError::Invalid)?;
        if filter.column.is_some_and(|v| usize::try_from(v).map_or(true, |v| v >= board.columns.len())) {
            return Err(ProjectError::Invalid("unknown column"));
        }
        if filter.iteration.is_some_and(|v| v > 0 && !iterations.iter().any(|it| it.id == v)) {
            return Err(ProjectError::NotFound);
        }
        let (key, after) = cursor(id, &filter)?;
        let sort = match filter.sort {
            Sort::Id => 0,
            Sort::Priority => 1,
            Sort::TargetDate => 2,
        };
        let mut rows: Vec<Row> = sqlx::query_as(concat!(
            "WITH visible AS (SELECT (i.state = 'closed' AND COALESCE(i.state_reason IN ('completed', 'merged'), 0)) AS automatic, i.id AS issue_id, i.number, r.name AS repository, i.title, i.kind, i.state, CASE WHEN i.state = 'closed' AND COALESCE(i.state_reason IN ('completed', 'merged'), 0) THEN json_array_length(b.columns_json) - 1 ELSE c.column_index END AS column, c.revision, COALESCE(p.priority, 0) AS priority, p.target_date, p.iteration_id, COALESCE(i.state = 'open' AND p.target_date < date('now'), 0) AS overdue, CASE ?10 WHEN 1 THEN CAST(CASE WHEN COALESCE(p.priority, 0) = 0 THEN 5 ELSE p.priority END AS TEXT) WHEN 2 THEN COALESCE(p.target_date, '9999-99-99') ELSE '' END AS sort_key FROM project_cards c JOIN project_boards b ON b.id = c.board_id JOIN issues i ON i.id = c.issue_id JOIN repositories r ON r.id = i.repo_id LEFT JOIN project_card_planning p ON p.board_id = c.board_id AND p.issue_id = c.issue_id WHERE b.id = ?3 AND r.owner_id = ?1 AND ",
            readable_by!("?2", "(SELECT is_admin FROM user_logins WHERE account_id = ?2)"),
            ") SELECT * FROM visible WHERE (?4 = '' OR instr(lower(title), lower(?4)) > 0) AND (?5 = '' OR repository = ?5 COLLATE NOCASE) AND (?6 IS NULL OR column = ?6) AND (?7 IS NULL OR priority = ?7) AND (?8 IS NULL OR COALESCE(iteration_id, 0) = ?8) AND (?9 = 0 OR overdue) AND (sort_key > ?11 OR (sort_key = ?11 AND issue_id > ?12)) ORDER BY sort_key, issue_id LIMIT ?13"))
            .bind(org).bind(actor).bind(id).bind(&filter.q).bind(&filter.repository).bind(filter.column)
            .bind(filter.priority.map(Priority::code)).bind(filter.iteration).bind(filter.overdue).bind(sort)
            .bind(key).bind(after).bind(PAGE_SIZE + 1).fetch_all(&mut *tx).await?;
        let more = rows.len() > usize::try_from(PAGE_SIZE).unwrap_or(100);
        if more {
            rows.pop();
        }
        let next = if more && filter.sort == Sort::Id { rows.last().map(|r| r.issue_id) } else { None };
        let next_cursor = if more {
            rows.last()
                .map(|row| {
                    let value = Cursor { board: id, criteria: criteria(&filter), issue: row.issue_id, key: row.sort_key.clone() };
                    serde_json::to_vec(&value).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)).map_err(|e| StorageError::Corrupt(e.to_string()))
                })
                .transpose()?
        } else {
            None
        };
        let mut cards: Vec<Card> = rows.into_iter().map(TryInto::try_into).collect::<Result<_>>()?;
        let ids: Vec<i64> = cards.iter().map(|card| card.issue_id).collect();
        let mut relations = super::super::issue_relations::read::batch(&mut tx, Some(actor), &ids).await.map_err(|error| match error {
            super::super::issue_relations::RelationError::Storage(error) => ProjectError::Storage(error),
            _ => ProjectError::Invalid("invalid relationship batch"),
        })?;
        for card in &mut cards {
            card.progress = relations.remove(&card.issue_id).unwrap_or_default().progress;
        }
        Ok(BoardPage { board, cards, next, next_cursor, can_manage, iterations, views, filter })
    }
}
