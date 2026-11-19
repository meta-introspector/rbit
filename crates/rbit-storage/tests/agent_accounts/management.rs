//! Stable credential handles, transactional rotation and permission-safe scope labels.
mod identifiers;
mod migration;
mod rotation;
mod scope_names;

use super::{AgentAccess, AgentScope, Database, User, Visibility, input, repo, user};
use rbit_identity_models::agents::Agent;
struct Fixture {
    _directory: tempfile::TempDir,
    db: Database,
    owner: User,
    other: User,
    agent: Agent,
}
impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let db = Database::open(&directory.path().join("db")).await.unwrap();
        let owner = user(&db, "owner").await;
        let other = user(&db, "other").await;
        let repository = repo(&db, owner.id(), "repo", Visibility::Private).await;
        let agent = db
            .agent_accounts()
            .create(owner.id(), &input(AgentScope::Repository { id: repository.0, access: AgentAccess::Write }))
            .await
            .unwrap();
        Self { _directory: directory, db, owner, other, agent }
    }
}
