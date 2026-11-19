CREATE TABLE tag_protections (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    pattern TEXT NOT NULL CHECK(length(pattern) BETWEEN 1 AND 255),
    UNIQUE(repo_id, pattern)
) STRICT;
