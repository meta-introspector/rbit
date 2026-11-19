-- Public keys explicitly trusted by this repository's administrators.
CREATE TABLE repository_signing_keys (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK(length(name) BETWEEN 1 AND 100),
    kind TEXT NOT NULL CHECK(kind IN ('ssh', 'openpgp')),
    public_key TEXT NOT NULL CHECK(length(public_key) BETWEEN 1 AND 65536),
    created_at INTEGER NOT NULL,
    UNIQUE(repo_id, name)
) STRICT;
CREATE INDEX repository_signing_keys_repo ON repository_signing_keys(repo_id);
