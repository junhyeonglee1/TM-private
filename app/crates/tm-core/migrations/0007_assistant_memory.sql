CREATE TABLE assistant_memories (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (
        kind IN ('preference', 'goal', 'routine', 'constraint', 'reference', 'summary')
    ),
    title TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 200),
    body TEXT NOT NULL CHECK (length(trim(body)) BETWEEN 1 AND 4000),
    source_type TEXT NOT NULL CHECK (
        source_type IN ('explicit', 'task', 'note', 'worklog', 'session', 'memory_rollup')
    ),
    source_id TEXT,
    provenance_json TEXT NOT NULL CHECK (json_valid(provenance_json)),
    sensitivity TEXT NOT NULL CHECK (sensitivity IN ('normal', 'private', 'restricted')),
    openai_allowed INTEGER NOT NULL CHECK (openai_allowed IN (0, 1)),
    retention TEXT NOT NULL CHECK (
        retention IN ('until_deleted', 'daily_90d', 'weekly_365d', 'monthly_1095d')
    ),
    expires_at TEXT,
    period_kind TEXT CHECK (period_kind IS NULL OR period_kind IN ('daily', 'weekly', 'monthly')),
    period_start TEXT,
    period_end TEXT,
    summary_key TEXT UNIQUE,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    content_sha256 TEXT NOT NULL CHECK (length(content_sha256) = 64),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    CHECK (openai_allowed = 0 OR sensitivity = 'normal'),
    CHECK (
        (source_type = 'explicit' AND source_id IS NULL)
        OR (source_type <> 'explicit' AND source_id IS NOT NULL)
    ),
    CHECK (
        (kind = 'summary' AND period_kind IS NOT NULL AND period_start IS NOT NULL
            AND period_end IS NOT NULL AND summary_key IS NOT NULL)
        OR (kind <> 'summary' AND period_kind IS NULL AND period_start IS NULL
            AND period_end IS NULL AND summary_key IS NULL)
    ),
    CHECK (
        (retention = 'until_deleted' AND expires_at IS NULL)
        OR (retention <> 'until_deleted' AND expires_at IS NOT NULL)
    )
) STRICT;

CREATE INDEX idx_assistant_memories_active_updated
    ON assistant_memories(deleted_at, updated_at DESC, id DESC);

CREATE INDEX idx_assistant_memories_openai
    ON assistant_memories(openai_allowed, sensitivity, expires_at)
    WHERE deleted_at IS NULL;

CREATE INDEX idx_assistant_memories_source
    ON assistant_memories(source_type, source_id)
    WHERE deleted_at IS NULL;

CREATE TABLE assistant_memory_sources (
    memory_id TEXT NOT NULL REFERENCES assistant_memories(id) ON DELETE CASCADE,
    source_type TEXT NOT NULL CHECK (
        source_type IN ('task', 'note', 'worklog', 'session', 'memory')
    ),
    source_id TEXT NOT NULL,
    source_revision INTEGER,
    source_updated_at TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (memory_id, source_type, source_id)
) WITHOUT ROWID, STRICT;

CREATE INDEX idx_assistant_memory_sources_source
    ON assistant_memory_sources(source_type, source_id, memory_id);

CREATE TABLE assistant_memory_events (
    id TEXT PRIMARY KEY,
    memory_id TEXT NOT NULL REFERENCES assistant_memories(id),
    event_type TEXT NOT NULL CHECK (
        event_type IN ('created', 'updated', 'deleted', 'expired', 'source_deleted', 'regenerated')
    ),
    revision INTEGER NOT NULL CHECK (revision > 0),
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    content_sha256 TEXT NOT NULL CHECK (length(content_sha256) = 64),
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_assistant_memory_events_memory_created
    ON assistant_memory_events(memory_id, created_at, id);

CREATE TRIGGER assistant_memory_events_no_update
BEFORE UPDATE ON assistant_memory_events
BEGIN
    SELECT RAISE(ABORT, 'assistant memory events are append-only');
END;

CREATE TRIGGER assistant_memory_events_no_delete
BEFORE DELETE ON assistant_memory_events
BEGIN
    SELECT RAISE(ABORT, 'assistant memory events cannot be deleted');
END;

CREATE VIRTUAL TABLE assistant_memory_search USING fts5(
    memory_id UNINDEXED,
    title,
    body,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER assistant_memory_search_after_insert
AFTER INSERT ON assistant_memories
WHEN NEW.deleted_at IS NULL
BEGIN
    INSERT INTO assistant_memory_search(memory_id, title, body)
    VALUES (NEW.id, NEW.title, NEW.body);
END;

CREATE TRIGGER assistant_memory_search_after_update
AFTER UPDATE OF title, body, deleted_at ON assistant_memories
BEGIN
    DELETE FROM assistant_memory_search WHERE memory_id = OLD.id;
    INSERT INTO assistant_memory_search(memory_id, title, body)
    SELECT NEW.id, NEW.title, NEW.body WHERE NEW.deleted_at IS NULL;
END;

CREATE TRIGGER assistant_memory_search_after_delete
AFTER DELETE ON assistant_memories
BEGIN
    DELETE FROM assistant_memory_search WHERE memory_id = OLD.id;
END;

-- SQLite cannot widen the STEP 12 operation CHECK in-place. Rebuild both
-- approval tables together so existing task approval history stays intact.
DROP INDEX idx_assistant_action_events_action_created;
DROP INDEX idx_assistant_action_status_created;
DROP INDEX idx_assistant_action_expires;
DROP TRIGGER assistant_action_payload_immutable;
DROP TRIGGER assistant_action_revision_cas;
DROP TRIGGER assistant_action_terminal_immutable;
DROP TRIGGER assistant_action_no_delete;
DROP TRIGGER assistant_action_events_no_update;
DROP TRIGGER assistant_action_events_no_delete;

ALTER TABLE assistant_action_events RENAME TO assistant_action_events_step12;
ALTER TABLE assistant_action_requests RENAME TO assistant_action_requests_step12;

CREATE TABLE assistant_action_requests (
    id TEXT PRIMARY KEY,
    operation TEXT NOT NULL CHECK (
        operation IN ('task.create', 'memory.create', 'memory.update', 'memory.delete')
    ),
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

INSERT INTO assistant_action_requests
SELECT * FROM assistant_action_requests_step12;

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

INSERT INTO assistant_action_events
SELECT * FROM assistant_action_events_step12;

CREATE INDEX idx_assistant_action_events_action_created
    ON assistant_action_events(action_id, created_at, id);

DROP TABLE assistant_action_events_step12;
DROP TABLE assistant_action_requests_step12;

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
