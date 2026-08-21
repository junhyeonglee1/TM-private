CREATE TABLE scheduler_jobs (
    id TEXT PRIMARY KEY,
    job_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('scheduler.canary', 'memory.maintenance')),
    schedule_type TEXT NOT NULL CHECK (schedule_type IN ('interval', 'daily', 'once')),
    interval_seconds INTEGER,
    local_time TEXT,
    timezone TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    max_attempts INTEGER NOT NULL CHECK (max_attempts BETWEEN 1 AND 10),
    misfire_grace_seconds INTEGER NOT NULL CHECK (misfire_grace_seconds >= 0),
    coalesce INTEGER NOT NULL CHECK (coalesce IN (0, 1)),
    next_run_at TEXT NOT NULL,
    last_scheduled_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        (schedule_type = 'interval' AND interval_seconds IS NOT NULL
            AND interval_seconds > 0 AND local_time IS NULL)
        OR (schedule_type = 'daily' AND interval_seconds IS NULL AND local_time IS NOT NULL)
        OR (schedule_type = 'once' AND interval_seconds IS NULL AND local_time IS NULL)
    )
) STRICT;

CREATE INDEX idx_scheduler_jobs_due
    ON scheduler_jobs(enabled, next_run_at, id);

CREATE TABLE scheduler_runs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES scheduler_jobs(id),
    scheduled_for TEXT NOT NULL,
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'running', 'retry_wait', 'succeeded', 'dead_letter', 'skipped')
    ),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts BETWEEN 1 AND 10),
    idempotency_key TEXT NOT NULL UNIQUE,
    available_at TEXT NOT NULL,
    lease_owner TEXT,
    lease_acquired_at TEXT,
    lease_expires_at TEXT,
    last_error TEXT,
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    started_at TEXT,
    completed_at TEXT,
    dead_letter_at TEXT,
    skipped_at TEXT,
    UNIQUE(job_id, scheduled_for),
    CHECK (
        (status = 'running' AND lease_owner IS NOT NULL
            AND lease_acquired_at IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR (status <> 'running' AND lease_owner IS NULL
            AND lease_acquired_at IS NULL AND lease_expires_at IS NULL)
    )
) STRICT;

CREATE INDEX idx_scheduler_runs_claim
    ON scheduler_runs(status, available_at, scheduled_for, id);

CREATE INDEX idx_scheduler_runs_lease
    ON scheduler_runs(status, lease_expires_at)
    WHERE status = 'running';

CREATE INDEX idx_scheduler_runs_completed
    ON scheduler_runs(status, completed_at DESC, id DESC);

CREATE TABLE scheduler_attempts (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES scheduler_runs(id),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    worker_id TEXT NOT NULL,
    started_at TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (
        outcome IN ('succeeded', 'retry_scheduled', 'dead_letter', 'lease_expired')
    ),
    error TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(run_id, attempt_number)
) STRICT;

CREATE INDEX idx_scheduler_attempts_run
    ON scheduler_attempts(run_id, attempt_number);

CREATE TABLE scheduler_effects (
    idempotency_key TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES scheduler_runs(id),
    job_kind TEXT NOT NULL CHECK (job_kind IN ('scheduler.canary', 'memory.maintenance')),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    applied_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;

CREATE TRIGGER scheduler_jobs_no_delete
BEFORE DELETE ON scheduler_jobs
BEGIN
    SELECT RAISE(ABORT, 'scheduler jobs cannot be deleted');
END;

CREATE TRIGGER scheduler_runs_identity_immutable
BEFORE UPDATE ON scheduler_runs
WHEN NEW.job_id <> OLD.job_id
  OR NEW.scheduled_for <> OLD.scheduled_for
  OR NEW.max_attempts <> OLD.max_attempts
  OR NEW.idempotency_key <> OLD.idempotency_key
  OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'scheduler run identity is immutable');
END;

CREATE TRIGGER scheduler_runs_no_delete
BEFORE DELETE ON scheduler_runs
BEGIN
    SELECT RAISE(ABORT, 'scheduler runs cannot be deleted');
END;

CREATE TRIGGER scheduler_attempts_no_update
BEFORE UPDATE ON scheduler_attempts
BEGIN
    SELECT RAISE(ABORT, 'scheduler attempts are append-only');
END;

CREATE TRIGGER scheduler_attempts_no_delete
BEFORE DELETE ON scheduler_attempts
BEGIN
    SELECT RAISE(ABORT, 'scheduler attempts cannot be deleted');
END;

CREATE TRIGGER scheduler_effects_no_update
BEFORE UPDATE ON scheduler_effects
BEGIN
    SELECT RAISE(ABORT, 'scheduler effects are immutable');
END;

CREATE TRIGGER scheduler_effects_no_delete
BEFORE DELETE ON scheduler_effects
BEGIN
    SELECT RAISE(ABORT, 'scheduler effects cannot be deleted');
END;
