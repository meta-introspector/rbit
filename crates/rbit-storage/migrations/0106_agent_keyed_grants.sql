-- Preserve live delegation semantics while keeping final dispatch publication
-- bounded by agent and repository keys under tiny or large planner statistics.
-- Listing grants retain the original aggregate view from migration0103.
DROP TRIGGER agent_manual_run_admission;
-- Manual run publication checks delegation in the same write transaction as the run.
-- A capability or org scope revoked while a workflow was being planned cannot admit it.
CREATE TRIGGER agent_manual_run_admission BEFORE INSERT ON actions_runs
WHEN NEW.event IN ('workflow_dispatch', 'repository_dispatch')
 AND EXISTS(SELECT 1 FROM user_logins WHERE account_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND is_agent = 1)
BEGIN
    SELECT CASE WHEN NOT EXISTS(
        SELECT 1 FROM agent_accounts a
        WHERE a.account_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND (SELECT (SELECT (SELECT NULLIF(MIN(
    (SELECT MAX(s.permission) FROM agent_scopes s
     WHERE s.agent_id = a.account_id AND (s.repo_id = keyed.id OR s.org_id = keyed.owner_id)
       AND (s.org_id IS NULL OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = s.org_id AND m.user_id = a.owner_id))),
    CASE
             WHEN u.is_admin = 1 OR keyed.owner_id = u.account_id THEN 2
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = keyed.owner_id AND m.user_id = u.account_id AND m.role = 'owner') THEN 2
             WHEN EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = keyed.id AND c.user_id = u.account_id AND c.permission IN ('write','admin')) THEN 2
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = keyed.owner_id AND m.user_id = u.account_id
                  AND (COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') = 'write'
                  OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id
                            WHERE t.org_id = keyed.owner_id AND tm.user_id = u.account_id AND tr.repo_id = keyed.id AND tr.permission IN ('write','admin')))) THEN 2
             WHEN keyed.visibility = 'public' THEN 1
             WHEN EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = keyed.id AND c.user_id = u.account_id) THEN 1
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = keyed.owner_id AND m.user_id = u.account_id
                  AND (COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') <> 'none'
                  OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id
                            WHERE t.org_id = keyed.owner_id AND tm.user_id = u.account_id AND tr.repo_id = keyed.id))) THEN 1
             ELSE 0 END), 0)
    FROM user_logins u WHERE u.account_id = a.owner_id
      AND u.is_agent = 0 AND u.suspended_at IS NULL AND u.password_change_required = 0
      AND NOT EXISTS(SELECT 1 FROM credential_revocations pending WHERE pending.account_id = u.account_id
                     AND pending.target_epoch = u.authentication_epoch AND pending.completed_at IS NULL))
    FROM agent_accounts a WHERE a.account_id = COALESCE(NEW.trigger_actor_id, NEW.actor_id) AND a.enabled = 1
      AND (SELECT bot.is_agent = 1 AND bot.suspended_at IS NULL FROM user_logins bot WHERE bot.account_id = a.account_id))
    FROM repositories keyed WHERE keyed.id = NEW.repo_id
      AND NOT EXISTS(SELECT 1 FROM org_agent_policies p WHERE p.org_id = keyed.owner_id AND p.allow_agents = 0)) = 2
          AND EXISTS(SELECT 1 FROM json_each(a.capabilities_json) capability WHERE capability.value = 'dispatch')
    ) THEN RAISE(ABORT, 'agent dispatch delegation revoked') END;
END;
