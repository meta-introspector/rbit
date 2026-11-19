//! Keyed effective access must never enumerate an agent's organization repositories.
use super::REPOSITORY_PERMISSION;
use crate::sqlite::plans;

const CORRELATED_PERMISSION: &str = concat!(
    "SELECT COALESCE(",
    crate::sqlite::agent_accounts::queries::repository_permission_sql!("?1", "r.id"),
    ", 0) > 0 FROM repositories r WHERE r.id = ?2"
);

#[tokio::test]
async fn repository_permission_pushes_both_keys_inside_scope_lookup() {
    let (_directory, database) = plans::database().await;
    // A SELECT of a scalar expression has one synthetic row, not a table read.
    // The correlated repository caller must not need even this allowance.
    for (sql, allowed) in [(REPOSITORY_PERMISSION, &["SCAN CONSTANT ROW"][..]), (CORRELATED_PERMISSION, &[][..])] {
        plans::assert_no_scans(&database, sql, allowed).await;
        for stats in plans::STATS {
            let plan = plans::query_plan_with(&database, stats, sql).await;
            assert_eq!(plan.iter().filter(|detail| *detail == "SCAN CONSTANT ROW").count(), allowed.len(), "{stats:?}: {plan:#?}");
            for index in [
                "SEARCH s USING INDEX agent_scopes_repository (agent_id=? AND repo_id=?)",
                "SEARCH s USING INDEX agent_scopes_organization (agent_id=? AND org_id=?)",
                "SEARCH keyed USING INTEGER PRIMARY KEY (rowid=?)",
                "SEARCH a USING INTEGER PRIMARY KEY (rowid=?)",
                "SEARCH u USING INTEGER PRIMARY KEY (rowid=?)",
                "SEARCH bot USING INTEGER PRIMARY KEY (rowid=?)",
            ] {
                assert!(plan.iter().any(|detail| detail == index), "{stats:?}: {plan:#?}");
            }
            assert!(!plan.iter().any(|detail| detail.contains("CO-ROUTINE")), "{stats:?}: {plan:#?}");
        }
    }
}

#[test]
fn final_manual_run_admission_uses_the_canonical_keyed_permission() {
    let migration = include_str!("../../../../migrations/0106_agent_keyed_grants.sql");
    let publication = migration
        .split_once("WHERE a.account_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND ")
        .unwrap()
        .1
        .split_once(" = 2\n          AND EXISTS")
        .unwrap()
        .0;
    assert_eq!(
        publication,
        crate::sqlite::agent_accounts::queries::repository_permission_sql!("COALESCE(NEW.trigger_actor_id, NEW.actor_id)", "NEW.repo_id")
    );
}
