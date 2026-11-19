//! Recheck delegated write authority at publication and asynchronous admission boundaries.

use rbit_core::{RepoId, User};
use rbit_identity_models::agents::{AgentAccess, Capability};

use crate::{ServiceContext, ServiceError};

/// Ordinary actors follow their existing authorization path. An agent's live owner,
/// organization policy, scope ceiling, capability and MFA must still authorize a write.
pub(crate) async fn require_capability(
    ctx: &ServiceContext,
    actor: &User,
    repo: RepoId,
    capability: Capability,
) -> Result<(), ServiceError> {
    if !actor.is_agent {
        return Ok(());
    }
    let store = ctx.database.agent_accounts();
    let agent = store.active(actor.id()).await.map_err(super::map)?.ok_or(ServiceError::Forbidden)?;
    if actor.agent_owner_id != Some(agent.owner_id)
        || !agent.capabilities.contains(&capability)
        || store.repository_permission(actor.id(), repo).await.map_err(super::map)? != Some(AgentAccess::Write)
        || ctx.authentication_policy.needs_enrollment(agent.owner_id).await?
    {
        return Err(ServiceError::Forbidden);
    }
    Ok(())
}
