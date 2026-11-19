-- Preserve completed job attempts before reruns replace the mutable job record. Run/job
-- numbers are historical identities, deliberately retained when Actions retention prunes
-- the source run. Repository deletion removes its history and deployment configuration.
CREATE TABLE actions_job_attempt_history (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    run_id INTEGER NOT NULL,
    run_number INTEGER NOT NULL,
    run_attempt INTEGER NOT NULL CHECK (run_attempt > 0),
    job_id INTEGER NOT NULL,
    job_attempt INTEGER NOT NULL CHECK (job_attempt > 0),
    job_key TEXT NOT NULL,
    job_name TEXT NOT NULL,
    workflow_path TEXT NOT NULL,
    ref TEXT NOT NULL,
    sha TEXT NOT NULL,
    workflow_sha TEXT NOT NULL,
    origin_attempt_id INTEGER REFERENCES actions_job_attempt_history(id) ON DELETE CASCADE,
    artifact_id INTEGER,
    artifact_digest TEXT,
    trusted INTEGER NOT NULL CHECK (trusted IN (0, 1)),
    is_fork INTEGER NOT NULL CHECK (is_fork IN (0, 1)),
    event TEXT NOT NULL,
    environment TEXT,
    environment_url TEXT,
    status INTEGER NOT NULL CHECK (status BETWEEN 1 AND 4),
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    UNIQUE (repo_id, run_number, job_id, job_attempt),
    CHECK ((artifact_id IS NULL) = (artifact_digest IS NULL))
) STRICT;
CREATE INDEX actions_job_attempt_environments ON actions_job_attempt_history(repo_id, environment, id DESC);

-- An administrator selects the workflow; writers select a successful source attempt.
-- The workflow is always evaluated from the repository's current default branch.
CREATE TABLE actions_deployment_workflows (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    environment TEXT NOT NULL COLLATE NOCASE CHECK (length(environment) BETWEEN 1 AND 255),
    source_environment TEXT NOT NULL COLLATE NOCASE CHECK (length(source_environment) BETWEEN 1 AND 255),
    workflow_path TEXT NOT NULL CHECK (length(workflow_path) BETWEEN 1 AND 1024),
    generation INTEGER NOT NULL CHECK (generation > 0),
    updated_at INTEGER NOT NULL,
    UNIQUE (repo_id, environment),
    FOREIGN KEY (repo_id, environment) REFERENCES actions_environments(repo_id, name) ON DELETE CASCADE,
    FOREIGN KEY (repo_id, source_environment) REFERENCES actions_environments(repo_id, name) ON DELETE CASCADE
) STRICT;
CREATE TABLE actions_deployment_intents (
    id INTEGER PRIMARY KEY,
    repo_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    actor_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
    operation TEXT NOT NULL CHECK (operation IN ('promote', 'rollback')),
    environment TEXT NOT NULL,
    source_attempt_id INTEGER NOT NULL REFERENCES actions_job_attempt_history(id) ON DELETE CASCADE,
    source_sha TEXT NOT NULL,
    origin_attempt_id INTEGER NOT NULL REFERENCES actions_job_attempt_history(id) ON DELETE CASCADE,
    origin_artifact_id INTEGER,
    origin_artifact_digest TEXT,
    artifact_id INTEGER,
    artifact_digest TEXT,
    workflow_path TEXT NOT NULL,
    workflow_sha TEXT NOT NULL,
    workflow_ref TEXT NOT NULL,
    configuration_id INTEGER NOT NULL,
    configuration_generation INTEGER NOT NULL,
    run_id INTEGER UNIQUE REFERENCES actions_runs(id) ON DELETE SET NULL,
    run_number INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE (repo_id, run_number),
    CHECK ((origin_artifact_id IS NULL) = (origin_artifact_digest IS NULL)),
    CHECK ((artifact_id IS NULL) = (artifact_digest IS NULL))
) STRICT;
CREATE INDEX actions_deployment_intents_repository ON actions_deployment_intents(repo_id, id DESC);

CREATE TRIGGER actions_deployment_intent_immutable BEFORE UPDATE OF
    repo_id, operation, environment, source_attempt_id, source_sha, origin_attempt_id, origin_artifact_id, origin_artifact_digest, artifact_id, artifact_digest,
    workflow_path, workflow_sha, workflow_ref, configuration_id, configuration_generation, run_number, created_at
ON actions_deployment_intents
BEGIN
    SELECT RAISE(ABORT, 'deployment intents are immutable');
END;

INSERT INTO actions_job_attempt_history
    (repo_id, run_id, run_number, run_attempt, job_id, job_attempt, job_key, job_name,
     workflow_path, ref, sha, workflow_sha, origin_attempt_id, artifact_id, artifact_digest, trusted, is_fork, event, environment, environment_url,
     status, created_at, started_at, finished_at)
SELECT j.repo_id, r.id, r.run_number, r.attempt, j.id, j.attempt, j.job_key, j.name,
       r.workflow_path, r.ref, r.sha, r.sha, NULL, NULL, NULL, r.trusted, r.is_fork, r.event, j.environment,
       j.environment_url, j.status, j.created_at, j.started_at, j.finished_at
FROM actions_jobs j JOIN actions_runs r ON r.id = j.run_id WHERE j.status BETWEEN 1 AND 4;

CREATE TRIGGER actions_job_attempt_finished AFTER UPDATE OF status ON actions_jobs
WHEN NEW.status BETWEEN 1 AND 4
BEGIN
    INSERT OR IGNORE INTO actions_job_attempt_history
        (repo_id, run_id, run_number, run_attempt, job_id, job_attempt, job_key, job_name,
         workflow_path, ref, sha, workflow_sha, origin_attempt_id, artifact_id, artifact_digest, trusted, is_fork, event, environment, environment_url,
         status, created_at, started_at, finished_at)
    SELECT NEW.repo_id, r.id, r.run_number, r.attempt, NEW.id, NEW.attempt, NEW.job_key,
           NEW.name, r.workflow_path, r.ref, COALESCE(i.source_sha, r.sha), r.sha, i.origin_attempt_id,
           i.origin_artifact_id, i.origin_artifact_digest, r.trusted, r.is_fork, r.event,
           NEW.environment, NEW.environment_url, NEW.status, NEW.created_at,
           NEW.started_at, NEW.finished_at FROM actions_runs r LEFT JOIN actions_deployment_intents i
           ON i.repo_id = r.repo_id AND i.run_number = r.run_number WHERE r.id = NEW.run_id;
END;
CREATE TRIGGER actions_job_attempt_inserted AFTER INSERT ON actions_jobs
WHEN NEW.status BETWEEN 1 AND 4
BEGIN
    INSERT OR IGNORE INTO actions_job_attempt_history
        (repo_id, run_id, run_number, run_attempt, job_id, job_attempt, job_key, job_name,
         workflow_path, ref, sha, workflow_sha, origin_attempt_id, artifact_id, artifact_digest, trusted, is_fork, event, environment, environment_url,
         status, created_at, started_at, finished_at)
    SELECT NEW.repo_id, r.id, r.run_number, r.attempt, NEW.id, NEW.attempt, NEW.job_key,
           NEW.name, r.workflow_path, r.ref, COALESCE(i.source_sha, r.sha), r.sha, i.origin_attempt_id,
           i.origin_artifact_id, i.origin_artifact_digest, r.trusted, r.is_fork, r.event,
           NEW.environment, NEW.environment_url, NEW.status, NEW.created_at,
           NEW.started_at, NEW.finished_at FROM actions_runs r LEFT JOIN actions_deployment_intents i
           ON i.repo_id = r.repo_id AND i.run_number = r.run_number WHERE r.id = NEW.run_id;
END;
CREATE TRIGGER actions_job_attempt_immutable BEFORE UPDATE ON actions_job_attempt_history
BEGIN
    SELECT RAISE(ABORT, 'completed job attempts are immutable');
END;
