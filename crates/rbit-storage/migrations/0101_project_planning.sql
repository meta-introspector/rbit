-- Planning belongs to each project placement, not to the linked issue lifecycle.
CREATE TABLE project_iterations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    board_id INTEGER NOT NULL REFERENCES project_boards(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 100),
    starts_on TEXT NOT NULL CHECK(length(starts_on) = 10),
    ends_on TEXT NOT NULL CHECK(length(ends_on) = 10 AND ends_on >= starts_on),
    archived INTEGER NOT NULL DEFAULT 0 CHECK(archived IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    UNIQUE(board_id, id)
) STRICT;
CREATE UNIQUE INDEX project_iterations_named ON project_iterations(board_id, title COLLATE NOCASE);
CREATE TABLE project_card_planning (
    board_id INTEGER NOT NULL,
    issue_id INTEGER NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0 CHECK(priority BETWEEN 0 AND 4),
    iteration_id INTEGER,
    target_date TEXT CHECK(target_date IS NULL OR length(target_date) = 10),
    PRIMARY KEY(board_id, issue_id),
    FOREIGN KEY(board_id, issue_id) REFERENCES project_cards(board_id, issue_id) ON DELETE CASCADE,
    FOREIGN KEY(board_id, iteration_id) REFERENCES project_iterations(board_id, id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED
) STRICT, WITHOUT ROWID;
CREATE INDEX project_planning_by_iteration ON project_card_planning(board_id, iteration_id);
CREATE INDEX project_planning_by_priority ON project_card_planning(board_id, priority, issue_id);
CREATE INDEX project_planning_by_date ON project_card_planning(board_id, target_date, issue_id);
CREATE TABLE project_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    board_id INTEGER NOT NULL REFERENCES project_boards(id) ON DELETE CASCADE,
    owner_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE,
    title TEXT NOT NULL CHECK(length(title) BETWEEN 1 AND 100),
    repository_id INTEGER REFERENCES repositories(id) ON DELETE CASCADE,
    iteration_id INTEGER,
    criteria_json TEXT NOT NULL CHECK(json_valid(criteria_json) AND length(criteria_json) <= 4096),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision > 0),
    created_at INTEGER NOT NULL,
    FOREIGN KEY(board_id, iteration_id) REFERENCES project_iterations(board_id, id) ON DELETE CASCADE
) STRICT;
CREATE UNIQUE INDEX project_views_named ON project_views(board_id, COALESCE(owner_id, 0), title COLLATE NOCASE);
CREATE INDEX project_views_by_owner ON project_views(board_id, owner_id, id);
