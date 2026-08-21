DROP TRIGGER scheduler_jobs_no_delete;
DROP TRIGGER scheduler_runs_identity_immutable;
DROP TRIGGER scheduler_runs_no_delete;
DROP TRIGGER scheduler_attempts_no_update;
DROP TRIGGER scheduler_attempts_no_delete;
DROP TRIGGER scheduler_effects_no_update;
DROP TRIGGER scheduler_effects_no_delete;

CREATE TABLE scheduler_jobs_v17 (
    id TEXT PRIMARY KEY,
    job_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN (
        'scheduler.canary', 'memory.maintenance', 'stock.daily_screen',
        'mail.gmail_watch', 'mail.gmail_reconcile', 'mail.naver_poll',
        'mail.triage', 'mail.digest.morning', 'mail.digest.evening',
        'mail.retention'
    )),
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

CREATE TABLE scheduler_runs_v17 (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES scheduler_jobs_v17(id),
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

CREATE TABLE scheduler_attempts_v17 (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES scheduler_runs_v17(id),
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

CREATE TABLE scheduler_effects_v17 (
    idempotency_key TEXT PRIMARY KEY,
    run_id TEXT NOT NULL UNIQUE REFERENCES scheduler_runs_v17(id),
    job_kind TEXT NOT NULL CHECK (job_kind IN (
        'scheduler.canary', 'memory.maintenance', 'stock.daily_screen',
        'mail.gmail_watch', 'mail.gmail_reconcile', 'mail.naver_poll',
        'mail.triage', 'mail.digest.morning', 'mail.digest.evening',
        'mail.retention'
    )),
    result_json TEXT NOT NULL CHECK (json_valid(result_json)),
    applied_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;

INSERT INTO scheduler_jobs_v17 SELECT * FROM scheduler_jobs;
INSERT INTO scheduler_runs_v17 SELECT * FROM scheduler_runs;
INSERT INTO scheduler_attempts_v17 SELECT * FROM scheduler_attempts;
INSERT INTO scheduler_effects_v17 SELECT * FROM scheduler_effects;

DROP TABLE scheduler_effects;
DROP TABLE scheduler_attempts;
DROP TABLE scheduler_runs;
DROP TABLE scheduler_jobs;

ALTER TABLE scheduler_jobs_v17 RENAME TO scheduler_jobs;
ALTER TABLE scheduler_runs_v17 RENAME TO scheduler_runs;
ALTER TABLE scheduler_attempts_v17 RENAME TO scheduler_attempts;
ALTER TABLE scheduler_effects_v17 RENAME TO scheduler_effects;

CREATE INDEX idx_scheduler_jobs_due ON scheduler_jobs(enabled, next_run_at, id);
CREATE INDEX idx_scheduler_runs_claim ON scheduler_runs(status, available_at, scheduled_for, id);
CREATE INDEX idx_scheduler_runs_lease ON scheduler_runs(status, lease_expires_at)
    WHERE status = 'running';
CREATE INDEX idx_scheduler_runs_completed ON scheduler_runs(status, completed_at DESC, id DESC);
CREATE INDEX idx_scheduler_attempts_run ON scheduler_attempts(run_id, attempt_number);

CREATE TRIGGER scheduler_jobs_no_delete BEFORE DELETE ON scheduler_jobs BEGIN
    SELECT RAISE(ABORT, 'scheduler jobs cannot be deleted');
END;
CREATE TRIGGER scheduler_runs_identity_immutable BEFORE UPDATE ON scheduler_runs
WHEN NEW.job_id <> OLD.job_id OR NEW.scheduled_for <> OLD.scheduled_for
  OR NEW.max_attempts <> OLD.max_attempts OR NEW.idempotency_key <> OLD.idempotency_key
  OR NEW.created_at <> OLD.created_at BEGIN
    SELECT RAISE(ABORT, 'scheduler run identity is immutable');
END;
CREATE TRIGGER scheduler_runs_no_delete BEFORE DELETE ON scheduler_runs BEGIN
    SELECT RAISE(ABORT, 'scheduler runs cannot be deleted');
END;
CREATE TRIGGER scheduler_attempts_no_update BEFORE UPDATE ON scheduler_attempts BEGIN
    SELECT RAISE(ABORT, 'scheduler attempts are append-only');
END;
CREATE TRIGGER scheduler_attempts_no_delete BEFORE DELETE ON scheduler_attempts BEGIN
    SELECT RAISE(ABORT, 'scheduler attempts cannot be deleted');
END;
CREATE TRIGGER scheduler_effects_no_update BEFORE UPDATE ON scheduler_effects BEGIN
    SELECT RAISE(ABORT, 'scheduler effects are immutable');
END;
CREATE TRIGGER scheduler_effects_no_delete BEFORE DELETE ON scheduler_effects BEGIN
    SELECT RAISE(ABORT, 'scheduler effects are immutable');
END;

CREATE TABLE mail_crypto_metadata (
    singleton_key TEXT PRIMARY KEY CHECK (singleton_key = 'mail-data-key-probe'),
    key_version INTEGER NOT NULL CHECK (key_version = 1),
    probe_ciphertext TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_accounts (
    id TEXT PRIMARY KEY,
    provider TEXT NOT NULL CHECK (provider IN ('gmail', 'naver')),
    email_ciphertext TEXT NOT NULL,
    email_blind_index TEXT NOT NULL CHECK (length(email_blind_index) = 64),
    display_name_ciphertext TEXT,
    status TEXT NOT NULL CHECK (status IN ('connected', 'reconnect_required', 'disabled')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    disconnected_at TEXT,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE(provider, email_blind_index)
) STRICT;

CREATE TABLE mail_credentials (
    account_id TEXT PRIMARY KEY REFERENCES mail_accounts(id),
    credential_kind TEXT NOT NULL CHECK (credential_kind IN ('oauth_refresh_token', 'app_password')),
    secret_ciphertext TEXT NOT NULL,
    scope TEXT,
    token_expires_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0)
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_sync_state (
    account_id TEXT PRIMARY KEY REFERENCES mail_accounts(id),
    cursor_ciphertext TEXT,
    uid_validity TEXT,
    watch_expires_at TEXT,
    last_attempt_at TEXT,
    last_success_at TEXT,
    next_expected_at TEXT,
    last_error_code TEXT CHECK (last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 80),
    consecutive_failures INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_failures >= 0),
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0)
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_items (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES mail_accounts(id),
    provider_message_sha256 TEXT NOT NULL CHECK (length(provider_message_sha256) = 64),
    provider_message_ciphertext TEXT NOT NULL,
    thread_sha256 TEXT CHECK (thread_sha256 IS NULL OR length(thread_sha256) = 64),
    received_at TEXT NOT NULL,
    sender_ciphertext TEXT NOT NULL,
    sender_domain_ciphertext TEXT,
    sender_domain_blind_index TEXT CHECK (
        sender_domain_blind_index IS NULL OR length(sender_domain_blind_index) = 64
    ),
    subject_ciphertext TEXT NOT NULL,
    summary_ciphertext TEXT,
    action_ciphertext TEXT,
    deadline TEXT,
    classification TEXT NOT NULL CHECK (
        classification IN ('important', 'review', 'not_important', 'excluded')
    ),
    importance_score INTEGER NOT NULL CHECK (importance_score BETWEEN 0 AND 100),
    confidence INTEGER NOT NULL CHECK (confidence BETWEEN 0 AND 100),
    decision_source TEXT NOT NULL CHECK (decision_source IN ('rule', 'ai', 'manual')),
    decision_reason TEXT NOT NULL CHECK (length(decision_reason) BETWEEN 1 AND 80),
    sensitive_kind TEXT CHECK (sensitive_kind IS NULL OR sensitive_kind IN ('otp', 'password_reset', 'security_alert')),
    acknowledged_at TEXT,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE(account_id, provider_message_sha256)
) STRICT;

CREATE INDEX idx_mail_items_queue
    ON mail_items(classification, acknowledged_at, received_at DESC, id DESC);
CREATE INDEX idx_mail_items_account
    ON mail_items(account_id, received_at DESC, id DESC);
CREATE INDEX idx_mail_items_expiry ON mail_items(expires_at, id);

CREATE TABLE mail_feedback (
    id TEXT PRIMARY KEY,
    mail_item_id TEXT NOT NULL REFERENCES mail_items(id),
    important INTEGER NOT NULL CHECK (important IN (0, 1)),
    actor TEXT NOT NULL CHECK (length(actor) BETWEEN 1 AND 160),
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX idx_mail_feedback_item ON mail_feedback(mail_item_id, created_at DESC, id DESC);

CREATE TABLE mail_rules (
    id TEXT PRIMARY KEY,
    rule_kind TEXT NOT NULL CHECK (rule_kind IN ('vip_sender', 'sender_domain', 'subject_keyword')),
    pattern_ciphertext TEXT NOT NULL,
    pattern_blind_index TEXT NOT NULL CHECK (length(pattern_blind_index) = 64),
    label_ciphertext TEXT NOT NULL,
    importance_score INTEGER NOT NULL CHECK (importance_score BETWEEN 0 AND 100),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    UNIQUE(rule_kind, pattern_blind_index)
) STRICT;

CREATE TABLE mail_oauth_states (
    state_sha256 TEXT PRIMARY KEY CHECK (length(state_sha256) = 64),
    pkce_verifier_ciphertext TEXT NOT NULL,
    return_uri_ciphertext TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;
CREATE INDEX idx_mail_oauth_states_expiry ON mail_oauth_states(expires_at, consumed_at);

CREATE TABLE mail_webhook_events (
    message_id_sha256 TEXT PRIMARY KEY CHECK (length(message_id_sha256) = 64),
    account_hint_sha256 TEXT CHECK (account_hint_sha256 IS NULL OR length(account_hint_sha256) = 64),
    published_at TEXT,
    status TEXT NOT NULL CHECK (status IN ('accepted', 'ignored', 'processed', 'failed')),
    created_at TEXT NOT NULL,
    processed_at TEXT
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_triage_batches (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL UNIQUE,
    input_sha256 TEXT NOT NULL CHECK (length(input_sha256) = 64),
    prompt_version TEXT NOT NULL CHECK (length(prompt_version) BETWEEN 1 AND 64),
    model TEXT NOT NULL CHECK (length(model) BETWEEN 1 AND 128),
    status TEXT NOT NULL CHECK (status IN ('claimed', 'succeeded', 'failed', 'uncertain')),
    item_count INTEGER NOT NULL CHECK (item_count BETWEEN 1 AND 10),
    response_id TEXT,
    upstream_request_id TEXT,
    input_tokens INTEGER,
    cached_input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK (cost_microusd >= 0),
    failure_code TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    CHECK ((status = 'claimed' AND completed_at IS NULL) OR (status <> 'claimed' AND completed_at IS NOT NULL))
) STRICT;
CREATE INDEX idx_mail_triage_batches_quota ON mail_triage_batches(created_at, status);

CREATE TABLE mail_triage_items (
    batch_id TEXT NOT NULL REFERENCES mail_triage_batches(id),
    mail_item_id TEXT NOT NULL,
    expected_version INTEGER NOT NULL CHECK (expected_version > 0),
    importance_score INTEGER CHECK (importance_score IS NULL OR importance_score BETWEEN 0 AND 100),
    confidence INTEGER CHECK (confidence IS NULL OR confidence BETWEEN 0 AND 100),
    classification TEXT CHECK (classification IS NULL OR classification IN ('important', 'review', 'not_important')),
    reason_code TEXT,
    PRIMARY KEY(batch_id, mail_item_id)
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_reports (
    id TEXT PRIMARY KEY,
    report_date TEXT NOT NULL,
    slot TEXT NOT NULL CHECK (slot IN ('morning', 'evening')),
    summary_ciphertext TEXT NOT NULL,
    important_count INTEGER NOT NULL CHECK (important_count >= 0),
    review_count INTEGER NOT NULL CHECK (review_count >= 0),
    generated_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    UNIQUE(report_date, slot)
) STRICT;
CREATE INDEX idx_mail_reports_date ON mail_reports(report_date DESC, slot);

CREATE TABLE mail_report_items (
    report_id TEXT NOT NULL REFERENCES mail_reports(id),
    mail_item_id TEXT NOT NULL REFERENCES mail_items(id),
    PRIMARY KEY(report_id, mail_item_id)
) WITHOUT ROWID, STRICT;

CREATE TABLE mail_sync_events (
    id TEXT PRIMARY KEY,
    account_id TEXT REFERENCES mail_accounts(id),
    provider TEXT NOT NULL CHECK (provider IN ('gmail', 'naver')),
    operation TEXT NOT NULL CHECK (operation IN ('watch', 'reconcile', 'poll', 'push', 'retention')),
    status TEXT NOT NULL CHECK (status IN ('started', 'succeeded', 'failed', 'skipped')),
    detected_count INTEGER NOT NULL DEFAULT 0 CHECK (detected_count >= 0),
    changed_provider_state INTEGER NOT NULL DEFAULT 0 CHECK (changed_provider_state = 0),
    error_code TEXT,
    started_at TEXT NOT NULL,
    completed_at TEXT,
    CHECK ((status = 'started' AND completed_at IS NULL) OR (status <> 'started' AND completed_at IS NOT NULL))
) STRICT;
CREATE INDEX idx_mail_sync_events_account ON mail_sync_events(account_id, started_at DESC, id DESC);

CREATE TABLE mail_mutation_receipts (
    idempotency_key TEXT PRIMARY KEY,
    operation TEXT NOT NULL CHECK (length(operation) BETWEEN 1 AND 80),
    request_sha256 TEXT NOT NULL CHECK (length(request_sha256) = 64),
    response_json TEXT NOT NULL CHECK (json_valid(response_json)),
    created_at TEXT NOT NULL
) WITHOUT ROWID, STRICT;

CREATE TRIGGER mail_feedback_immutable_update BEFORE UPDATE ON mail_feedback BEGIN
    SELECT RAISE(ABORT, 'mail feedback is append-only');
END;
CREATE TRIGGER mail_webhook_events_immutable_delete BEFORE DELETE ON mail_webhook_events BEGIN
    SELECT RAISE(ABORT, 'mail webhook events are durable');
END;
CREATE TRIGGER mail_triage_batches_identity_immutable BEFORE UPDATE ON mail_triage_batches
WHEN NEW.id <> OLD.id OR NEW.request_id <> OLD.request_id OR NEW.input_sha256 <> OLD.input_sha256
  OR NEW.prompt_version <> OLD.prompt_version OR NEW.model <> OLD.model OR NEW.created_at <> OLD.created_at
  OR OLD.status <> 'claimed' BEGIN
    SELECT RAISE(ABORT, 'mail triage batch is immutable after completion');
END;
CREATE TRIGGER mail_triage_batches_no_delete BEFORE DELETE ON mail_triage_batches BEGIN
    SELECT RAISE(ABORT, 'mail triage batches cannot be deleted');
END;
CREATE TRIGGER mail_triage_items_guarded_completion BEFORE UPDATE ON mail_triage_items
WHEN NEW.batch_id <> OLD.batch_id
  OR NEW.mail_item_id <> OLD.mail_item_id
  OR NEW.expected_version <> OLD.expected_version
  OR OLD.importance_score IS NOT NULL
  OR OLD.confidence IS NOT NULL
  OR OLD.classification IS NOT NULL
  OR OLD.reason_code IS NOT NULL
  OR NEW.importance_score IS NULL
  OR NEW.confidence IS NULL
  OR NEW.classification IS NULL
  OR NEW.reason_code IS NULL
  OR NOT EXISTS(
      SELECT 1 FROM mail_triage_batches AS batch
      WHERE batch.id = OLD.batch_id AND batch.status = 'claimed'
  ) BEGIN
    SELECT RAISE(ABORT, 'mail triage item completion is invalid');
END;
CREATE TRIGGER mail_triage_items_no_delete BEFORE DELETE ON mail_triage_items BEGIN
    SELECT RAISE(ABORT, 'mail triage items are immutable');
END;
CREATE TRIGGER mail_mutation_receipts_no_update BEFORE UPDATE ON mail_mutation_receipts BEGIN
    SELECT RAISE(ABORT, 'mail mutation receipts are immutable');
END;
CREATE TRIGGER mail_mutation_receipts_no_delete BEFORE DELETE ON mail_mutation_receipts BEGIN
    SELECT RAISE(ABORT, 'mail mutation receipts are immutable');
END;
