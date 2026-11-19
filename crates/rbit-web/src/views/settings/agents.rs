//! Human-owned agent settings projections, with secrets confined to one response.
use super::{
    SettingsLink, SettingsSection,
    tokens::{ExpiryChoice, TokenRow},
};
use crate::views::{components::FormError, layout::Page, org::OrgHeader};
use askama::Template;
use askama_web::WebTemplate;
use rbit_core::Timestamp;
use rbit_identity_models::agents::{Agent, AgentAccess, AgentScope, Capability};
use rbit_localization::settings::agents::AgentsMessage;
use rbit_services::agents::AgentCredential;

pub fn capability_message(capability: Capability) -> AgentsMessage {
    match capability {
        Capability::Read => AgentsMessage::Read,
        Capability::Push => AgentsMessage::Push,
        Capability::Issues => AgentsMessage::Issues,
        Capability::PullRequests => AgentsMessage::PullRequests,
        Capability::Statuses => AgentsMessage::Statuses,
        Capability::Dispatch => AgentsMessage::Dispatch,
        Capability::ReadLogs => AgentsMessage::ReadLogs,
        Capability::Merge => AgentsMessage::Merge,
    }
}
#[derive(Debug)]
pub struct AgentRow {
    pub handle: String,
    pub name: String,
    pub enabled: bool,
    pub url: String,
    pub capabilities: Vec<AgentsMessage>,
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "settings/agents.html")]
pub struct AgentsPage {
    pub page: Page,
    pub settings_nav: Vec<SettingsLink>,
    pub agents: Vec<AgentRow>,
}
impl AgentsPage {
    pub fn new(page: Page, agents: Vec<Agent>) -> Self {
        let title = AgentsMessage::Title.text(&page.locale);
        Self {
            page: page.titled(title.as_str()),
            settings_nav: SettingsSection::Agents.nav(),
            agents: agents
                .into_iter()
                .map(|agent| AgentRow {
                    handle: agent.handle,
                    name: agent.name,
                    enabled: agent.enabled,
                    url: format!("/settings/agents/{}", agent.account_id.0),
                    capabilities: agent.capabilities.into_iter().map(capability_message).collect(),
                })
                .collect(),
        }
    }
}
#[derive(Debug, Clone, Default)]
#[expect(clippy::struct_excessive_bools, reason = "one checkbox per independent capability and enabled flag")]
pub struct AgentForm {
    pub slug: String,
    pub name: String,
    pub read_repositories: String,
    pub write_repositories: String,
    pub read_organizations: String,
    pub write_organizations: String,
    pub revision: i64,
    pub enabled: bool,
    pub push: bool,
    pub issues: bool,
    pub pull_requests: bool,
    pub statuses: bool,
    pub dispatch: bool,
    pub read_logs: bool,
    pub merge: bool,
}
#[derive(Debug)]
pub struct CapabilityChoice {
    pub value: &'static str,
    pub label: AgentsMessage,
    pub selected: bool,
    pub fixed: bool,
}
impl AgentForm {
    pub fn blank() -> Self {
        Self { enabled: true, ..Self::default() }
    }
    pub fn capabilities(&self) -> Vec<Capability> {
        let mut selected = vec![Capability::Read];
        for (on, capability) in [
            (self.push, Capability::Push),
            (self.issues, Capability::Issues),
            (self.pull_requests, Capability::PullRequests),
            (self.statuses, Capability::Statuses),
            (self.dispatch, Capability::Dispatch),
            (self.read_logs, Capability::ReadLogs),
            (self.merge, Capability::Merge),
        ] {
            if on {
                selected.push(capability);
            }
        }
        selected
    }
    pub fn choices(&self) -> Vec<CapabilityChoice> {
        let selected = self.capabilities();
        Capability::ALL
            .into_iter()
            .map(|capability| CapabilityChoice {
                value: capability.as_str(),
                label: capability_message(capability),
                selected: selected.contains(&capability),
                fixed: capability == Capability::Read,
            })
            .collect()
    }
    pub fn stored(agent: &Agent, scopes: &[(AgentScope, Option<String>)]) -> Self {
        let has = |capability| agent.capabilities.contains(&capability);
        let mut form = Self {
            slug: agent.slug.clone(),
            name: agent.name.clone(),
            revision: agent.revision,
            enabled: agent.enabled,
            push: has(Capability::Push),
            issues: has(Capability::Issues),
            pull_requests: has(Capability::PullRequests),
            statuses: has(Capability::Statuses),
            dispatch: has(Capability::Dispatch),
            read_logs: has(Capability::ReadLogs),
            merge: has(Capability::Merge),
            ..Self::default()
        };
        for (scope, name) in scopes {
            if let Some(name) = name {
                let field = match (scope, scope.access()) {
                    (AgentScope::Repository { .. }, AgentAccess::Read) => &mut form.read_repositories,
                    (AgentScope::Repository { .. }, AgentAccess::Write) => &mut form.write_repositories,
                    (AgentScope::Organization { .. }, AgentAccess::Read) => &mut form.read_organizations,
                    (AgentScope::Organization { .. }, AgentAccess::Write) => &mut form.write_organizations,
                };
                if !field.is_empty() {
                    field.push('\n');
                }
                field.push_str(name);
            }
        }
        form
    }
}
pub struct CreatedCredential {
    pub secret: String,
}
impl std::fmt::Debug for CreatedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedCredential").field("secret", &"<redacted>").finish()
    }
}
#[derive(Debug)]
pub struct CredentialRow {
    pub id: i64,
    pub token: TokenRow,
    pub rotate_url: String,
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "settings/agent.html")]
pub struct AgentPage {
    pub page: Page,
    pub settings_nav: Vec<SettingsLink>,
    pub form: AgentForm,
    pub choices: Vec<CapabilityChoice>,
    pub error: FormError,
    pub action: String,
    pub handle: String,
    pub is_new: bool,
    pub unavailable: bool,
    pub credentials: Vec<CredentialRow>,
    pub created: Option<CreatedCredential>,
    pub expiry_choices: Vec<ExpiryChoice>,
    pub credential_name: String,
    pub credential_write: bool,
}
impl AgentPage {
    pub fn show_created(&mut self, credential: &AgentCredential, secret: String, replaced: Option<i64>) {
        self.credentials.retain(|row| Some(row.id) != replaced);
        let token = &credential.token;
        let mut row = TokenRow::new(token, Timestamp::now(), self.page.locale);
        row.revoke_url = format!("{}/credentials/{}/revoke", self.action, credential.id);
        self.credentials.insert(
            0,
            CredentialRow { id: credential.id, token: row, rotate_url: format!("{}/credentials/{}/rotate", self.action, credential.id) },
        );
        self.created = Some(CreatedCredential { secret });
        self.credential_name.clear();
    }
    pub fn new(
        page: Page,
        agent: Option<&Agent>,
        form: AgentForm,
        scopes: &[(AgentScope, Option<String>)],
        tokens: &[AgentCredential],
    ) -> Self {
        let title = if agent.is_some() { AgentsMessage::Edit } else { AgentsMessage::New }.text(&page.locale);
        let base = agent.map_or_else(|| "/settings/agents".into(), |agent| format!("/settings/agents/{}", agent.account_id.0));
        let now = Timestamp::now();
        Self {
            credentials: tokens
                .iter()
                .map(|credential| {
                    let token = &credential.token;
                    let mut row = TokenRow::new(token, now, page.locale);
                    row.revoke_url = format!("{base}/credentials/{}/revoke", credential.id);
                    CredentialRow { id: credential.id, token: row, rotate_url: format!("{base}/credentials/{}/rotate", credential.id) }
                })
                .collect(),
            page: page.titled(title.as_str()),
            settings_nav: SettingsSection::Agents.nav(),
            choices: form.choices(),
            form,
            error: FormError::default(),
            action: base,
            handle: agent.map_or_else(String::new, |agent| agent.handle.clone()),
            is_new: agent.is_none(),
            unavailable: scopes.iter().any(|(_, name)| name.is_none()),
            created: None,
            credential_name: String::new(),
            credential_write: true,
            expiry_choices: ExpiryChoice::list(rbit_services::AccessTokenService::EXPIRY_CHOICES, Some(30)),
        }
    }
}
#[derive(Debug, Template, WebTemplate)]
#[template(path = "org/agents.html")]
pub struct AgentPolicyPage {
    pub page: Page,
    pub header: OrgHeader,
    pub action_url: String,
    pub allow: bool,
    pub error: FormError,
}

#[cfg(test)]
mod tests {
    use super::CreatedCredential;
    #[test]
    fn one_time_agent_credential_debug_never_contains_the_secret() {
        let created = CreatedCredential { secret: "private-credential-material".into() };
        let debug = format!("{created:?}");
        assert!(!debug.contains(&created.secret));
        assert!(debug.contains("<redacted>"));
    }
}
