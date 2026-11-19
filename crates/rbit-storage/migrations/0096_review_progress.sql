-- Personal file-review progress follows exact base/head tree entries, not merely names.
CREATE TABLE pull_review_progress (
    pull_id INTEGER NOT NULL REFERENCES pull_requests(issue_id) ON DELETE CASCADE,
    reviewer_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    path TEXT NOT NULL CHECK (length(path) BETWEEN 1 AND 4096),
    fingerprint TEXT NOT NULL CHECK (length(fingerprint) BETWEEN 1 AND 8192),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (pull_id, reviewer_id, path)
) STRICT;
