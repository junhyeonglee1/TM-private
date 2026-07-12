use std::str::FromStr;

use crate::{
    ChangeRequest, ChangeRequestClaim, ChangeRequestEvent, CreateChangeRequestInput, Error, Result,
    UpdateChangeRequestInput,
    database::{Database, new_id, now_utc},
    error::{invalid, not_found},
};
use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params, types::Type,
};

const REQUEST_COLUMNS: &str = "id, kind, project_id, task_id, title, description, desired_outcome,
     reproduction_steps, priority, status, revision, approved_revision, attempt_count,
     requested_by, updated_by, created_at, updated_at, approved_at, approved_by,
     claimed_at, claimed_by, completed_at, completed_by, failed_at, failed_by,
     cancelled_at, cancelled_by, result_summary, patch_ref, failure_reason,
     cancellation_reason";

pub(crate) fn create(
    database: &Database,
    input: CreateChangeRequestInput,
) -> Result<ChangeRequest> {
    validate_content(
        &input.title,
        &input.description,
        &input.desired_outcome,
        input.priority,
        &input.requested_by,
        "requestedBy",
    )?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let id = new_id();
        let now = now_utc();
        transaction.execute(
            "INSERT INTO change_requests(
                id, kind, project_id, task_id, title, description, desired_outcome,
                reproduction_steps, priority, status, revision, attempt_count,
                requested_by, updated_by, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
                'draft', 1, 0, ?10, ?10, ?11, ?11
             )",
            params![
                id,
                input.kind.as_str(),
                input.project_id,
                input.task_id,
                input.title.trim(),
                input.description.trim(),
                input.desired_outcome.trim(),
                input.reproduction_steps.trim(),
                input.priority,
                input.requested_by.trim(),
                now,
            ],
        )?;
        query_request(transaction, &id)
    })
}

pub(crate) fn update(
    database: &Database,
    request_id: &str,
    input: UpdateChangeRequestInput,
) -> Result<ChangeRequest> {
    validate_content(
        &input.title,
        &input.description,
        &input.desired_outcome,
        input.priority,
        &input.updated_by,
        "updatedBy",
    )?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let changed = transaction.execute(
            "UPDATE change_requests
             SET kind = ?3, project_id = ?4, task_id = ?5, title = ?6,
                 description = ?7, desired_outcome = ?8, reproduction_steps = ?9,
                 priority = ?10, revision = revision + 1,
                 updated_by = ?11, updated_at = ?12
             WHERE id = ?1 AND status = 'draft' AND revision = ?2
               AND attempt_count = ?13",
            params![
                request_id,
                input.expected_revision,
                input.kind.as_str(),
                input.project_id,
                input.task_id,
                input.title.trim(),
                input.description.trim(),
                input.desired_outcome.trim(),
                input.reproduction_steps.trim(),
                input.priority,
                input.updated_by.trim(),
                now_utc(),
                input.expected_attempt_count,
            ],
        )?;
        require_one_or_state_error(
            transaction,
            request_id,
            input.expected_revision,
            input.expected_attempt_count,
            changed,
            "only the expected draft revision can be updated",
        )?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn approve(
    database: &Database,
    request_id: &str,
    expected_revision: u32,
    expected_attempt_count: u32,
    approved_by: &str,
) -> Result<ChangeRequest> {
    let approved_by = required_text("approvedBy", approved_by)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'approved', approved_revision = revision,
                 approved_at = ?3, approved_by = ?4,
                 claim_key = NULL, claimed_at = NULL, claimed_by = NULL,
                 failed_at = NULL, failed_by = NULL, failure_reason = NULL,
                 updated_by = ?4, updated_at = ?3
             WHERE id = ?1 AND revision = ?2 AND status IN ('draft', 'failed')
               AND attempt_count = ?5",
            params![
                request_id,
                expected_revision,
                now,
                approved_by,
                expected_attempt_count,
            ],
        )?;
        require_one_or_state_error(
            transaction,
            request_id,
            expected_revision,
            expected_attempt_count,
            changed,
            "only the expected draft or failed revision can be approved",
        )?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn return_to_draft(
    database: &Database,
    request_id: &str,
    expected_revision: u32,
    expected_attempt_count: u32,
    returned_by: &str,
) -> Result<ChangeRequest> {
    let returned_by = required_text("returnedBy", returned_by)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'draft', revision = revision + 1, approved_revision = NULL,
                 approved_at = NULL, approved_by = NULL,
                 claim_key = NULL, claimed_at = NULL, claimed_by = NULL,
                 failed_at = NULL, failed_by = NULL, failure_reason = NULL,
                 updated_by = ?3, updated_at = ?4
             WHERE id = ?1 AND revision = ?2 AND status IN ('approved', 'failed')
               AND attempt_count = ?5",
            params![
                request_id,
                expected_revision,
                returned_by,
                now_utc(),
                expected_attempt_count,
            ],
        )?;
        require_one_or_state_error(
            transaction,
            request_id,
            expected_revision,
            expected_attempt_count,
            changed,
            "only the expected approved or failed revision can return to draft",
        )?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn cancel(
    database: &Database,
    request_id: &str,
    expected_revision: u32,
    expected_attempt_count: u32,
    cancellation_reason: &str,
    cancelled_by: &str,
) -> Result<ChangeRequest> {
    let cancellation_reason = required_text("cancellationReason", cancellation_reason)?;
    let cancelled_by = required_text("cancelledBy", cancelled_by)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'cancelled', approved_revision = NULL,
                 approved_at = NULL, approved_by = NULL,
                 claim_key = NULL, claimed_at = NULL, claimed_by = NULL,
                 failed_at = NULL, failed_by = NULL, failure_reason = NULL,
                 cancelled_at = ?3, cancelled_by = ?4, cancellation_reason = ?5,
                 updated_by = ?4, updated_at = ?3
             WHERE id = ?1 AND revision = ?2
               AND status IN ('draft', 'approved', 'failed')
               AND attempt_count = ?6",
            params![
                request_id,
                expected_revision,
                now,
                cancelled_by,
                cancellation_reason,
                expected_attempt_count,
            ],
        )?;
        require_one_or_state_error(
            transaction,
            request_id,
            expected_revision,
            expected_attempt_count,
            changed,
            "only the expected draft, approved, or failed revision can be cancelled",
        )?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn claim_next(database: &Database, worker_id: &str) -> Result<ChangeRequestClaim> {
    let worker_id = required_text("workerId", worker_id)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let request_id: Option<String> = transaction
            .query_row(
                "SELECT id FROM change_requests
                 WHERE status = 'approved'
                 ORDER BY priority DESC, approved_at ASC, created_at ASC, id ASC
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(request_id) = request_id else {
            return Ok(ChangeRequestClaim {
                should_process: false,
                request: None,
                claim_key: None,
            });
        };
        let claim_key = new_id();
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'claimed', claim_key = ?2, claimed_at = ?3, claimed_by = ?4,
                 attempt_count = attempt_count + 1, updated_by = ?4, updated_at = ?3
             WHERE id = ?1 AND status = 'approved'",
            params![request_id, claim_key, now, worker_id],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(
                "change request was claimed concurrently".to_owned(),
            ));
        }
        Ok(ChangeRequestClaim {
            should_process: true,
            request: Some(query_request(transaction, &request_id)?),
            claim_key: Some(claim_key),
        })
    })
}

pub(crate) fn complete(
    database: &Database,
    request_id: &str,
    claim_key: &str,
    result_summary: &str,
    patch_ref: Option<&str>,
    worker_id: &str,
) -> Result<ChangeRequest> {
    let claim_key = required_text("claimKey", claim_key)?;
    let result_summary = required_text("resultSummary", result_summary)?;
    let worker_id = required_text("workerId", worker_id)?;
    let patch_ref = patch_ref
        .map(|value| required_text("patchRef", value))
        .transpose()?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'completed', completed_at = ?4, completed_by = ?5,
                 result_summary = ?6, patch_ref = ?7, updated_by = ?5, updated_at = ?4
             WHERE id = ?1 AND status = 'claimed' AND claim_key = ?2 AND claimed_by = ?3",
            params![
                request_id,
                claim_key,
                worker_id,
                now,
                worker_id,
                result_summary,
                patch_ref,
            ],
        )?;
        require_claim_update(transaction, request_id, changed)?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn fail(
    database: &Database,
    request_id: &str,
    claim_key: &str,
    failure_reason: &str,
    worker_id: &str,
) -> Result<ChangeRequest> {
    let claim_key = required_text("claimKey", claim_key)?;
    let failure_reason = required_text("failureReason", failure_reason)?;
    let worker_id = required_text("workerId", worker_id)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'failed', failed_at = ?4, failed_by = ?5,
                 failure_reason = ?6, updated_by = ?5, updated_at = ?4
             WHERE id = ?1 AND status = 'claimed' AND claim_key = ?2 AND claimed_by = ?3",
            params![
                request_id,
                claim_key,
                worker_id,
                now,
                worker_id,
                failure_reason,
            ],
        )?;
        require_claim_update(transaction, request_id, changed)?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn abandon(
    database: &Database,
    request_id: &str,
    expected_revision: u32,
    expected_attempt_count: u32,
    failure_reason: &str,
    abandoned_by: &str,
) -> Result<ChangeRequest> {
    let failure_reason = required_text("failureReason", failure_reason)?;
    let abandoned_by = required_text("abandonedBy", abandoned_by)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE change_requests
             SET status = 'failed', failed_at = ?2, failed_by = ?3,
                 failure_reason = ?4, updated_by = ?3, updated_at = ?2
             WHERE id = ?1 AND status = 'claimed'
               AND revision = ?5 AND attempt_count = ?6",
            params![
                request_id,
                now,
                abandoned_by,
                failure_reason,
                expected_revision,
                expected_attempt_count,
            ],
        )?;
        require_claim_update(transaction, request_id, changed)?;
        query_request(transaction, request_id)
    })
}

pub(crate) fn list(database: &Database) -> Result<Vec<ChangeRequest>> {
    let connection = database.connect()?;
    let sql = format!(
        "SELECT {REQUEST_COLUMNS} FROM change_requests
         ORDER BY
           CASE status
             WHEN 'approved' THEN 0 WHEN 'claimed' THEN 1 WHEN 'draft' THEN 2
             WHEN 'failed' THEN 3 WHEN 'completed' THEN 4 ELSE 5
           END,
           priority DESC, updated_at DESC, id DESC"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], map_request)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

pub(crate) fn list_events(
    database: &Database,
    request_id: &str,
) -> Result<Vec<ChangeRequestEvent>> {
    let connection = database.connect()?;
    query_request(&connection, request_id)?;
    let mut statement = connection.prepare(
        "SELECT id, change_request_id, event_type, from_status, to_status,
                revision, actor, details_json, created_at
         FROM change_request_events
         WHERE change_request_id = ?1
         ORDER BY created_at ASC, id ASC",
    )?;
    let rows = statement.query_map([request_id], map_event)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_request(connection: &Connection, request_id: &str) -> Result<ChangeRequest> {
    let sql = format!("SELECT {REQUEST_COLUMNS} FROM change_requests WHERE id = ?1");
    connection
        .query_row(&sql, [request_id], map_request)
        .optional()?
        .ok_or_else(|| not_found("change request", request_id))
}

fn map_request(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeRequest> {
    let kind: String = row.get(1)?;
    let status: String = row.get(9)?;
    Ok(ChangeRequest {
        id: row.get(0)?,
        kind: enum_from_sql(1, &kind)?,
        project_id: row.get(2)?,
        task_id: row.get(3)?,
        title: row.get(4)?,
        description: row.get(5)?,
        desired_outcome: row.get(6)?,
        reproduction_steps: row.get(7)?,
        priority: integer_from_sql(8, row.get(8)?)?,
        status: enum_from_sql(9, &status)?,
        revision: integer_from_sql(10, row.get(10)?)?,
        approved_revision: optional_integer_from_sql(11, row.get(11)?)?,
        attempt_count: integer_from_sql(12, row.get(12)?)?,
        requested_by: row.get(13)?,
        updated_by: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        approved_at: row.get(17)?,
        approved_by: row.get(18)?,
        claimed_at: row.get(19)?,
        claimed_by: row.get(20)?,
        completed_at: row.get(21)?,
        completed_by: row.get(22)?,
        failed_at: row.get(23)?,
        failed_by: row.get(24)?,
        cancelled_at: row.get(25)?,
        cancelled_by: row.get(26)?,
        result_summary: row.get(27)?,
        patch_ref: row.get(28)?,
        failure_reason: row.get(29)?,
        cancellation_reason: row.get(30)?,
    })
}

fn map_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChangeRequestEvent> {
    let from_status: Option<String> = row.get(3)?;
    let to_status: String = row.get(4)?;
    let details: String = row.get(7)?;
    Ok(ChangeRequestEvent {
        id: row.get(0)?,
        change_request_id: row.get(1)?,
        event_type: row.get(2)?,
        from_status: from_status
            .as_deref()
            .map(|value| enum_from_sql(3, value))
            .transpose()?,
        to_status: enum_from_sql(4, &to_status)?,
        revision: integer_from_sql(5, row.get(5)?)?,
        actor: row.get(6)?,
        details: serde_json::from_str(&details).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
        })?,
        created_at: row.get(8)?,
    })
}

fn validate_content(
    title: &str,
    description: &str,
    desired_outcome: &str,
    priority: u8,
    actor: &str,
    actor_field: &str,
) -> Result<()> {
    required_text("title", title)?;
    required_text("description", description)?;
    required_text("desiredOutcome", desired_outcome)?;
    required_text(actor_field, actor)?;
    if priority > 3 {
        return Err(invalid("change request priority must be between 0 and 3"));
    }
    Ok(())
}

fn required_text(field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(format!("{field} cannot be empty")));
    }
    Ok(value.to_owned())
}

fn require_one_or_state_error(
    transaction: &Transaction<'_>,
    request_id: &str,
    expected_revision: u32,
    expected_attempt_count: u32,
    changed: usize,
    message: &str,
) -> Result<()> {
    if changed == 1 {
        return Ok(());
    }
    let current: Option<(String, u32, u32)> = transaction
        .query_row(
            "SELECT status, revision, attempt_count FROM change_requests WHERE id = ?1",
            [request_id],
            |row| {
                Ok((
                    row.get(0)?,
                    integer_from_sql(1, row.get(1)?)?,
                    integer_from_sql(2, row.get(2)?)?,
                ))
            },
        )
        .optional()?;
    let Some((status, revision, attempt_count)) = current else {
        return Err(not_found("change request", request_id));
    };
    Err(Error::Conflict(format!(
        "{message}; current status={status}, revision={revision}, attemptCount={attempt_count}, expectedRevision={expected_revision}, expectedAttemptCount={expected_attempt_count}"
    )))
}

fn require_claim_update(
    transaction: &Transaction<'_>,
    request_id: &str,
    changed: usize,
) -> Result<()> {
    if changed == 1 {
        return Ok(());
    }
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM change_requests WHERE id = ?1)",
        [request_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(not_found("change request", request_id));
    }
    Err(Error::Conflict(
        "change request claim key, worker, or status did not match".to_owned(),
    ))
}

fn enum_from_sql<T>(index: usize, value: &str) -> rusqlite::Result<T>
where
    T: FromStr<Err = Error>,
{
    T::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn integer_from_sql<T>(index: usize, value: i64) -> rusqlite::Result<T>
where
    T: TryFrom<i64>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    T::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Integer, Box::new(error))
    })
}

fn optional_integer_from_sql<T>(index: usize, value: Option<i64>) -> rusqlite::Result<Option<T>>
where
    T: TryFrom<i64>,
    T::Error: std::error::Error + Send + Sync + 'static,
{
    value
        .map(|value| integer_from_sql(index, value))
        .transpose()
}
