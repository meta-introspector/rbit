-- Organization boards store workflow placement; completed state remains owned by issues.
CREATE TABLE project_boards (
    id INTEGER PRIMARY KEY,
    org_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 120),
    columns_json TEXT NOT NULL CHECK(json_valid(columns_json) AND json_array_length(columns_json) BETWEEN 2 AND 12),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX project_boards_by_org ON project_boards(org_id, id);
CREATE TABLE project_cards (
    board_id INTEGER NOT NULL REFERENCES project_boards(id) ON DELETE CASCADE,
    issue_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    column_index INTEGER NOT NULL CHECK(column_index BETWEEN 0 AND 11),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    PRIMARY KEY(board_id, issue_id)
) STRICT, WITHOUT ROWID;
CREATE INDEX project_cards_by_issue ON project_cards(issue_id);
