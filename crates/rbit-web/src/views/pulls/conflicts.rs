//! A bounded conflict editor and retained draft after a refused publication.
use super::{Page, PullHeaderView, RepoHeader};
use askama::Template;
use askama_web::WebTemplate;
use rbit_core::pull::conflicts::{Preview, Resolution};
use rbit_localization::ReviewMessage;

#[derive(Debug, Template, WebTemplate)]
#[template(path = "pulls/conflicts.html")]
pub struct ConflictPage {
    pub page: Page,
    pub header: RepoHeader,
    pub pull: PullHeaderView,
    pub preview: Option<Preview>,
    pub notice: Option<ReviewMessage<'static>>,
    pub draft: Vec<Resolution>,
}
