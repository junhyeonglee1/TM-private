DROP TRIGGER scheduler_jobs_no_delete;
DROP TRIGGER scheduler_runs_identity_immutable;
DROP TRIGGER scheduler_runs_no_delete;
DROP TRIGGER scheduler_attempts_no_update;
DROP TRIGGER scheduler_attempts_no_delete;
DROP TRIGGER scheduler_effects_no_update;
DROP TRIGGER scheduler_effects_no_delete;

CREATE TABLE scheduler_jobs_v13 (
    id TEXT PRIMARY KEY,
    job_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (
        kind IN ('scheduler.canary', 'memory.maintenance', 'stock.daily_screen')
    ),
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

CREATE TABLE scheduler_runs_v13 (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES scheduler_jobs_v13(id),
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

CREATE TABLE scheduler_attempts_v13 (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES scheduler_runs_v13(id),
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

CREATE TABLE scheduler_effects_v13 (
    idempotency_key TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES scheduler_runs_v13(id),
    job_kind TEXT NOT NULL CHECK (
        job_kind IN ('scheduler.canary', 'memory.maintenance', 'stock.daily_screen')
    ),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    applied_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;

INSERT INTO scheduler_jobs_v13 SELECT * FROM scheduler_jobs;
INSERT INTO scheduler_runs_v13 SELECT * FROM scheduler_runs;
INSERT INTO scheduler_attempts_v13 SELECT * FROM scheduler_attempts;
INSERT INTO scheduler_effects_v13 SELECT * FROM scheduler_effects;

DROP TABLE scheduler_effects;
DROP TABLE scheduler_attempts;
DROP TABLE scheduler_runs;
DROP TABLE scheduler_jobs;

ALTER TABLE scheduler_jobs_v13 RENAME TO scheduler_jobs;
ALTER TABLE scheduler_runs_v13 RENAME TO scheduler_runs;
ALTER TABLE scheduler_attempts_v13 RENAME TO scheduler_attempts;
ALTER TABLE scheduler_effects_v13 RENAME TO scheduler_effects;

CREATE INDEX idx_scheduler_jobs_due
    ON scheduler_jobs(enabled, next_run_at, id);
CREATE INDEX idx_scheduler_runs_claim
    ON scheduler_runs(status, available_at, scheduled_for, id);
CREATE INDEX idx_scheduler_runs_lease
    ON scheduler_runs(status, lease_expires_at)
    WHERE status = 'running';
CREATE INDEX idx_scheduler_runs_completed
    ON scheduler_runs(status, completed_at DESC, id DESC);
CREATE INDEX idx_scheduler_attempts_run
    ON scheduler_attempts(run_id, attempt_number);

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

CREATE TABLE stock_universe_snapshots (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) BETWEEN 1 AND 80),
    effective_date TEXT NOT NULL,
    source_url TEXT NOT NULL CHECK (length(source_url) BETWEEN 1 AND 2048),
    source_revision TEXT NOT NULL CHECK (length(source_revision) BETWEEN 1 AND 160),
    source_sha256 TEXT NOT NULL CHECK (length(source_sha256) = 64),
    license_name TEXT NOT NULL CHECK (length(license_name) BETWEEN 1 AND 160),
    license_url TEXT NOT NULL CHECK (length(license_url) BETWEEN 1 AND 2048),
    attribution_text TEXT NOT NULL CHECK (length(trim(attribution_text)) BETWEEN 1 AND 500),
    member_count INTEGER NOT NULL CHECK (member_count BETWEEN 1 AND 1000),
    fetched_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE(source_url, effective_date, source_sha256)
) STRICT;

CREATE INDEX idx_stock_universe_snapshots_effective
    ON stock_universe_snapshots(effective_date DESC, fetched_at DESC, id DESC);

CREATE TABLE stock_universe_members (
    snapshot_id TEXT NOT NULL REFERENCES stock_universe_snapshots(id),
    ticker TEXT NOT NULL CHECK (
        length(ticker) BETWEEN 1 AND 10
        AND ticker NOT GLOB '*[^A-Z0-9.-]*'
    ),
    display_name TEXT NOT NULL CHECK (length(trim(display_name)) BETWEEN 1 AND 160),
    sector TEXT CHECK (sector IS NULL OR length(trim(sector)) BETWEEN 1 AND 160),
    sub_industry TEXT CHECK (
        sub_industry IS NULL OR length(trim(sub_industry)) BETWEEN 1 AND 200
    ),
    created_at TEXT NOT NULL,
    PRIMARY KEY(snapshot_id, ticker)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_stock_universe_members_ticker
    ON stock_universe_members(ticker, snapshot_id);

CREATE TABLE stock_market_data_batches (
    source_sha256 TEXT PRIMARY KEY CHECK (length(source_sha256) = 64),
    source TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 80),
    feed TEXT NOT NULL CHECK (length(feed) BETWEEN 1 AND 40),
    adjustment TEXT NOT NULL CHECK (adjustment = 'split'),
    session_count INTEGER NOT NULL CHECK (session_count BETWEEN 1 AND 366),
    bar_count INTEGER NOT NULL CHECK (bar_count BETWEEN 0 AND 50000),
    earliest_session TEXT NOT NULL,
    latest_session TEXT NOT NULL,
    fetched_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK (earliest_session <= latest_session)
) WITHOUT ROWID, STRICT;

CREATE TABLE stock_market_sessions (
    session_date TEXT PRIMARY KEY,
    source TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 80),
    feed TEXT NOT NULL CHECK (length(feed) BETWEEN 1 AND 40),
    adjustment TEXT NOT NULL CHECK (adjustment = 'split'),
    source_sha256 TEXT NOT NULL REFERENCES stock_market_data_batches(source_sha256),
    bar_count INTEGER NOT NULL CHECK (bar_count >= 0),
    fetched_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;

CREATE TABLE stock_daily_bars (
    ticker TEXT NOT NULL CHECK (
        length(ticker) BETWEEN 1 AND 10
        AND ticker NOT GLOB '*[^A-Z0-9.-]*'
    ),
    session_date TEXT NOT NULL REFERENCES stock_market_sessions(session_date),
    close_microusd INTEGER NOT NULL CHECK (close_microusd > 0),
    source_sha256 TEXT NOT NULL REFERENCES stock_market_data_batches(source_sha256),
    fetched_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(ticker, session_date)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_stock_daily_bars_session
    ON stock_daily_bars(session_date DESC, ticker);

CREATE TABLE stock_screen_runs (
    id TEXT PRIMARY KEY,
    market_date TEXT NOT NULL,
    universe_snapshot_id TEXT REFERENCES stock_universe_snapshots(id),
    status TEXT NOT NULL CHECK (
        status IN (
            'started', 'succeeded', 'no_candidates', 'coverage_failed',
            'upstream_unavailable', 'failed'
        )
    ),
    total_members INTEGER NOT NULL DEFAULT 0 CHECK (total_members BETWEEN 0 AND 1000),
    current_covered INTEGER NOT NULL DEFAULT 0 CHECK (current_covered >= 0),
    baseline_5_covered INTEGER NOT NULL DEFAULT 0 CHECK (baseline_5_covered >= 0),
    baseline_21_covered INTEGER NOT NULL DEFAULT 0 CHECK (baseline_21_covered >= 0),
    result_count INTEGER NOT NULL DEFAULT 0 CHECK (result_count >= 0),
    universe_sha256 TEXT CHECK (universe_sha256 IS NULL OR length(universe_sha256) = 64),
    market_data_sha256 TEXT REFERENCES stock_market_data_batches(source_sha256),
    failure_code TEXT CHECK (failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 120),
    started_at TEXT NOT NULL,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    CHECK (
        (status = 'started' AND completed_at IS NULL)
        OR (status <> 'started' AND completed_at IS NOT NULL)
    ),
    CHECK (current_covered <= total_members),
    CHECK (baseline_5_covered <= total_members),
    CHECK (baseline_21_covered <= total_members)
) STRICT;

CREATE INDEX idx_stock_screen_runs_latest
    ON stock_screen_runs(market_date DESC, created_at DESC, id DESC);

CREATE TABLE stock_screen_results (
    run_id TEXT NOT NULL REFERENCES stock_screen_runs(id),
    ticker TEXT NOT NULL,
    display_name TEXT NOT NULL,
    sector TEXT,
    current_date TEXT NOT NULL,
    current_close_microusd INTEGER NOT NULL CHECK (current_close_microusd > 0),
    baseline_5_date TEXT,
    baseline_5_close_microusd INTEGER CHECK (baseline_5_close_microusd > 0),
    return_5_micros INTEGER,
    band_5 TEXT CHECK (
        band_5 IS NULL OR band_5 IN (
            'down_20_plus', 'down_10_to_20', 'up_10_to_20', 'up_20_plus'
        )
    ),
    baseline_21_date TEXT,
    baseline_21_close_microusd INTEGER CHECK (baseline_21_close_microusd > 0),
    return_21_micros INTEGER,
    band_21 TEXT CHECK (
        band_21 IS NULL OR band_21 IN (
            'down_20_plus', 'down_10_to_20', 'up_10_to_20', 'up_20_plus'
        )
    ),
    universe_sha256 TEXT NOT NULL CHECK (length(universe_sha256) = 64),
    market_data_sha256 TEXT NOT NULL REFERENCES stock_market_data_batches(source_sha256),
    created_at TEXT NOT NULL,
    PRIMARY KEY(run_id, ticker),
    CHECK (
        (baseline_5_date IS NULL AND baseline_5_close_microusd IS NULL
            AND return_5_micros IS NULL AND band_5 IS NULL)
        OR (baseline_5_date IS NOT NULL AND baseline_5_close_microusd IS NOT NULL
            AND return_5_micros IS NOT NULL AND band_5 IS NOT NULL)
    ),
    CHECK (
        (baseline_21_date IS NULL AND baseline_21_close_microusd IS NULL
            AND return_21_micros IS NULL AND band_21 IS NULL)
        OR (baseline_21_date IS NOT NULL AND baseline_21_close_microusd IS NOT NULL
            AND return_21_micros IS NOT NULL AND band_21 IS NOT NULL)
    ),
    CHECK (band_5 IS NOT NULL OR band_21 IS NOT NULL)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_stock_screen_results_run
    ON stock_screen_results(run_id, ticker);

CREATE TABLE stock_ai_reports (
    id TEXT PRIMARY KEY,
    screen_run_id TEXT NOT NULL REFERENCES stock_screen_runs(id),
    status TEXT NOT NULL CHECK (
        status IN ('started', 'succeeded', 'failed', 'ai_uncertain')
    ),
    prompt_version TEXT NOT NULL CHECK (length(prompt_version) BETWEEN 1 AND 64),
    model TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 128),
    response_id TEXT,
    upstream_request_id TEXT,
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    cached_input_tokens INTEGER CHECK (
        cached_input_tokens IS NULL OR cached_input_tokens >= 0
    ),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    estimated_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (
        estimated_cost_microusd >= 0
    ),
    failure_code TEXT,
    request_started_at TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    CHECK (
        (status = 'started' AND completed_at IS NULL)
        OR (status <> 'started' AND completed_at IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_stock_ai_reports_run
    ON stock_ai_reports(screen_run_id, created_at DESC, id DESC);

CREATE TRIGGER stock_universe_snapshots_no_update
BEFORE UPDATE ON stock_universe_snapshots
BEGIN
    SELECT RAISE(ABORT, 'stock universe snapshots are immutable');
END;

CREATE TRIGGER stock_universe_snapshots_no_delete
BEFORE DELETE ON stock_universe_snapshots
BEGIN
    SELECT RAISE(ABORT, 'stock universe snapshots cannot be deleted');
END;

CREATE TRIGGER stock_universe_members_no_update
BEFORE UPDATE ON stock_universe_members
BEGIN
    SELECT RAISE(ABORT, 'stock universe members are immutable');
END;

CREATE TRIGGER stock_universe_members_no_delete
BEFORE DELETE ON stock_universe_members
BEGIN
    SELECT RAISE(ABORT, 'stock universe members cannot be deleted');
END;

CREATE TRIGGER stock_market_data_batches_no_update
BEFORE UPDATE ON stock_market_data_batches
BEGIN
    SELECT RAISE(ABORT, 'stock market data batches are immutable');
END;

CREATE TRIGGER stock_market_data_batches_no_delete
BEFORE DELETE ON stock_market_data_batches
BEGIN
    SELECT RAISE(ABORT, 'stock market data batches cannot be deleted');
END;

CREATE TRIGGER stock_screen_runs_identity_immutable
BEFORE UPDATE ON stock_screen_runs
WHEN NEW.id <> OLD.id
  OR NEW.market_date <> OLD.market_date
  OR NEW.started_at <> OLD.started_at
  OR NEW.created_at <> OLD.created_at
  OR OLD.status <> 'started'
BEGIN
    SELECT RAISE(ABORT, 'stock screen run is immutable after completion');
END;

CREATE TRIGGER stock_screen_runs_no_delete
BEFORE DELETE ON stock_screen_runs
BEGIN
    SELECT RAISE(ABORT, 'stock screen runs cannot be deleted');
END;

CREATE TRIGGER stock_screen_results_no_update
BEFORE UPDATE ON stock_screen_results
BEGIN
    SELECT RAISE(ABORT, 'stock screen results are immutable');
END;

CREATE TRIGGER stock_screen_results_no_delete
BEFORE DELETE ON stock_screen_results
BEGIN
    SELECT RAISE(ABORT, 'stock screen results cannot be deleted');
END;

CREATE TRIGGER stock_ai_reports_identity_immutable
BEFORE UPDATE ON stock_ai_reports
WHEN NEW.id <> OLD.id
  OR NEW.screen_run_id <> OLD.screen_run_id
  OR NEW.prompt_version <> OLD.prompt_version
  OR NEW.model <> OLD.model
  OR NEW.created_at <> OLD.created_at
  OR OLD.status <> 'started'
BEGIN
    SELECT RAISE(ABORT, 'stock AI report is immutable after completion');
END;

CREATE TRIGGER stock_ai_reports_no_delete
BEFORE DELETE ON stock_ai_reports
BEGIN
    SELECT RAISE(ABORT, 'stock AI reports cannot be deleted');
END;
