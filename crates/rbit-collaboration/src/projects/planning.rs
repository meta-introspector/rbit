//! Project-owned planning fields and bounded, reusable view criteria.
use serde::{Deserialize, Serialize};

use super::Filter;
use crate::milestones::is_calendar_day;

pub const MAX_ITERATIONS: i64 = 200;
pub const MAX_VIEWS: i64 = 50;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    #[default]
    None,
    Urgent,
    High,
    Normal,
    Low,
}
impl Priority {
    pub const fn code(self) -> i64 {
        match self {
            Self::None => 0,
            Self::Urgent => 1,
            Self::High => 2,
            Self::Normal => 3,
            Self::Low => 4,
        }
    }
    pub fn from_code(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Urgent),
            2 => Some(Self::High),
            3 => Some(Self::Normal),
            4 => Some(Self::Low),
            _ => None,
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Urgent => "urgent",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    #[default]
    Board,
    Table,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sort {
    #[default]
    Id,
    Priority,
    TargetDate,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Iteration {
    pub id: i64,
    pub title: String,
    pub starts_on: String,
    pub ends_on: String,
    pub archived: bool,
    pub revision: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationInput {
    pub title: String,
    pub starts_on: String,
    pub ends_on: String,
    #[serde(default)]
    pub archived: bool,
}
fn title(value: &str) -> Result<(), &'static str> {
    if value.trim().is_empty() || value.len() > 100 || value.chars().any(char::is_control) {
        return Err("title must contain 1–100 bytes without control characters");
    }
    Ok(())
}
impl IterationInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        title(&self.title)?;
        if !is_calendar_day(&self.starts_on) || !is_calendar_day(&self.ends_on) || self.ends_on < self.starts_on {
            return Err("iteration dates must be YYYY-MM-DD with the end on or after the start");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanningInput {
    pub revision: i64,
    pub priority: Priority,
    pub iteration_id: Option<i64>,
    pub target_date: Option<String>,
}
impl PlanningInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.revision < 1 || self.iteration_id.is_some_and(|id| id < 1) {
            return Err("invalid planning revision or iteration");
        }
        if self.target_date.as_deref().is_some_and(|day| !is_calendar_day(day)) {
            return Err("target date must be YYYY-MM-DD or null");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewInput {
    pub title: String,
    #[serde(default)]
    pub shared: bool,
    pub filter: Filter,
}
impl ViewInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        title(&self.title)?;
        self.filter.validate()?;
        if self.filter.after != 0 || !self.filter.cursor.is_empty() || self.filter.view.is_some() {
            return Err("saved views contain criteria, not pagination or another view");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedView {
    pub id: i64,
    pub title: String,
    pub shared: bool,
    pub revision: i64,
    pub filter: Filter,
    pub can_edit: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_dates_priorities_and_saved_views_are_bounded() {
        let mut value =
            IterationInput { title: "Sprint".into(), starts_on: "2028-02-29".into(), ends_on: "2028-03-12".into(), archived: false };
        assert!(value.validate().is_ok());
        value.starts_on = "2027-02-29".into();
        assert!(value.validate().is_err());
        value.starts_on = "2028-03-13".into();
        assert!(value.validate().is_err());
        assert!(Priority::from_code(5).is_none());
        assert!(ViewInput { title: "View".into(), shared: false, filter: Filter { after: 2, ..Filter::default() } }.validate().is_err());
        assert!(
            PlanningInput { revision: 1, priority: Priority::Urgent, iteration_id: None, target_date: Some("2026-04-31".into()) }
                .validate()
                .is_err()
        );
    }
}
