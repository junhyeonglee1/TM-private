CREATE TABLE change_requests (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('bug', 'ui', 'feature', 'other')),
    project_id TEXT REFERENCES projects(id),
    task_id TEXT REFERENCES tasks(id),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    description TEXT NOT NULL DEFAULT '',
    desired_outcome TEXT NOT NULL DEFAULT '',
    reproduction_steps TEXT NOT NULL DEFAULT '',
    priority INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 3),
    status TEXT NOT NULL CHECK (
        status IN ('draft', 'approved', 'claimed', 'completed', 'failed', 'cancelled')
    ),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    approved_revision INTEGER,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    requested_by TEXT NOT NULL CHECK (length(trim(requested_by)) > 0),
    updated_by TEXT NOT NULL CHECK (length(trim(updated_by)) > 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    approved_at TEXT,
    approved_by TEXT,
    claim_key TEXT UNIQUE,
    claimed_at TEXT,
    claimed_by TEXT,
    completed_at TEXT,
    completed_by TEXT,
    failed_at TEXT,
    failed_by TEXT,
    cancelled_at TEXT,
    cancelled_by TEXT,
    result_summary TEXT,
    patch_ref TEXT,
    failure_reason TEXT,
    cancellation_reason TEXT,
    CHECK (
        (status = 'draft'
         AND approved_revision IS NULL AND approved_at IS NULL AND approved_by IS NULL
         AND claim_key IS NULL AND claimed_at IS NULL AND claimed_by IS NULL
         AND completed_at IS NULL AND completed_by IS NULL
         AND failed_at IS NULL AND failed_by IS NULL
         AND cancelled_at IS NULL AND cancelled_by IS NULL
         AND result_summary IS NULL AND patch_ref IS NULL
         AND failure_reason IS NULL AND cancellation_reason IS NULL)
        OR
        (status = 'approved'
         AND approved_revision = revision AND approved_at IS NOT NULL AND approved_by IS NOT NULL
         AND claim_key IS NULL AND claimed_at IS NULL AND claimed_by IS NULL
         AND completed_at IS NULL AND completed_by IS NULL
         AND failed_at IS NULL AND failed_by IS NULL
         AND cancelled_at IS NULL AND cancelled_by IS NULL
         AND result_summary IS NULL AND patch_ref IS NULL
         AND failure_reason IS NULL AND cancellation_reason IS NULL)
        OR
        (status = 'claimed'
         AND approved_revision = revision AND approved_at IS NOT NULL AND approved_by IS NOT NULL
         AND claim_key IS NOT NULL AND claimed_at IS NOT NULL AND claimed_by IS NOT NULL
         AND completed_at IS NULL AND completed_by IS NULL
         AND failed_at IS NULL AND failed_by IS NULL
         AND cancelled_at IS NULL AND cancelled_by IS NULL
         AND result_summary IS NULL AND patch_ref IS NULL
         AND failure_reason IS NULL AND cancellation_reason IS NULL)
        OR
        (status = 'completed'
         AND approved_revision = revision AND approved_at IS NOT NULL AND approved_by IS NOT NULL
         AND claim_key IS NOT NULL AND claimed_at IS NOT NULL AND claimed_by IS NOT NULL
         AND completed_at IS NOT NULL AND completed_by IS NOT NULL
         AND failed_at IS NULL AND failed_by IS NULL
         AND cancelled_at IS NULL AND cancelled_by IS NULL
         AND result_summary IS NOT NULL AND length(trim(result_summary)) > 0
         AND failure_reason IS NULL AND cancellation_reason IS NULL)
        OR
        (status = 'failed'
         AND approved_revision = revision AND approved_at IS NOT NULL AND approved_by IS NOT NULL
         AND claim_key IS NOT NULL AND claimed_at IS NOT NULL AND claimed_by IS NOT NULL
         AND completed_at IS NULL AND completed_by IS NULL
         AND failed_at IS NOT NULL AND failed_by IS NOT NULL
         AND cancelled_at IS NULL AND cancelled_by IS NULL
         AND result_summary IS NULL AND patch_ref IS NULL
         AND failure_reason IS NOT NULL AND length(trim(failure_reason)) > 0
         AND cancellation_reason IS NULL)
        OR
        (status = 'cancelled'
         AND approved_revision IS NULL AND approved_at IS NULL AND approved_by IS NULL
         AND claim_key IS NULL AND claimed_at IS NULL AND claimed_by IS NULL
         AND completed_at IS NULL AND completed_by IS NULL
         AND failed_at IS NULL AND failed_by IS NULL
         AND cancelled_at IS NOT NULL AND cancelled_by IS NOT NULL
         AND result_summary IS NULL AND patch_ref IS NULL AND failure_reason IS NULL
         AND cancellation_reason IS NOT NULL AND length(trim(cancellation_reason)) > 0)
    )
) STRICT;

CREATE TABLE change_request_events (
    id TEXT PRIMARY KEY,
    change_request_id TEXT NOT NULL REFERENCES change_requests(id),
    event_type TEXT NOT NULL,
    from_status TEXT,
    to_status TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    actor TEXT NOT NULL CHECK (length(trim(actor)) > 0),
    details_json TEXT NOT NULL CHECK (json_valid(details_json)),
    created_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_change_requests_status_updated
    ON change_requests(status, updated_at DESC);

CREATE INDEX idx_change_requests_claim_queue
    ON change_requests(priority DESC, approved_at ASC, created_at ASC, id ASC)
    WHERE status = 'approved';

CREATE INDEX idx_change_request_events_request_created
    ON change_request_events(change_request_id, created_at ASC, id ASC);

CREATE TRIGGER change_requests_insert_must_be_draft
BEFORE INSERT ON change_requests
WHEN (NEW.status <> 'draft' OR NEW.revision <> 1 OR NEW.attempt_count <> 0)
 AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    SELECT RAISE(ABORT, 'change requests must be created as revision 1 drafts');
END;

CREATE TRIGGER change_requests_no_delete
BEFORE DELETE ON change_requests
BEGIN
    SELECT RAISE(ABORT, 'change requests cannot be deleted; cancel them instead');
END;

CREATE TRIGGER change_requests_identity_immutable
BEFORE UPDATE ON change_requests
WHEN (
       OLD.id IS NOT NEW.id
    OR OLD.requested_by IS NOT NEW.requested_by
    OR OLD.created_at IS NOT NEW.created_at
)
AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    SELECT RAISE(ABORT, 'change request identity is immutable');
END;

CREATE TRIGGER change_requests_terminal_immutable
BEFORE UPDATE ON change_requests
WHEN OLD.status IN ('completed', 'cancelled')
 AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    SELECT RAISE(ABORT, 'completed and cancelled change requests are immutable');
END;

CREATE TRIGGER change_requests_non_draft_update_guard
BEFORE UPDATE ON change_requests
WHEN OLD.status = NEW.status
 AND OLD.status <> 'draft'
 AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    SELECT RAISE(ABORT, 'non-draft change requests require an explicit status transition');
END;

CREATE TRIGGER change_requests_transition_guard
BEFORE UPDATE OF status ON change_requests
WHEN OLD.status <> NEW.status
 AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
 AND NOT (
      (OLD.status = 'draft' AND NEW.status IN ('approved', 'cancelled'))
   OR (OLD.status = 'approved' AND NEW.status IN ('draft', 'claimed', 'cancelled'))
   OR (OLD.status = 'claimed' AND NEW.status IN ('completed', 'failed'))
   OR (OLD.status = 'failed' AND NEW.status IN ('draft', 'approved', 'cancelled'))
 )
BEGIN
    SELECT RAISE(ABORT, 'invalid change request status transition');
END;

CREATE TRIGGER change_requests_revision_guard
BEFORE UPDATE ON change_requests
WHEN NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
AND (
     (OLD.status = 'draft' AND NEW.status = 'draft' AND NEW.revision <> OLD.revision + 1)
  OR (NOT (OLD.status = 'draft' AND NEW.status = 'draft') AND NEW.revision <> OLD.revision)
  OR (OLD.status = 'approved' AND NEW.status = 'claimed' AND NEW.attempt_count <> OLD.attempt_count + 1)
  OR (NOT (OLD.status = 'approved' AND NEW.status = 'claimed') AND NEW.attempt_count <> OLD.attempt_count)
)
BEGIN
    SELECT RAISE(ABORT, 'invalid change request revision or attempt count');
END;

CREATE TRIGGER change_requests_content_guard
BEFORE UPDATE ON change_requests
WHEN (
       OLD.kind IS NOT NEW.kind
    OR OLD.project_id IS NOT NEW.project_id
    OR OLD.task_id IS NOT NEW.task_id
    OR OLD.title IS NOT NEW.title
    OR OLD.description IS NOT NEW.description
    OR OLD.desired_outcome IS NOT NEW.desired_outcome
    OR OLD.reproduction_steps IS NOT NEW.reproduction_steps
    OR OLD.priority IS NOT NEW.priority
)
AND NOT (OLD.status = 'draft' AND NEW.status = 'draft')
AND NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    SELECT RAISE(ABORT, 'only draft change requests can be edited');
END;

CREATE TRIGGER change_request_events_no_update
BEFORE UPDATE ON change_request_events
BEGIN
    SELECT RAISE(ABORT, 'change request events are append-only');
END;

CREATE TRIGGER change_request_events_no_delete
BEFORE DELETE ON change_request_events
BEGIN
    SELECT RAISE(ABORT, 'change request events are append-only');
END;

CREATE TRIGGER change_requests_event_after_insert
AFTER INSERT ON change_requests
WHEN NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    INSERT INTO change_request_events(
        id, change_request_id, event_type, from_status, to_status,
        revision, actor, details_json, created_at
    ) VALUES (
        tm_uuid_v7(), NEW.id, 'created', NULL, NEW.status,
        NEW.revision, NEW.updated_by,
        json_object(
            'kind', NEW.kind,
            'projectId', NEW.project_id,
            'taskId', NEW.task_id,
            'title', NEW.title,
            'priority', NEW.priority
        ),
        tm_now_utc()
    );
END;

CREATE TRIGGER change_requests_event_after_update
AFTER UPDATE ON change_requests
WHEN NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
BEGIN
    INSERT INTO change_request_events(
        id, change_request_id, event_type, from_status, to_status,
        revision, actor, details_json, created_at
    ) VALUES (
        tm_uuid_v7(), NEW.id,
        CASE
            WHEN OLD.status = NEW.status AND NEW.status = 'draft' THEN 'draft_updated'
            WHEN NEW.status = 'approved' AND OLD.status = 'failed' THEN 'reapproved'
            WHEN NEW.status = 'approved' THEN 'approved'
            WHEN NEW.status = 'draft' AND OLD.status = 'failed' THEN 'failed_returned_to_draft'
            WHEN NEW.status = 'draft' THEN 'approval_returned'
            WHEN NEW.status = 'claimed' THEN 'claimed'
            WHEN NEW.status = 'completed' THEN 'completed'
            WHEN NEW.status = 'failed' AND NEW.failed_by = NEW.claimed_by THEN 'failed'
            WHEN NEW.status = 'failed' THEN 'abandoned'
            WHEN NEW.status = 'cancelled' THEN 'cancelled'
            ELSE 'updated'
        END,
        OLD.status, NEW.status, NEW.revision, NEW.updated_by,
        json_object(
            'approvedRevision', NEW.approved_revision,
            'attemptCount', NEW.attempt_count,
            'workerId', NEW.claimed_by,
            'resultSummary', NEW.result_summary,
            'patchRef', NEW.patch_ref,
            'failureReason', NEW.failure_reason,
            'cancellationReason', NEW.cancellation_reason
        ),
        tm_now_utc()
    );
END;
