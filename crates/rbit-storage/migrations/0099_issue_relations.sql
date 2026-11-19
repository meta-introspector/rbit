-- Stable edge identities fence stale unlink requests; both endpoints cascade on deletion.
CREATE TABLE issue_relations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    target_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('child', 'blocked_by')),
    created_at INTEGER NOT NULL,
    CHECK(source_id <> target_id),
    UNIQUE(source_id, target_id, kind)
) STRICT;
CREATE INDEX issue_relations_target ON issue_relations(target_id);
CREATE UNIQUE INDEX issue_relations_parent ON issue_relations(target_id) WHERE kind = 'child';
-- Cross-repository links do not survive transfer. Same-repository links follow the repository.
CREATE TRIGGER issue_relations_transfer AFTER UPDATE OF owner_id ON repositories
WHEN NEW.owner_id <> OLD.owner_id
BEGIN
    DELETE FROM issue_relations
    WHERE (source_id IN (SELECT id FROM issues WHERE repo_id = NEW.id)
        OR target_id IN (SELECT id FROM issues WHERE repo_id = NEW.id))
      AND (SELECT repo_id FROM issues WHERE id = source_id) <> (SELECT repo_id FROM issues WHERE id = target_id);
END;
