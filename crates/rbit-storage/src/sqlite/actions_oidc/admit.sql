SELECT t.id AS task_id, j.id AS job_id, ar.id AS run_id, ar.run_number, ar.attempt,
       r.id AS repo_id, r.owner_id, ri.uid AS repository_uid, oi.uid AS owner_uid,
       r.name AS repository, o.name AS owner, ar.actor_id, ar.trigger_actor_id,
       ar.ref AS reference, ar.sha, ar.event, ar.workflow_path,
       env.name AS environment, env.branch_policy AS environment_policy,
       env.branch_patterns AS environment_patterns, j.payload, j.permissions, t.started_at
FROM actions_tasks t
JOIN actions_jobs j ON j.id = t.job_id AND j.task_id = t.id
JOIN actions_runs ar ON ar.id = j.run_id
JOIN actions_runners runner ON runner.id = t.runner_id
JOIN repositories r ON r.id = ar.repo_id
JOIN accounts o ON o.id = r.owner_id
JOIN actions_oidc_repositories ri ON ri.repo_id = r.id AND ri.uid = ar.oidc_repository_uid
JOIN actions_oidc_accounts oi ON oi.account_id = r.owner_id AND oi.uid = ar.oidc_owner_uid
JOIN actions_repositories settings ON settings.repo_id = r.id AND settings.enabled = 1
LEFT JOIN actions_owner_settings policies ON policies.owner_id = r.owner_id
LEFT JOIN actions_environments env ON env.repo_id = r.id AND env.name = j.environment
WHERE t.token_digest = ?1 AND t.status = 6 AND j.status = 6 AND ar.status = 6 AND ar.cancel_requested = 0
  AND t.heartbeat_at >= ?2 AND runner.disabled = 0
  AND ar.trusted = 1 AND ar.is_fork = 0 AND j.permissions IS NOT NULL
  AND ar.event <> 'pull_request'
  AND (j.environment IS NULL OR (env.id IS NOT NULL AND (
      NOT EXISTS(SELECT 1 FROM actions_environment_reviewers er WHERE er.environment_id = env.id)
      OR (j.approved_attempt = j.attempt AND EXISTS(
          SELECT 1 FROM actions_deployment_reviews review
          JOIN actions_environment_reviewers er ON er.account_id = review.reviewer_id AND er.environment_id = env.id
          JOIN user_logins reviewer ON reviewer.account_id = review.reviewer_id AND reviewer.suspended_at IS NULL
          WHERE review.job_id = j.id AND review.attempt = j.attempt AND review.decision = 'approved'
            AND (env.prevent_self_review = 0 OR ((ar.actor_id IS NULL OR review.reviewer_id <> ar.actor_id)
                AND (ar.trigger_actor_id IS NULL OR review.reviewer_id <> ar.trigger_actor_id)))
            AND (r.visibility = 'public' OR reviewer.is_admin OR r.owner_id = reviewer.account_id
                OR EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = reviewer.account_id)
                OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = reviewer.account_id
                    AND (m.role = 'owner' OR COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') <> 'none'
                        OR EXISTS(SELECT 1 FROM teams team JOIN team_members tm ON tm.team_id = team.id
                            JOIN team_repositories tr ON tr.team_id = team.id WHERE team.org_id = r.owner_id
                            AND tm.user_id = reviewer.account_id AND tr.repo_id = r.id)))))))))
  AND NOT EXISTS(SELECT 1 FROM repo_archives a WHERE a.repo_id = r.id)
  AND CASE COALESCE(policies.enabled_repositories, 'all')
      WHEN 'all' THEN 1
      WHEN 'selected' THEN EXISTS(SELECT 1 FROM actions_owner_enabled_repositories e WHERE e.owner_id = r.owner_id AND e.repo_id = r.id)
      ELSE 0 END
  AND ((ar.actor_id IS NULL AND ar.event = 'schedule') OR EXISTS(
      SELECT 1 FROM user_logins u WHERE u.account_id = ar.actor_id AND u.suspended_at IS NULL
      AND (u.is_admin OR r.owner_id = u.account_id
          OR EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id AND c.permission IN ('write', 'admin'))
          OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id
              AND (m.role = 'owner'
                  OR COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') = 'write'
                  OR EXISTS(SELECT 1 FROM teams team JOIN team_members tm ON tm.team_id = team.id
                      JOIN team_repositories tr ON tr.team_id = team.id WHERE team.org_id = r.owner_id
                      AND tm.user_id = u.account_id AND tr.repo_id = r.id AND tr.permission IN ('write', 'admin')))))))

  AND ((ar.trigger_actor_id IS NULL AND ar.event = 'schedule' AND ar.attempt = 1) OR EXISTS(
      SELECT 1 FROM user_logins u WHERE u.account_id = ar.trigger_actor_id AND u.suspended_at IS NULL
      AND (u.is_admin OR r.owner_id = u.account_id
          OR EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id AND c.permission IN ('write', 'admin'))
          OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id
              AND (m.role = 'owner'
                  OR COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') = 'write'
                  OR EXISTS(SELECT 1 FROM teams team JOIN team_members tm ON tm.team_id = team.id
                      JOIN team_repositories tr ON tr.team_id = team.id WHERE team.org_id = r.owner_id
                      AND tm.user_id = u.account_id AND tr.repo_id = r.id AND tr.permission IN ('write', 'admin')))))))
