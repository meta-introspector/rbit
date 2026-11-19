-- Agent accounts retain a permanent non-interactive identity marker even after their
-- owner is deleted. Missing delegation records always deny authentication.
ALTER TABLE user_logins ADD COLUMN is_agent INTEGER NOT NULL DEFAULT 0 CHECK (is_agent IN (0, 1));

CREATE TABLE agent_accounts (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    owner_id INTEGER NOT NULL REFERENCES user_logins(account_id) ON DELETE CASCADE,
    slug TEXT NOT NULL COLLATE NOCASE,
    name TEXT NOT NULL,
    capabilities_json TEXT NOT NULL CHECK (json_valid(capabilities_json)),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(owner_id, slug),
    CHECK (account_id <> owner_id)
) STRICT;

CREATE TABLE agent_scopes (
    id INTEGER PRIMARY KEY,
    agent_id INTEGER NOT NULL REFERENCES agent_accounts(account_id) ON DELETE CASCADE,
    repo_id INTEGER REFERENCES repositories(id) ON DELETE CASCADE,
    org_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    permission INTEGER NOT NULL CHECK (permission IN (1, 2)),
    CHECK ((repo_id IS NULL) <> (org_id IS NULL))
) STRICT;
CREATE UNIQUE INDEX agent_scopes_repository ON agent_scopes(agent_id, repo_id) WHERE repo_id IS NOT NULL;
CREATE UNIQUE INDEX agent_scopes_organization ON agent_scopes(agent_id, org_id) WHERE org_id IS NOT NULL;

CREATE TABLE agent_credentials (
    token_id INTEGER PRIMARY KEY REFERENCES access_tokens(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL REFERENCES agent_accounts(account_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX agent_credentials_identity ON agent_credentials(agent_id);

-- Parent deletion must remove credential digests as well as delegation links. The
-- account/login survives with its permanent agent marker and cannot become a human.
CREATE TRIGGER agent_accounts_revoke_credentials BEFORE DELETE ON agent_accounts
BEGIN
    DELETE FROM access_tokens WHERE account_id = OLD.account_id;
END;

-- Password recovery and epoch revocation permanently retire child credentials,
-- including ones issued by a compromised parent before recovery began.
CREATE TRIGGER agent_parent_credentials_changed AFTER UPDATE OF password_hash, authentication_epoch ON user_logins
WHEN NEW.is_agent = 0 AND (NEW.password_hash <> OLD.password_hash OR NEW.authentication_epoch <> OLD.authentication_epoch)
BEGIN
    DELETE FROM access_tokens WHERE account_id IN (SELECT account_id FROM agent_accounts WHERE owner_id = NEW.account_id);
END;

CREATE TABLE org_agent_policies (
    org_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    allow_agents INTEGER NOT NULL DEFAULT 1 CHECK (allow_agents IN (0, 1))
) STRICT;

-- Agent scopes cannot turn ownership into administration. Effective rights are capped
-- at write and recomputed from the human owner's live grants and organization policy.
CREATE VIEW agent_repository_grants AS
WITH eligible AS (
    SELECT a.account_id AS agent_id, a.owner_id, r.id AS repo_id, r.owner_id AS repo_owner,
           s.permission AS requested,
           CASE
             WHEN u.is_admin = 1 OR r.owner_id = u.account_id THEN 2
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id AND m.role = 'owner') THEN 2
             WHEN EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id AND c.permission IN ('write','admin')) THEN 2
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id
                  AND (COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') = 'write'
                  OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id
                            WHERE t.org_id = r.owner_id AND tm.user_id = u.account_id AND tr.repo_id = r.id AND tr.permission IN ('write','admin')))) THEN 2
             WHEN r.visibility = 'public' THEN 1
             WHEN EXISTS(SELECT 1 FROM collaborators c WHERE c.repo_id = r.id AND c.user_id = u.account_id) THEN 1
             WHEN EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = r.owner_id AND m.user_id = u.account_id
                  AND (COALESCE((SELECT member_permission FROM org_profiles WHERE account_id = m.org_id), 'write') <> 'none'
                  OR EXISTS(SELECT 1 FROM teams t JOIN team_members tm ON tm.team_id = t.id JOIN team_repositories tr ON tr.team_id = t.id
                            WHERE t.org_id = r.owner_id AND tm.user_id = u.account_id AND tr.repo_id = r.id))) THEN 1
             ELSE 0 END AS inherited
    FROM agent_accounts a
    JOIN user_logins u ON u.account_id = a.owner_id AND u.is_agent = 0 AND u.suspended_at IS NULL AND u.password_change_required = 0
    JOIN user_logins bot ON bot.account_id = a.account_id AND bot.is_agent = 1 AND bot.suspended_at IS NULL
    JOIN agent_scopes s ON s.agent_id = a.account_id
    JOIN repositories r ON r.id = s.repo_id OR r.owner_id = s.org_id
    WHERE a.enabled = 1
      AND NOT EXISTS(SELECT 1 FROM credential_revocations pending WHERE pending.account_id = u.account_id
                     AND pending.target_epoch = u.authentication_epoch AND pending.completed_at IS NULL)
      AND (s.org_id IS NULL OR EXISTS(SELECT 1 FROM org_members m WHERE m.org_id = s.org_id AND m.user_id = a.owner_id))
      AND NOT EXISTS(SELECT 1 FROM org_agent_policies p WHERE p.org_id = r.owner_id AND p.allow_agents = 0)
)
SELECT agent_id, owner_id, repo_id, repo_owner, MAX(MIN(requested, inherited)) AS permission
FROM eligible WHERE inherited > 0 GROUP BY agent_id, repo_id;
