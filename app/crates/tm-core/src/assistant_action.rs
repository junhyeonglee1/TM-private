use chrono::{Duration, SecondsFormat, Utc};
use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params, types::Type,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    CreateTaskInput, Error, MutationApprovalPolicy, MutationCommand, MutationExpectedVersion,
    MutationOperation, MutationRequest, MutationResult, Result, TaskStatus, TmCore,
    core::query_project,
    database::{new_id, now_utc},
    error::{invalid, not_found},
    mutation::validate_task_create,
};

pub const ASSISTANT_ACTION_APPROVAL_TTL_SECONDS: u64 = 600;
const MAX_ACTION_TTL_SECONDS: u64 = 600;
const MAX_REASON_CHARS: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssistantActionStatus {
    Pending,
    Approved,
    Executing,
    Completed,
    Rejected,
    Cancelled,
    Expired,
    Failed,
}

impl AssistantActionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Rejected | Self::Cancelled | Self::Expired | Self::Failed
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation")]
pub enum AssistantActionPayload {
    #[serde(rename = "task.create")]
    TaskCreate { input: CreateTaskInput },
}

impl AssistantActionPayload {
    #[must_use]
    pub const fn operation(&self) -> MutationOperation {
        match self {
            Self::TaskCreate { .. } => MutationOperation::TaskCreate,
        }
    }

    fn command(&self) -> MutationCommand {
        match self {
            Self::TaskCreate { input } => MutationCommand::TaskCreate {
                input: input.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantActionRequest {
    pub id: String,
    pub operation: MutationOperation,
    pub status: AssistantActionStatus,
    pub revision: u64,
    pub payload: AssistantActionPayload,
    pub payload_sha256: String,
    pub origin_request_id: String,
    pub execution_idempotency_key: String,
    pub approval_idempotency_key: Option<String>,
    pub result: Option<MutationResult>,
    pub failure_code: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub approved_at: Option<String>,
    pub executing_at: Option<String>,
    pub completed_at: Option<String>,
    pub terminal_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantActionExecution {
    pub action: AssistantActionRequest,
    pub mutation_replayed: bool,
}

impl TmCore {
    pub fn propose_task_create_action(
        &self,
        mut input: CreateTaskInput,
        origin_request_id: &str,
        ttl_seconds: u64,
    ) -> Result<AssistantActionRequest> {
        validate_request_id(origin_request_id)?;
        if ttl_seconds > MAX_ACTION_TTL_SECONDS {
            return Err(invalid(format!(
                "assistant action TTL cannot exceed {MAX_ACTION_TTL_SECONDS} seconds"
            )));
        }
        input.title = input.title.trim().to_owned();
        input.description = input.description.trim().to_owned();
        if !matches!(input.status, TaskStatus::Inbox | TaskStatus::Todo) {
            return Err(invalid(
                "assistant-created tasks must start as inbox or todo",
            ));
        }
        validate_task_create(&input)?;

        let payload = AssistantActionPayload::TaskCreate { input };
        let payload_json = serde_json::to_string(&payload)?;
        let payload_sha256 = format!("{:x}", Sha256::digest(payload_json.as_bytes()));
        let id = new_id();
        let execution_idempotency_key = format!("ai-action-exec:{id}");
        let created = Utc::now();
        let created_at = created.to_rfc3339_opts(SecondsFormat::Millis, true);
        let expires_at = (created + Duration::seconds(ttl_seconds as i64))
            .to_rfc3339_opts(SecondsFormat::Millis, true);

        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                validate_payload_targets(transaction, &payload)?;
                transaction.execute(
                    "INSERT INTO assistant_action_requests(
                        id, operation, status, revision, payload_json, payload_sha256,
                        origin_request_id, execution_idempotency_key, created_at, expires_at
                     ) VALUES (?1, 'task.create', 'pending', 1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        id,
                        payload_json,
                        payload_sha256,
                        origin_request_id,
                        execution_idempotency_key,
                        created_at,
                        expires_at,
                    ],
                )?;
                insert_event(
                    transaction,
                    &id,
                    "proposed",
                    None,
                    AssistantActionStatus::Pending,
                    1,
                    "assistant",
                    origin_request_id,
                    &payload_sha256,
                    None,
                    &created_at,
                )?;
                query_action(transaction, &id)
            })
    }

    pub fn list_assistant_actions(&self) -> Result<Vec<AssistantActionRequest>> {
        self.expire_due_assistant_actions()?;
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {ACTION_COLUMNS} FROM assistant_action_requests
             ORDER BY created_at DESC, id DESC LIMIT 100"
        ))?;
        let rows = statement.query_map([], map_action)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_assistant_action(&self, action_id: &str) -> Result<AssistantActionRequest> {
        validate_uuid("action ID", action_id)?;
        self.expire_due_assistant_actions()?;
        let connection = self.database.connect()?;
        query_action(&connection, action_id)
    }

    pub fn approve_and_execute_assistant_action(
        &self,
        action_id: &str,
        expected_revision: u64,
        payload_sha256: &str,
        approval_idempotency_key: &str,
        request_id: &str,
    ) -> Result<AssistantActionExecution> {
        validate_action_preconditions(action_id, expected_revision, payload_sha256, request_id)?;
        validate_idempotency_key(approval_idempotency_key)?;
        self.expire_due_assistant_actions()?;

        let mut action =
            self.database
                .transaction(TransactionBehavior::Immediate, |transaction| {
                    let current = query_action(transaction, action_id)?;
                    match current.status {
                        AssistantActionStatus::Pending => {
                            require_revision_and_hash(&current, expected_revision, payload_sha256)?;
                            if transaction.query_row(
                                "SELECT EXISTS(
                                SELECT 1 FROM assistant_action_requests
                                WHERE approval_idempotency_key = ?1 AND id <> ?2
                             )",
                                params![approval_idempotency_key, action_id],
                                |row| row.get::<_, bool>(0),
                            )? {
                                return Err(Error::Conflict(
                                    "approval idempotency key was already used for another action"
                                        .to_owned(),
                                ));
                            }
                            let now = now_utc();
                            transition_status(
                                transaction,
                                &current,
                                AssistantActionStatus::Approved,
                                "approved",
                                "single_user",
                                request_id,
                                Some(approval_idempotency_key),
                                None,
                                None,
                                &now,
                            )
                        }
                        AssistantActionStatus::Approved | AssistantActionStatus::Executing => {
                            require_replay_identity(
                                &current,
                                payload_sha256,
                                approval_idempotency_key,
                            )?;
                            Ok(current)
                        }
                        AssistantActionStatus::Completed => {
                            require_replay_identity(
                                &current,
                                payload_sha256,
                                approval_idempotency_key,
                            )?;
                            Ok(current)
                        }
                        _ => Err(action_state_conflict(&current)),
                    }
                })?;

        if action.status == AssistantActionStatus::Completed {
            return Ok(AssistantActionExecution {
                action,
                mutation_replayed: true,
            });
        }

        if action.status == AssistantActionStatus::Approved {
            action = self
                .database
                .transaction(TransactionBehavior::Immediate, |transaction| {
                    let current = query_action(transaction, action_id)?;
                    if current.status == AssistantActionStatus::Executing {
                        require_replay_identity(
                            &current,
                            payload_sha256,
                            approval_idempotency_key,
                        )?;
                        return Ok(current);
                    }
                    if current.status != AssistantActionStatus::Approved {
                        return Err(action_state_conflict(&current));
                    }
                    let now = now_utc();
                    transition_status(
                        transaction,
                        &current,
                        AssistantActionStatus::Executing,
                        "execution_started",
                        "system",
                        request_id,
                        None,
                        None,
                        None,
                        &now,
                    )
                })?;
        }

        let request = MutationRequest {
            idempotency_key: action.execution_idempotency_key.clone(),
            expected_version: MutationExpectedVersion::Absent,
            actor: "tm_ai_assistant".to_owned(),
            request_id: request_id.to_owned(),
            approval_policy: MutationApprovalPolicy::AiActionApproval,
            command: action.payload.command(),
        };
        let mutation = match self.execute_remote_mutation(request) {
            Ok(result) => result,
            Err(error) => {
                if action_failure_code(&error).is_some() {
                    self.fail_executing_action(action_id, request_id, &error)?;
                }
                return Err(error);
            }
        };
        let mutation_replayed = mutation.replayed;
        let mut stored_mutation = mutation;
        stored_mutation.replayed = false;
        let result_json = serde_json::to_string(&stored_mutation)?;
        let metadata = serde_json::to_string(&json!({
            "resourceId": stored_mutation.resource_id,
            "version": stored_mutation.version
        }))?;
        let completed =
            self.database
                .transaction(TransactionBehavior::Immediate, |transaction| {
                    let current = query_action(transaction, action_id)?;
                    if current.status == AssistantActionStatus::Completed {
                        require_replay_identity(
                            &current,
                            payload_sha256,
                            approval_idempotency_key,
                        )?;
                        return Ok(current);
                    }
                    if current.status != AssistantActionStatus::Executing {
                        return Err(action_state_conflict(&current));
                    }
                    let now = now_utc();
                    transition_status(
                        transaction,
                        &current,
                        AssistantActionStatus::Completed,
                        "completed",
                        "system",
                        request_id,
                        None,
                        Some(&result_json),
                        Some(&metadata),
                        &now,
                    )
                })?;

        Ok(AssistantActionExecution {
            action: completed,
            mutation_replayed,
        })
    }

    pub fn reject_assistant_action(
        &self,
        action_id: &str,
        expected_revision: u64,
        payload_sha256: &str,
        request_id: &str,
        reason: Option<&str>,
    ) -> Result<AssistantActionRequest> {
        self.finish_pending_action(
            action_id,
            expected_revision,
            payload_sha256,
            request_id,
            reason,
            AssistantActionStatus::Rejected,
            "rejected",
        )
    }

    pub fn cancel_assistant_action(
        &self,
        action_id: &str,
        expected_revision: u64,
        payload_sha256: &str,
        request_id: &str,
        reason: Option<&str>,
    ) -> Result<AssistantActionRequest> {
        self.finish_pending_action(
            action_id,
            expected_revision,
            payload_sha256,
            request_id,
            reason,
            AssistantActionStatus::Cancelled,
            "cancelled",
        )
    }

    fn finish_pending_action(
        &self,
        action_id: &str,
        expected_revision: u64,
        payload_sha256: &str,
        request_id: &str,
        reason: Option<&str>,
        target: AssistantActionStatus,
        event_type: &str,
    ) -> Result<AssistantActionRequest> {
        validate_action_preconditions(action_id, expected_revision, payload_sha256, request_id)?;
        let reason = normalize_reason(reason)?;
        self.expire_due_assistant_actions()?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_action(transaction, action_id)?;
                require_revision_and_hash(&current, expected_revision, payload_sha256)?;
                if current.status != AssistantActionStatus::Pending {
                    return Err(action_state_conflict(&current));
                }
                let metadata = reason
                    .as_ref()
                    .map(|value| serde_json::to_string(&json!({"reason": value})))
                    .transpose()?;
                let now = now_utc();
                transition_status(
                    transaction,
                    &current,
                    target,
                    event_type,
                    "single_user",
                    request_id,
                    None,
                    None,
                    metadata.as_deref(),
                    &now,
                )
            })
    }

    fn expire_due_assistant_actions(&self) -> Result<()> {
        let now = now_utc();
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let ids = {
                    let mut statement = transaction.prepare(
                        "SELECT id FROM assistant_action_requests
                         WHERE status IN ('pending', 'approved') AND expires_at <= ?1
                         ORDER BY created_at, id",
                    )?;
                    statement
                        .query_map([&now], |row| row.get::<_, String>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                for id in ids {
                    let current = query_action(transaction, &id)?;
                    transition_status(
                        transaction,
                        &current,
                        AssistantActionStatus::Expired,
                        "expired",
                        "system",
                        "system-expiry",
                        None,
                        None,
                        None,
                        &now,
                    )?;
                }
                Ok(())
            })
    }

    fn fail_executing_action(
        &self,
        action_id: &str,
        request_id: &str,
        error: &Error,
    ) -> Result<()> {
        let Some(code) = action_failure_code(error) else {
            return Ok(());
        };
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_action(transaction, action_id)?;
                if current.status != AssistantActionStatus::Executing {
                    return Ok(());
                }
                let metadata = serde_json::to_string(&json!({"failureCode": code}))?;
                let now = now_utc();
                transition_status(
                    transaction,
                    &current,
                    AssistantActionStatus::Failed,
                    "failed",
                    "system",
                    request_id,
                    None,
                    None,
                    Some(&metadata),
                    &now,
                )?;
                Ok(())
            })
    }
}

const ACTION_COLUMNS: &str = "id, operation, status, revision, payload_json, payload_sha256,
    origin_request_id, execution_idempotency_key, approval_idempotency_key, result_json,
    failure_code, created_at, expires_at, approved_at, executing_at, completed_at, terminal_at";

fn query_action(connection: &Connection, action_id: &str) -> Result<AssistantActionRequest> {
    connection
        .query_row(
            &format!("SELECT {ACTION_COLUMNS} FROM assistant_action_requests WHERE id = ?1"),
            [action_id],
            map_action,
        )
        .optional()?
        .ok_or_else(|| not_found("assistant action", action_id))
}

fn map_action(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssistantActionRequest> {
    let operation: String = row.get(1)?;
    let status: String = row.get(2)?;
    let revision: i64 = row.get(3)?;
    let payload_json: String = row.get(4)?;
    let result_json: Option<String> = row.get(9)?;
    Ok(AssistantActionRequest {
        id: row.get(0)?,
        operation: parse_operation(1, &operation)?,
        status: parse_status(2, &status)?,
        revision: u64::try_from(revision).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
        })?,
        payload: serde_json::from_str(&payload_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
        })?,
        payload_sha256: row.get(5)?,
        origin_request_id: row.get(6)?,
        execution_idempotency_key: row.get(7)?,
        approval_idempotency_key: row.get(8)?,
        result: result_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(9, Type::Text, Box::new(error))
            })?,
        failure_code: row.get(10)?,
        created_at: row.get(11)?,
        expires_at: row.get(12)?,
        approved_at: row.get(13)?,
        executing_at: row.get(14)?,
        completed_at: row.get(15)?,
        terminal_at: row.get(16)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn transition_status(
    transaction: &Transaction<'_>,
    current: &AssistantActionRequest,
    target: AssistantActionStatus,
    event_type: &str,
    actor: &str,
    request_id: &str,
    approval_idempotency_key: Option<&str>,
    result_json: Option<&str>,
    metadata_json: Option<&str>,
    now: &str,
) -> Result<AssistantActionRequest> {
    let next_revision = current.revision.saturating_add(1);
    let changed = transaction.execute(
        "UPDATE assistant_action_requests SET
            status = ?1,
            revision = ?2,
            approval_idempotency_key = COALESCE(?3, approval_idempotency_key),
            result_json = COALESCE(?4, result_json),
            failure_code = CASE WHEN ?1 = 'failed' THEN json_extract(?5, '$.failureCode') ELSE failure_code END,
            approved_at = CASE WHEN ?1 = 'approved' THEN ?6 ELSE approved_at END,
            executing_at = CASE WHEN ?1 = 'executing' THEN ?6 ELSE executing_at END,
            completed_at = CASE WHEN ?1 = 'completed' THEN ?6 ELSE completed_at END,
            terminal_at = CASE WHEN ?1 IN ('completed', 'rejected', 'cancelled', 'expired', 'failed') THEN ?6 ELSE terminal_at END
         WHERE id = ?7 AND revision = ?8 AND status = ?9",
        params![
            target.as_str(),
            next_revision,
            approval_idempotency_key,
            result_json,
            metadata_json,
            now,
            current.id,
            current.revision,
            current.status.as_str(),
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "assistant action changed concurrently".to_owned(),
        ));
    }
    insert_event(
        transaction,
        &current.id,
        event_type,
        Some(current.status),
        target,
        next_revision,
        actor,
        request_id,
        &current.payload_sha256,
        metadata_json,
        now,
    )?;
    query_action(transaction, &current.id)
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    transaction: &Transaction<'_>,
    action_id: &str,
    event_type: &str,
    from_status: Option<AssistantActionStatus>,
    to_status: AssistantActionStatus,
    revision: u64,
    actor: &str,
    request_id: &str,
    payload_sha256: &str,
    metadata_json: Option<&str>,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO assistant_action_events(
            id, action_id, event_type, from_status, to_status, revision, actor,
            request_id, payload_sha256, metadata_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            new_id(),
            action_id,
            event_type,
            from_status.map(AssistantActionStatus::as_str),
            to_status.as_str(),
            revision,
            actor,
            request_id,
            payload_sha256,
            metadata_json,
            created_at,
        ],
    )?;
    Ok(())
}

fn validate_payload_targets(
    transaction: &Transaction<'_>,
    payload: &AssistantActionPayload,
) -> Result<()> {
    match payload {
        AssistantActionPayload::TaskCreate { input } => {
            if let Some(project_id) = input.project_id.as_deref() {
                validate_uuid("project ID", project_id)?;
                let project = query_project(transaction, project_id)?;
                if project.deleted_at.is_some() || project.archived_at.is_some() {
                    return Err(Error::Conflict(
                        "assistant task project must be active".to_owned(),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_action_preconditions(
    action_id: &str,
    expected_revision: u64,
    payload_sha256: &str,
    request_id: &str,
) -> Result<()> {
    validate_uuid("action ID", action_id)?;
    if expected_revision == 0 {
        return Err(invalid("expected revision must be positive"));
    }
    if payload_sha256.len() != 64
        || !payload_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(
            "payload SHA-256 must be 64 lowercase hexadecimal characters",
        ));
    }
    validate_request_id(request_id)
}

fn require_revision_and_hash(
    action: &AssistantActionRequest,
    expected_revision: u64,
    payload_sha256: &str,
) -> Result<()> {
    if action.revision != expected_revision {
        return Err(Error::Conflict(format!(
            "assistant action revision conflict: expected {expected_revision}, found {}",
            action.revision
        )));
    }
    if action.payload_sha256 != payload_sha256 {
        return Err(Error::Conflict(
            "assistant action payload hash does not match".to_owned(),
        ));
    }
    Ok(())
}

fn require_replay_identity(
    action: &AssistantActionRequest,
    payload_sha256: &str,
    approval_idempotency_key: &str,
) -> Result<()> {
    if action.payload_sha256 != payload_sha256
        || action.approval_idempotency_key.as_deref() != Some(approval_idempotency_key)
    {
        return Err(Error::Conflict(
            "assistant action approval replay does not match the original approval".to_owned(),
        ));
    }
    Ok(())
}

fn normalize_reason(reason: Option<&str>) -> Result<Option<String>> {
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());
    if reason.is_some_and(|value| value.chars().count() > MAX_REASON_CHARS) {
        return Err(invalid(format!(
            "assistant action reason cannot exceed {MAX_REASON_CHARS} characters"
        )));
    }
    Ok(reason.map(str::to_owned))
}

fn validate_idempotency_key(value: &str) -> Result<()> {
    if !(16..=128).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid(
            "approval idempotency key must be 16-128 ASCII letters, digits, '.', '_', ':' or '-'",
        ));
    }
    Ok(())
}

fn validate_request_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid("assistant action request ID has an invalid format"));
    }
    Ok(())
}

fn validate_uuid(field: &str, value: &str) -> Result<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid(format!("{field} must be a UUID")))
}

fn action_state_conflict(action: &AssistantActionRequest) -> Error {
    Error::Conflict(format!(
        "assistant action cannot transition from {}",
        action.status.as_str()
    ))
}

fn action_failure_code(error: &Error) -> Option<&'static str> {
    match error {
        Error::InvalidInput(_) => Some("invalid_mutation"),
        Error::NotFound { .. } => Some("resource_not_found"),
        Error::Conflict(_) => Some("mutation_conflict"),
        _ => None,
    }
}

fn parse_operation(index: usize, value: &str) -> rusqlite::Result<MutationOperation> {
    match value {
        "task.create" => Ok(MutationOperation::TaskCreate),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            invalid(format!("unknown assistant action operation: {value}")).into(),
        )),
    }
}

fn parse_status(index: usize, value: &str) -> rusqlite::Result<AssistantActionStatus> {
    let status = match value {
        "pending" => AssistantActionStatus::Pending,
        "approved" => AssistantActionStatus::Approved,
        "executing" => AssistantActionStatus::Executing,
        "completed" => AssistantActionStatus::Completed,
        "rejected" => AssistantActionStatus::Rejected,
        "cancelled" => AssistantActionStatus::Cancelled,
        "expired" => AssistantActionStatus::Expired,
        "failed" => AssistantActionStatus::Failed,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                index,
                Type::Text,
                invalid(format!("unknown assistant action status: {value}")).into(),
            ));
        }
    };
    Ok(status)
}
