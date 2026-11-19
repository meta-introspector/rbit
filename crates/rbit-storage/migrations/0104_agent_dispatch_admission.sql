-- Manual run publication checks delegation in the same write transaction as the run.
-- A capability or org scope revoked while a workflow was being planned cannot admit it.
CREATE TRIGGER agent_manual_run_admission BEFORE INSERT ON actions_runs
WHEN NEW.event IN ('workflow_dispatch', 'repository_dispatch')
 AND EXISTS(SELECT 1 FROM user_logins WHERE account_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND is_agent = 1)
BEGIN
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM agent_repository_grants g JOIN agent_accounts a ON a.account_id = g.agent_id
        WHERE g.agent_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND g.repo_id = NEW.repo_id AND g.permission = 2
          AND EXISTS(SELECT 1 FROM json_each(a.capabilities_json) capability WHERE capability.value = 'dispatch')
    ) THEN RAISE(ABORT, 'agent dispatch delegation revoked') END;
END;
