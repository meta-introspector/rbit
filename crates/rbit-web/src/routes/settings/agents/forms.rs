//! Bounded friendly names resolve through current repository and organization services.
use crate::{AppState, views::settings::agents::AgentForm};
use rbit_core::User;
use rbit_identity_models::agents::{AgentAccess, AgentInput, AgentScope, MAX_SCOPES};
use rbit_localization::{Locale, settings::agents::AgentsMessage};
use rbit_services::ServiceError;

fn invalid(locale: Locale) -> ServiceError {
    ServiceError::Invalid(AgentsMessage::InvalidScopes.text(&locale).to_string())
}
impl AgentForm {
    pub async fn input(&self, state: &AppState, user: &User, locale: Locale) -> Result<AgentInput, ServiceError> {
        let fields = [
            (&self.read_repositories, false, AgentAccess::Read),
            (&self.write_repositories, false, AgentAccess::Write),
            (&self.read_organizations, true, AgentAccess::Read),
            (&self.write_organizations, true, AgentAccess::Write),
        ];
        if fields.iter().any(|(text, _, _)| text.len() > 8192) {
            return Err(invalid(locale));
        }
        let names: Vec<_> = fields
            .iter()
            .flat_map(|(text, org, access)| {
                text.lines().map(str::trim).filter(|line| !line.is_empty()).map(move |line| (line, *org, *access))
            })
            .collect();
        if names.is_empty() || names.len() > MAX_SCOPES {
            return Err(invalid(locale));
        }
        let mut scopes = Vec::with_capacity(names.len());
        for (name, organization, access) in names {
            if organization {
                let ctx = state.services.orgs.open(Some(user), name).await?;
                if !ctx.role.is_some_and(rbit_core::OrgRole::is_owner) {
                    return Err(ServiceError::NotFound);
                }
                scopes.push(AgentScope::Organization { id: ctx.org.account.id.0, access });
            } else {
                let (owner, repo) = name
                    .split_once('/')
                    .filter(|(owner, repo)| !owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
                    .ok_or_else(|| invalid(locale))?;
                let ctx = state.services.repositories.open(Some(user), owner, repo).await?;
                scopes.push(AgentScope::Repository { id: ctx.repo.repo.id.0, access });
            }
        }
        let input =
            AgentInput { slug: self.slug.trim().to_owned(), name: self.name.trim().to_owned(), capabilities: self.capabilities(), scopes };
        input.validate().map_err(|message| ServiceError::Invalid(message.into()))?;
        Ok(input)
    }
}

#[cfg(test)]
mod tests {
    use crate::views::settings::agents::AgentForm;
    use rbit_identity_models::agents::Capability;
    #[test]
    fn agent_form_capabilities_are_explicit_and_always_include_read() {
        let form = AgentForm { issues: true, push: true, ..AgentForm::blank() };
        assert_eq!(form.capabilities(), [Capability::Read, Capability::Push, Capability::Issues]);
        assert!(!form.capabilities().contains(&Capability::Merge));
    }
}
