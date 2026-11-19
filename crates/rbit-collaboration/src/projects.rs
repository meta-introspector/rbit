//! Organization project board records, bounded inputs and completion policy.

mod planning;
pub use planning::{Iteration, IterationInput, Layout, MAX_ITERATIONS, MAX_VIEWS, PlanningInput, Priority, SavedView, Sort, ViewInput};

use serde::{Deserialize, Serialize};

pub const MAX_BOARDS: i64 = 100;
pub const MAX_CARDS: i64 = 10_000;
pub const PAGE_SIZE: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub id: i64,
    pub org_id: i64,
    pub title: String,
    pub columns: Vec<String>,
    pub revision: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub priority: Priority,
    pub target_date: Option<String>,
    pub overdue: bool,
    pub iteration_id: Option<i64>,
    pub progress: crate::issue_relations::Progress,
    pub automatic: bool,
    pub issue_id: i64,
    pub number: i64,
    pub repository: String,
    pub title: String,
    pub kind: String,
    pub state: String,
    pub column: i64,
    pub revision: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filter {
    pub column: Option<i64>,
    pub priority: Option<Priority>,
    /// Zero means unassigned; positive values select one board iteration.
    pub iteration: Option<i64>,
    pub overdue: bool,
    pub layout: Layout,
    pub sort: Sort,
    pub view: Option<i64>,
    pub cursor: String,
    #[serde(default)]
    pub q: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub after: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BoardPage {
    pub iterations: Vec<Iteration>,
    pub views: Vec<SavedView>,
    pub filter: Filter,
    pub next_cursor: Option<String>,
    pub board: Board,
    pub cards: Vec<Card>,
    pub next: Option<i64>,
    pub can_manage: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardInput {
    pub title: String,
    pub columns: Vec<String>,
}

impl BoardInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.title.trim().is_empty() || self.title.len() > 120 || self.title.chars().any(char::is_control) {
            return Err("board title must contain 1–120 bytes without control characters");
        }
        if !(2..=12).contains(&self.columns.len()) {
            return Err("a board needs 2–12 columns; the last column is completed work");
        }
        for (index, column) in self.columns.iter().enumerate() {
            if column.trim().is_empty() || column.len() > 60 || column.chars().any(char::is_control) {
                return Err("column names must contain 1–60 bytes without control characters");
            }
            if self.columns[..index].iter().any(|previous| previous.trim().eq_ignore_ascii_case(column.trim())) {
                return Err("column names must be unique");
            }
        }
        Ok(())
    }
}

impl Filter {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.q.len() > 200
            || self.repository.len() > 100
            || self.after < 0
            || self.cursor.len() > 2048
            || self.column.is_some_and(|v| !(0..12).contains(&v))
            || self.iteration.is_some_and(|v| v < 0)
            || self.view.is_some_and(|v| v < 1)
            || (self.after != 0 && (self.sort != Sort::Id || !self.cursor.is_empty()))
            || self.q.chars().chain(self.repository.chars()).any(char::is_control)
        {
            return Err("invalid project filter");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_and_unique_column_names() {
        let mut input = BoardInput { title: "Release".into(), columns: vec!["Todo".into(), "Done".into()] };
        assert!(input.validate().is_ok());
        input.columns.push(" todo ".into());
        assert!(input.validate().is_err());
        input.columns = vec!["Only".into()];
        assert!(input.validate().is_err());
        input.title = "bad\nname".into();
        assert!(input.validate().is_err());
        assert!(Filter { after: -1, ..Filter::default() }.validate().is_err());
    }
}
