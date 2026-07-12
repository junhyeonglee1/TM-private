-- Tighten optimistic concurrency for approval returns and preserve complete
-- before/after content snapshots in the append-only event stream. Recreating
-- these triggers makes an already-opened schema 2 database behave exactly like
-- a fresh database after it reaches schema 3.

DROP TRIGGER IF EXISTS change_requests_revision_guard;
DROP TRIGGER IF EXISTS change_requests_event_after_insert;
DROP TRIGGER IF EXISTS change_requests_event_after_update;

CREATE TRIGGER change_requests_revision_guard
BEFORE UPDATE ON change_requests
WHEN NOT EXISTS (SELECT 1 FROM app_state WHERE key = 'change_request_restore')
AND (
     (
       (
            (OLD.status = 'draft' AND NEW.status = 'draft')
         OR (OLD.status IN ('approved', 'failed') AND NEW.status = 'draft')
       )
       AND NEW.revision <> OLD.revision + 1
     )
  OR (
       NOT (
            (OLD.status = 'draft' AND NEW.status = 'draft')
         OR (OLD.status IN ('approved', 'failed') AND NEW.status = 'draft')
       )
       AND NEW.revision <> OLD.revision
     )
  OR (OLD.status = 'approved' AND NEW.status = 'claimed' AND NEW.attempt_count <> OLD.attempt_count + 1)
  OR (NOT (OLD.status = 'approved' AND NEW.status = 'claimed') AND NEW.attempt_count <> OLD.attempt_count)
)
BEGIN
    SELECT RAISE(ABORT, 'invalid change request revision or attempt count');
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
            'snapshot', json_object(
                'kind', NEW.kind,
                'projectId', NEW.project_id,
                'taskId', NEW.task_id,
                'title', NEW.title,
                'description', NEW.description,
                'desiredOutcome', NEW.desired_outcome,
                'reproductionSteps', NEW.reproduction_steps,
                'priority', NEW.priority
            )
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
        CASE
            WHEN OLD.status = 'draft' AND NEW.status = 'draft' THEN
                json_object(
                    'before', json_object(
                        'kind', OLD.kind,
                        'projectId', OLD.project_id,
                        'taskId', OLD.task_id,
                        'title', OLD.title,
                        'description', OLD.description,
                        'desiredOutcome', OLD.desired_outcome,
                        'reproductionSteps', OLD.reproduction_steps,
                        'priority', OLD.priority
                    ),
                    'after', json_object(
                        'kind', NEW.kind,
                        'projectId', NEW.project_id,
                        'taskId', NEW.task_id,
                        'title', NEW.title,
                        'description', NEW.description,
                        'desiredOutcome', NEW.desired_outcome,
                        'reproductionSteps', NEW.reproduction_steps,
                        'priority', NEW.priority
                    )
                )
            ELSE
                json_object(
                    'approvedRevision', NEW.approved_revision,
                    'attemptCount', NEW.attempt_count,
                    'workerId', NEW.claimed_by,
                    'resultSummary', NEW.result_summary,
                    'patchRef', NEW.patch_ref,
                    'failureReason', NEW.failure_reason,
                    'cancellationReason', NEW.cancellation_reason
                )
        END,
        tm_now_utc()
    );
END;
