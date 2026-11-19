-- Cloud trust must not follow a reclaimed name or a reused SQLite integer ID.
CREATE TABLE actions_oidc_accounts (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    uid TEXT NOT NULL UNIQUE CHECK(length(uid) = 32)
) STRICT;
CREATE TABLE actions_oidc_repositories (
    repo_id INTEGER PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    uid TEXT NOT NULL UNIQUE CHECK(length(uid) = 32)
) STRICT;
CREATE TRIGGER actions_oidc_repository_transfer AFTER UPDATE OF owner_id ON repositories
WHEN OLD.owner_id <> NEW.owner_id BEGIN
    UPDATE actions_oidc_repositories SET uid = lower(hex(randomblob(16))) WHERE repo_id = NEW.id;
END;

-- Allocate identities only when Actions creates a run: disabled Actions leaves these
-- tables empty. Capture tenure in that same insert, never from runner-supplied claims.
ALTER TABLE actions_runs ADD COLUMN oidc_owner_uid TEXT;
ALTER TABLE actions_runs ADD COLUMN oidc_repository_uid TEXT;
CREATE TRIGGER actions_oidc_run_insert AFTER INSERT ON actions_runs BEGIN
    INSERT INTO actions_oidc_accounts (account_id, uid)
      SELECT owner_id, lower(hex(randomblob(16))) FROM repositories WHERE id = NEW.repo_id
      ON CONFLICT(account_id) DO NOTHING;
    INSERT INTO actions_oidc_repositories (repo_id, uid)
      VALUES(NEW.repo_id, lower(hex(randomblob(16)))) ON CONFLICT(repo_id) DO NOTHING;
    UPDATE actions_runs SET
      oidc_owner_uid = (SELECT o.uid FROM repositories r JOIN actions_oidc_accounts o ON o.account_id = r.owner_id WHERE r.id = NEW.repo_id),
      oidc_repository_uid = (SELECT uid FROM actions_oidc_repositories WHERE repo_id = NEW.repo_id)
    WHERE id = NEW.id;
END;

-- Only sealed private scalars are stored. The Actions secrets key stays outside SQLite.
CREATE TABLE actions_oidc_keys (
    period INTEGER PRIMARY KEY CHECK(period >= 0),
    key_id BLOB NOT NULL CHECK(length(key_id) = 8),
    nonce BLOB NOT NULL CHECK(length(nonce) = 12),
    ciphertext BLOB NOT NULL CHECK(length(ciphertext) = 59),
    public_jwk TEXT NOT NULL CHECK(length(public_jwk) <= 1024 AND json_valid(public_jwk))
) STRICT;
CREATE TABLE actions_oidc_issuance (
    task_id INTEGER PRIMARY KEY REFERENCES actions_tasks(id) ON DELETE CASCADE,
    minute INTEGER NOT NULL,
    count INTEGER NOT NULL CHECK(count BETWEEN 1 AND 60)
) STRICT;
