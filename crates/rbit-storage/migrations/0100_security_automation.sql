ALTER TABLE security_scans ADD COLUMN channel TEXT NOT NULL DEFAULT 'external' CHECK(channel IN ('external', 'secrets', 'npm'));
CREATE TABLE security_automation (
    repo_id INTEGER PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    settings TEXT NOT NULL CHECK(json_valid(settings)),
    generation INTEGER NOT NULL DEFAULT 1 CHECK(generation > 0),
    enabled_by INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    fingerprint_key BLOB NOT NULL DEFAULT (randomblob(32)) CHECK(length(fingerprint_key) = 32)
) STRICT;
CREATE TABLE security_scan_jobs (
    scan_id INTEGER PRIMARY KEY REFERENCES security_scans(id) ON DELETE CASCADE,
    claimed_until INTEGER NOT NULL DEFAULT 0,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK(attempts >= 0)
) STRICT;
CREATE INDEX security_scan_jobs_claim ON security_scan_jobs(claimed_until, scan_id);
CREATE INDEX security_scans_channel ON security_scans(repo_id, channel, commit_sha, generation, id DESC);
CREATE TABLE security_remediations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    base_sha TEXT NOT NULL,
    package TEXT NOT NULL,
    target_version TEXT NOT NULL,
    branch TEXT NOT NULL,
    commit_sha TEXT,
    pull_number INTEGER,
    created_at INTEGER NOT NULL,
    UNIQUE(repo_id, package, target_version),
    UNIQUE(repo_id, branch)
) STRICT;
