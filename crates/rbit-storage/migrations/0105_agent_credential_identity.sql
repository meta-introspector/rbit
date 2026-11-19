-- Public management identifiers must never target a newer credential after deletion.
-- access_tokens rowids may be reused; keep authentication links separate from stable ids.
CREATE TABLE agent_credentials_stable (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token_id INTEGER NOT NULL UNIQUE REFERENCES access_tokens(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL REFERENCES agent_accounts(account_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL
) STRICT;
INSERT INTO agent_credentials_stable(token_id, agent_id, created_at)
SELECT token_id, agent_id, created_at FROM agent_credentials ORDER BY token_id;
DROP TABLE agent_credentials;
ALTER TABLE agent_credentials_stable RENAME TO agent_credentials;
CREATE INDEX agent_credentials_identity ON agent_credentials(agent_id);
