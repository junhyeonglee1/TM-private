CREATE TABLE task_report_runs (
    id TEXT PRIMARY KEY,
    report_date TEXT NOT NULL,
    actor TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('started', 'succeeded', 'failed', 'no_tasks')),
    candidate_count INTEGER NOT NULL CHECK (candidate_count >= 0 AND candidate_count <= 20),
    prompt_version TEXT NOT NULL,
    model TEXT NOT NULL,
    response_id TEXT,
    upstream_request_id TEXT,
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    cached_input_tokens INTEGER CHECK (cached_input_tokens IS NULL OR cached_input_tokens >= 0),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    estimated_cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (estimated_cost_microusd >= 0),
    latency_ms INTEGER CHECK (latency_ms IS NULL OR latency_ms >= 0),
    failure_code TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    CHECK (
        (status = 'started' AND completed_at IS NULL AND result_json IS NULL)
        OR (status = 'succeeded' AND completed_at IS NOT NULL AND result_json IS NOT NULL)
        OR (status = 'failed' AND completed_at IS NOT NULL AND failure_code IS NOT NULL)
        OR (status = 'no_tasks' AND completed_at IS NOT NULL AND result_json IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_task_report_runs_date_created
    ON task_report_runs(report_date, created_at DESC, id DESC);

CREATE TABLE task_report_feedback (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES task_report_runs(id),
    helpful INTEGER NOT NULL CHECK (helpful IN (0, 1)),
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_task_report_feedback_created
    ON task_report_feedback(created_at DESC, id DESC);

CREATE TRIGGER task_report_runs_no_delete
BEFORE DELETE ON task_report_runs
BEGIN
    SELECT RAISE(ABORT, 'task report runs cannot be deleted');
END;

CREATE TRIGGER task_report_runs_transition_guard
BEFORE UPDATE ON task_report_runs
WHEN OLD.status <> 'started'
  OR NEW.status NOT IN ('succeeded', 'failed')
  OR NEW.id <> OLD.id
  OR NEW.report_date <> OLD.report_date
  OR NEW.actor <> OLD.actor
  OR NEW.candidate_count <> OLD.candidate_count
  OR NEW.prompt_version <> OLD.prompt_version
  OR NEW.model <> OLD.model
  OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'task report run transition is invalid');
END;

CREATE TRIGGER task_report_feedback_no_update
BEFORE UPDATE ON task_report_feedback
BEGIN
    SELECT RAISE(ABORT, 'task report feedback is append-only');
END;

CREATE TRIGGER task_report_feedback_no_delete
BEFORE DELETE ON task_report_feedback
BEGIN
    SELECT RAISE(ABORT, 'task report feedback cannot be deleted');
END;
