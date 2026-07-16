ALTER TABLE tasks
ADD COLUMN version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0);

ALTER TABLE checklist_items
ADD COLUMN version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0);

ALTER TABLE notes
ADD COLUMN version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0);

CREATE TABLE mutation_idempotency_records (
    idempotency_key TEXT PRIMARY KEY CHECK (
        length(idempotency_key) BETWEEN 16 AND 128
    ),
    operation TEXT NOT NULL,
    request_sha256 TEXT NOT NULL CHECK (length(request_sha256) = 64),
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    resource_version INTEGER NOT NULL CHECK (resource_version > 0),
    response_json TEXT NOT NULL CHECK (json_valid(response_json)),
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE mutation_audit_events (
    id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE
        REFERENCES mutation_idempotency_records(idempotency_key),
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    operation TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    expected_version TEXT NOT NULL,
    resulting_version INTEGER NOT NULL CHECK (resulting_version > 0),
    before_json TEXT,
    after_json TEXT NOT NULL CHECK (json_valid(after_json)),
    result TEXT NOT NULL CHECK (result = 'completed'),
    approval_policy TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK (before_json IS NULL OR json_valid(before_json))
) STRICT;

CREATE INDEX idx_mutation_audit_resource_created
    ON mutation_audit_events(resource_type, resource_id, created_at, id);

CREATE INDEX idx_mutation_audit_request
    ON mutation_audit_events(request_id);

CREATE TRIGGER mutation_idempotency_no_update
BEFORE UPDATE ON mutation_idempotency_records
BEGIN
    SELECT RAISE(ABORT, 'mutation idempotency records are append-only');
END;

CREATE TRIGGER mutation_idempotency_no_delete
BEFORE DELETE ON mutation_idempotency_records
BEGIN
    SELECT RAISE(ABORT, 'mutation idempotency records cannot be deleted');
END;

CREATE TRIGGER mutation_audit_no_update
BEFORE UPDATE ON mutation_audit_events
BEGIN
    SELECT RAISE(ABORT, 'mutation audit events are append-only');
END;

CREATE TRIGGER mutation_audit_no_delete
BEFORE DELETE ON mutation_audit_events
BEGIN
    SELECT RAISE(ABORT, 'mutation audit events cannot be deleted');
END;
