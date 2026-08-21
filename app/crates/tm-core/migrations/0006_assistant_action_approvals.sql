CREATE TABLE assistant_action_requests (
    id TEXT PRIMARY KEY,
    operation TEXT NOT NULL CHECK (operation = 'task.create'),
    status TEXT NOT NULL CHECK (
        status IN (
            'pending', 'approved', 'executing', 'completed',
            'rejected', 'cancelled', 'expired', 'failed'
        )
    ),
    revision INTEGER NOT NULL CHECK (revision > 0),
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64),
    origin_request_id TEXT NOT NULL,
    execution_idempotency_key TEXT NOT NULL UNIQUE,
    approval_idempotency_key TEXT UNIQUE,
    result_json TEXT CHECK (result_json IS NULL OR json_valid(result_json)),
    failure_code TEXT,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    approved_at TEXT,
    executing_at TEXT,
    completed_at TEXT,
    terminal_at TEXT
) STRICT;

CREATE INDEX idx_assistant_action_status_created
    ON assistant_action_requests(status, created_at, id);

CREATE INDEX idx_assistant_action_expires
    ON assistant_action_requests(status, expires_at);

CREATE TABLE assistant_action_events (
    id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES assistant_action_requests(id),
    event_type TEXT NOT NULL CHECK (
        event_type IN (
            'proposed', 'approved', 'execution_started', 'completed',
            'rejected', 'cancelled', 'expired', 'failed'
        )
    ),
    from_status TEXT,
    to_status TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    payload_sha256 TEXT NOT NULL CHECK (length(payload_sha256) = 64),
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_assistant_action_events_action_created
    ON assistant_action_events(action_id, created_at, id);

CREATE TRIGGER assistant_action_payload_immutable
BEFORE UPDATE ON assistant_action_requests
WHEN NEW.operation <> OLD.operation
  OR NEW.payload_json <> OLD.payload_json
  OR NEW.payload_sha256 <> OLD.payload_sha256
  OR NEW.origin_request_id <> OLD.origin_request_id
  OR NEW.execution_idempotency_key <> OLD.execution_idempotency_key
  OR NEW.created_at <> OLD.created_at
  OR NEW.expires_at <> OLD.expires_at
BEGIN
    SELECT RAISE(ABORT, 'assistant action payload is immutable');
END;

CREATE TRIGGER assistant_action_revision_cas
BEFORE UPDATE ON assistant_action_requests
WHEN NEW.revision <> OLD.revision + 1
BEGIN
    SELECT RAISE(ABORT, 'assistant action revision must increment by one');
END;

CREATE TRIGGER assistant_action_terminal_immutable
BEFORE UPDATE ON assistant_action_requests
WHEN OLD.status IN ('completed', 'rejected', 'cancelled', 'expired', 'failed')
BEGIN
    SELECT RAISE(ABORT, 'terminal assistant action cannot be changed');
END;

CREATE TRIGGER assistant_action_no_delete
BEFORE DELETE ON assistant_action_requests
BEGIN
    SELECT RAISE(ABORT, 'assistant action requests cannot be deleted');
END;

CREATE TRIGGER assistant_action_events_no_update
BEFORE UPDATE ON assistant_action_events
BEGIN
    SELECT RAISE(ABORT, 'assistant action events are append-only');
END;

CREATE TRIGGER assistant_action_events_no_delete
BEFORE DELETE ON assistant_action_events
BEGIN
    SELECT RAISE(ABORT, 'assistant action events cannot be deleted');
END;
