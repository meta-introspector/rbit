//! Safe links and localized relationship labels shared by issue and pull sidebars.
use rbit_localization::IssueSidebarMessage;
use rbit_services::issue_relations::Relations;
#[derive(Debug, Clone)]
pub struct RelationLink {
    pub title: String,
    pub reference: String,
    pub url: String,
    pub remove_url: String,
    pub relationship: IssueSidebarMessage,
    pub open: bool,
    pub can_remove: bool,
}
#[derive(Debug, Clone)]
pub struct RelationView {
    pub data: Relations,
    pub links: Vec<RelationLink>,
    pub action: String,
}
impl RelationView {
    pub fn new(data: Relations, forms: &str) -> Self {
        let action = format!("{forms}/relations");
        let links = data
            .items
            .iter()
            .map(|item| RelationLink {
                title: item.title.clone(),
                reference: format!("{}/{} #{}", item.owner, item.repository, item.number),
                url: format!(
                    "/{}/{}/{}/{}",
                    item.owner,
                    item.repository,
                    if item.issue_kind == "pull" { "pull" } else { "issues" },
                    item.number
                ),
                remove_url: format!("{action}/{}/remove", item.id),
                relationship: match item.relationship.as_str() {
                    "child" => IssueSidebarMessage::Subissue,
                    "parent" => IssueSidebarMessage::ParentIssue,
                    "blocked_by" => IssueSidebarMessage::BlockedBy,
                    _ => IssueSidebarMessage::Blocking,
                },
                open: item.state == "open",
                can_remove: item.can_remove,
            })
            .collect();
        Self { data, links, action }
    }
}
