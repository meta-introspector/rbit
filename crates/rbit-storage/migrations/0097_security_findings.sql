CREATE TABLE security_settings (
    repo_id INTEGER PRIMARY KEY REFERENCES repositories(id) ON DELETE CASCADE,
    scanner TEXT NOT NULL CHECK(length(scanner) BETWEEN 1 AND 100),
    workflow TEXT NOT NULL CHECK(length(workflow) <= 1024),
    require_scan INTEGER NOT NULL CHECK(require_scan IN (0, 1)),
    minimum_severity INTEGER NOT NULL CHECK(minimum_severity BETWEEN 1 AND 4),
    generation INTEGER NOT NULL DEFAULT 1 CHECK(generation > 0)
) STRICT;
CREATE TABLE security_scans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    scanner TEXT NOT NULL,
    commit_sha TEXT NOT NULL CHECK(length(commit_sha) IN (40, 64)),
    generation INTEGER NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK(state IN ('pending', 'complete', 'incomplete')),
    created_at INTEGER NOT NULL,
    finished_at INTEGER
) STRICT;
CREATE INDEX security_scans_revision ON security_scans(repo_id, scanner, commit_sha, generation, id DESC);
CREATE TABLE security_findings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    scanner TEXT NOT NULL,
    fingerprint TEXT NOT NULL CHECK(length(fingerprint) = 64),
    dismissed_by INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    dismissed_at INTEGER,
    reason TEXT CHECK(reason IN ('false_positive', 'accepted_risk', 'used_in_tests')),
    note TEXT CHECK(length(note) <= 1000),
    UNIQUE(repo_id, scanner, fingerprint)
) STRICT;
CREATE TABLE security_occurrences (
    scan_id INTEGER NOT NULL REFERENCES security_scans(id) ON DELETE CASCADE,
    finding_id INTEGER NOT NULL REFERENCES security_findings(id) ON DELETE CASCADE,
    rule TEXT NOT NULL CHECK(length(rule) <= 200),
    message TEXT NOT NULL CHECK(length(message) <= 2000),
    path TEXT NOT NULL CHECK(length(path) <= 1024),
    line INTEGER NOT NULL CHECK(line > 0),
    severity INTEGER NOT NULL CHECK(severity BETWEEN 0 AND 4),
    PRIMARY KEY(scan_id, finding_id)
) STRICT;
