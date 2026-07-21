use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params, types::Type};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    AssistantMemory, CreateMemoryInput, CreateNoteInput, CreateTaskInput, Error, MemoryPatch, Note,
    NotePatch, Result, Task, TaskPatch, TaskStatus, TmCore,
    core::{
        create_note_in_transaction, create_task_in_transaction, query_checklist_item, query_note,
        query_project, query_task, update_checklist_item_in_transaction,
        update_note_in_transaction, update_task_in_transaction,
    },
    database::{new_id, now_utc},
    error::invalid,
    memory::{
        create_memory_in_transaction, delete_memory_in_transaction, query_memory,
        update_memory_in_transaction,
    },
};

const MAX_TITLE_CHARS: usize = 500;
const MAX_TASK_DESCRIPTION_CHARS: usize = 20_000;
const MAX_NOTE_BODY_CHARS: usize = 50_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MutationExpectedVersion {
    Absent,
    Exact(u64),
}

impl MutationExpectedVersion {
    fn audit_value(self) -> String {
        match self {
            Self::Absent => "absent".to_owned(),
            Self::Exact(version) => version.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MutationApprovalPolicy {
    ExplicitUserConfirmation,
    AiActionApproval,
}

impl MutationApprovalPolicy {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitUserConfirmation => "explicit_user_confirmation",
            Self::AiActionApproval => "ai_action_approval",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MutationOperation {
    TaskCreate,
    TaskUpdate,
    NoteCreate,
    NoteUpdate,
    ChecklistSetDone,
    MemoryCreate,
    MemoryUpdate,
    MemoryDelete,
}

impl MutationOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TaskCreate => "task.create",
            Self::TaskUpdate => "task.update",
            Self::NoteCreate => "note.create",
            Self::NoteUpdate => "note.update",
            Self::ChecklistSetDone => "checklist.set_done",
            Self::MemoryCreate => "memory.create",
            Self::MemoryUpdate => "memory.update",
            Self::MemoryDelete => "memory.delete",
        }
    }

    const fn resource_type(self) -> &'static str {
        match self {
            Self::TaskCreate | Self::TaskUpdate => "task",
            Self::NoteCreate | Self::NoteUpdate => "note",
            Self::ChecklistSetDone => "checklist",
            Self::MemoryCreate | Self::MemoryUpdate | Self::MemoryDelete => "memory",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum MutationCommand {
    TaskCreate {
        input: CreateTaskInput,
    },
    TaskUpdate {
        task_id: String,
        patch: TaskPatch,
    },
    NoteCreate {
        input: CreateNoteInput,
    },
    NoteUpdate {
        note_id: String,
        patch: NotePatch,
    },
    ChecklistSetDone {
        item_id: String,
        is_done: bool,
    },
    MemoryCreate {
        input: CreateMemoryInput,
    },
    MemoryUpdate {
        memory_id: String,
        patch: MemoryPatch,
    },
    MemoryDelete {
        memory_id: String,
    },
}

impl MutationCommand {
    #[must_use]
    pub const fn operation(&self) -> MutationOperation {
        match self {
            Self::TaskCreate { .. } => MutationOperation::TaskCreate,
            Self::TaskUpdate { .. } => MutationOperation::TaskUpdate,
            Self::NoteCreate { .. } => MutationOperation::NoteCreate,
            Self::NoteUpdate { .. } => MutationOperation::NoteUpdate,
            Self::ChecklistSetDone { .. } => MutationOperation::ChecklistSetDone,
            Self::MemoryCreate { .. } => MutationOperation::MemoryCreate,
            Self::MemoryUpdate { .. } => MutationOperation::MemoryUpdate,
            Self::MemoryDelete { .. } => MutationOperation::MemoryDelete,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MutationRequest {
    pub idempotency_key: String,
    pub expected_version: MutationExpectedVersion,
    pub actor: String,
    pub request_id: String,
    pub approval_policy: MutationApprovalPolicy,
    pub command: MutationCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MutationResult {
    pub operation: MutationOperation,
    pub resource_type: String,
    pub resource_id: String,
    pub version: u64,
    pub replayed: bool,
    pub entity: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MutationAuditEvent {
    pub id: String,
    pub idempotency_key: String,
    pub actor: String,
    pub request_id: String,
    pub operation: MutationOperation,
    pub resource_type: String,
    pub resource_id: String,
    pub expected_version: String,
    pub resulting_version: u64,
    pub before: Option<Value>,
    pub after: Value,
    pub result: String,
    pub approval_policy: String,
    pub created_at: String,
}

impl TmCore {
    pub fn execute_remote_mutation(&self, request: MutationRequest) -> Result<MutationResult> {
        validate_request_metadata(&request)?;
        let operation = request.command.operation();
        let request_sha256 = request_hash(&request)?;

        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some((stored_operation, stored_hash, response_json)) = transaction
                    .query_row(
                        "SELECT operation, request_sha256, response_json
                         FROM mutation_idempotency_records WHERE idempotency_key = ?1",
                        [&request.idempotency_key],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?
                {
                    if stored_operation != operation.as_str() || stored_hash != request_sha256 {
                        return Err(Error::Conflict(
                            "idempotency key was already used for a different mutation".to_owned(),
                        ));
                    }
                    let mut result: MutationResult = serde_json::from_str(&response_json)?;
                    result.replayed = true;
                    return Ok(result);
                }

                let execution = execute_command(
                    transaction,
                    &request.command,
                    request.expected_version,
                    &request.actor,
                    &request.request_id,
                )?;
                let result = MutationResult {
                    operation,
                    resource_type: operation.resource_type().to_owned(),
                    resource_id: execution.resource_id,
                    version: execution.version,
                    replayed: false,
                    entity: execution.after.clone(),
                };
                let response_json = serde_json::to_string(&result)?;
                let before_json = execution
                    .before
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()?;
                let after_json = serde_json::to_string(&execution.after)?;
                let created_at = now_utc();

                transaction.execute(
                    "INSERT INTO mutation_idempotency_records(
                        idempotency_key, operation, request_sha256, actor, request_id,
                        resource_type, resource_id, resource_version, response_json, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        request.idempotency_key,
                        operation.as_str(),
                        request_sha256,
                        request.actor,
                        request.request_id,
                        operation.resource_type(),
                        result.resource_id,
                        result.version,
                        response_json,
                        created_at,
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO mutation_audit_events(
                        id, idempotency_key, actor, request_id, operation, resource_type,
                        resource_id, expected_version, resulting_version, before_json,
                        after_json, result, approval_policy, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                               'completed', ?12, ?13)",
                    params![
                        new_id(),
                        request.idempotency_key,
                        request.actor,
                        request.request_id,
                        operation.as_str(),
                        operation.resource_type(),
                        result.resource_id,
                        request.expected_version.audit_value(),
                        result.version,
                        before_json,
                        after_json,
                        request.approval_policy.as_str(),
                        created_at,
                    ],
                )?;

                Ok(result)
            })
    }

    pub fn list_mutation_audit_events(&self) -> Result<Vec<MutationAuditEvent>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, idempotency_key, actor, request_id, operation, resource_type,
                    resource_id, expected_version, resulting_version, before_json,
                    after_json, result, approval_policy, created_at
             FROM mutation_audit_events ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([], map_audit_event)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

struct MutationExecution {
    resource_id: String,
    version: u64,
    before: Option<Value>,
    after: Value,
}

fn execute_command(
    transaction: &Transaction<'_>,
    command: &MutationCommand,
    expected_version: MutationExpectedVersion,
    actor: &str,
    request_id: &str,
) -> Result<MutationExecution> {
    match command {
        MutationCommand::TaskCreate { input } => {
            require_absent(expected_version)?;
            validate_task_create(input)?;
            if let Some(project_id) = input.project_id.as_deref() {
                validate_id("projectId", project_id)?;
                validate_project_target(transaction, project_id)?;
            }
            let task = create_task_in_transaction(transaction, input)?;
            completed_execution(None, &task, &task.id, task.version)
        }
        MutationCommand::TaskUpdate { task_id, patch } => {
            validate_id("taskId", task_id)?;
            validate_task_patch(patch)?;
            let expected = require_exact(expected_version)?;
            let current = query_task(transaction, task_id, true)?;
            require_version("task", current.version, expected)?;
            validate_task_transition(current.status, patch.status)?;
            if let Some(project_id) = patch.project_id.as_deref() {
                validate_id("projectId", project_id)?;
                validate_project_target(transaction, project_id)?;
            }
            if !task_patch_changes(&current, patch) {
                return Err(invalid("task update must change at least one field"));
            }
            let before = serde_json::to_value(&current)?;
            let task = update_task_in_transaction(transaction, task_id, patch.clone())?;
            completed_execution(Some(before), &task, &task.id, task.version)
        }
        MutationCommand::NoteCreate { input } => {
            require_absent(expected_version)?;
            validate_note_create(input)?;
            let note = create_note_in_transaction(transaction, input)?;
            completed_execution(None, &note, &note.id, note.version)
        }
        MutationCommand::NoteUpdate { note_id, patch } => {
            validate_id("noteId", note_id)?;
            validate_note_patch(patch)?;
            let expected = require_exact(expected_version)?;
            let current = query_note(transaction, note_id)?;
            require_version("note", current.version, expected)?;
            if !note_patch_changes(&current, patch) {
                return Err(invalid("note update must change at least one field"));
            }
            let before = serde_json::to_value(&current)?;
            let note = update_note_in_transaction(transaction, note_id, patch.clone())?;
            completed_execution(Some(before), &note, &note.id, note.version)
        }
        MutationCommand::ChecklistSetDone { item_id, is_done } => {
            validate_id("checklistId", item_id)?;
            let expected = require_exact(expected_version)?;
            let current = query_checklist_item(transaction, item_id)?;
            require_version("checklist item", current.version, expected)?;
            let owner = query_task(transaction, &current.task_id, true)?;
            if owner.deleted_at.is_some() {
                return Err(Error::Conflict(
                    "checklist owner is in the trash; restore it before editing".to_owned(),
                ));
            }
            if current.is_done == *is_done {
                return Err(invalid("checklist state must change"));
            }
            let before = serde_json::to_value(&current)?;
            let item = update_checklist_item_in_transaction(
                transaction,
                item_id,
                None,
                Some(*is_done),
                None,
            )?;
            completed_execution(Some(before), &item, &item.id, item.version)
        }
        MutationCommand::MemoryCreate { input } => {
            require_absent(expected_version)?;
            let memory = create_memory_in_transaction(transaction, input, actor, request_id)?;
            completed_execution(None, &memory, &memory.id, memory.revision)
        }
        MutationCommand::MemoryUpdate { memory_id, patch } => {
            validate_id("memoryId", memory_id)?;
            let expected = require_exact(expected_version)?;
            let current = query_memory(transaction, memory_id)?;
            let before = serde_json::to_value(&current)?;
            let memory = update_memory_in_transaction(
                transaction,
                memory_id,
                expected,
                patch,
                actor,
                request_id,
            )?;
            completed_execution(Some(before), &memory, &memory.id, memory.revision)
        }
        MutationCommand::MemoryDelete { memory_id } => {
            validate_id("memoryId", memory_id)?;
            let expected = require_exact(expected_version)?;
            let current: AssistantMemory = query_memory(transaction, memory_id)?;
            let before = serde_json::to_value(&current)?;
            let memory =
                delete_memory_in_transaction(transaction, memory_id, expected, actor, request_id)?;
            completed_execution(Some(before), &memory, &memory.id, memory.revision)
        }
    }
}

fn completed_execution<T: Serialize>(
    before: Option<Value>,
    entity: &T,
    resource_id: &str,
    version: u64,
) -> Result<MutationExecution> {
    Ok(MutationExecution {
        resource_id: resource_id.to_owned(),
        version,
        before,
        after: serde_json::to_value(entity)?,
    })
}

fn validate_request_metadata(request: &MutationRequest) -> Result<()> {
    let key = request.idempotency_key.as_str();
    if !(16..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid(
            "idempotency key must be 16-128 ASCII letters, digits, '.', '_', ':' or '-'",
        ));
    }
    validate_metadata_value("actor", &request.actor, 64)?;
    validate_metadata_value("request ID", &request.request_id, 128)?;
    Ok(())
}

fn validate_metadata_value(field: &str, value: &str, max: usize) -> Result<()> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(invalid(format!("{field} has an invalid format")));
    }
    Ok(())
}

fn request_hash(request: &MutationRequest) -> Result<String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct HashMaterial<'a> {
        actor: &'a str,
        expected_version: MutationExpectedVersion,
        approval_policy: MutationApprovalPolicy,
        command: &'a MutationCommand,
    }
    let bytes = serde_json::to_vec(&HashMaterial {
        actor: &request.actor,
        expected_version: request.expected_version,
        approval_policy: request.approval_policy,
        command: &request.command,
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn require_absent(expected: MutationExpectedVersion) -> Result<()> {
    if expected != MutationExpectedVersion::Absent {
        return Err(invalid(
            "create mutations require an absent expected version",
        ));
    }
    Ok(())
}

fn require_exact(expected: MutationExpectedVersion) -> Result<u64> {
    match expected {
        MutationExpectedVersion::Exact(version) if version > 0 => Ok(version),
        _ => Err(invalid(
            "update mutations require a positive exact expected version",
        )),
    }
}

fn require_version(resource: &str, actual: u64, expected: u64) -> Result<()> {
    if actual != expected {
        return Err(Error::Conflict(format!(
            "{resource} version conflict: expected {expected}, found {actual}"
        )));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid(format!("{field} must be a UUID")))
}

fn validate_project_target(transaction: &Transaction<'_>, project_id: &str) -> Result<()> {
    let project = query_project(transaction, project_id)?;
    if project.deleted_at.is_some() || project.archived_at.is_some() {
        return Err(Error::Conflict(
            "task project must be active before it can receive remote changes".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn validate_task_create(input: &CreateTaskInput) -> Result<()> {
    validate_required_text("task title", &input.title, MAX_TITLE_CHARS)?;
    validate_optional_text(
        "task description",
        &input.description,
        MAX_TASK_DESCRIPTION_CHARS,
    )?;
    if input.priority > 4 {
        return Err(invalid("task priority must be between 0 and 4"));
    }
    if matches!(input.status, TaskStatus::Done | TaskStatus::Cancelled) {
        return Err(invalid("new tasks cannot start as done or cancelled"));
    }
    Ok(())
}

fn validate_task_patch(patch: &TaskPatch) -> Result<()> {
    if let Some(title) = patch.title.as_deref() {
        validate_required_text("task title", title, MAX_TITLE_CHARS)?;
    }
    if let Some(description) = patch.description.as_deref() {
        validate_optional_text("task description", description, MAX_TASK_DESCRIPTION_CHARS)?;
    }
    if patch.priority.is_some_and(|priority| priority > 4) {
        return Err(invalid("task priority must be between 0 and 4"));
    }
    if patch.clear_project && patch.project_id.is_some() {
        return Err(invalid("project cannot be both set and cleared"));
    }
    if patch.clear_due_date && patch.due_date.is_some() {
        return Err(invalid("due date cannot be both set and cleared"));
    }
    Ok(())
}

fn validate_task_transition(current: TaskStatus, requested: Option<TaskStatus>) -> Result<()> {
    let Some(next) = requested else {
        return Ok(());
    };
    if current == next {
        return Ok(());
    }
    let allowed = match current {
        TaskStatus::Inbox => matches!(
            next,
            TaskStatus::Todo | TaskStatus::InProgress | TaskStatus::Cancelled
        ),
        TaskStatus::Todo => matches!(
            next,
            TaskStatus::Inbox
                | TaskStatus::InProgress
                | TaskStatus::Blocked
                | TaskStatus::Done
                | TaskStatus::Cancelled
        ),
        TaskStatus::InProgress => matches!(
            next,
            TaskStatus::Todo | TaskStatus::Blocked | TaskStatus::Done | TaskStatus::Cancelled
        ),
        TaskStatus::Blocked => matches!(
            next,
            TaskStatus::Todo | TaskStatus::InProgress | TaskStatus::Cancelled
        ),
        TaskStatus::Done => matches!(next, TaskStatus::Todo | TaskStatus::InProgress),
        TaskStatus::Cancelled => matches!(next, TaskStatus::Inbox | TaskStatus::Todo),
    };
    if !allowed {
        return Err(invalid(format!(
            "task status cannot transition from {} to {}",
            current.as_str(),
            next.as_str()
        )));
    }
    Ok(())
}

fn task_patch_changes(current: &Task, patch: &TaskPatch) -> bool {
    patch
        .title
        .as_ref()
        .is_some_and(|value| value.trim() != current.title)
        || patch
            .description
            .as_ref()
            .is_some_and(|value| value.trim() != current.description)
        || patch.status.is_some_and(|value| value != current.status)
        || patch
            .priority
            .is_some_and(|value| value != current.priority)
        || patch
            .project_id
            .as_ref()
            .is_some_and(|value| Some(value) != current.project_id.as_ref())
        || (patch.clear_project && current.project_id.is_some())
        || patch
            .due_date
            .is_some_and(|value| Some(value) != current.due_date)
        || (patch.clear_due_date && current.due_date.is_some())
}

fn validate_note_create(input: &CreateNoteInput) -> Result<()> {
    validate_required_text("note title", &input.title, MAX_TITLE_CHARS)?;
    validate_optional_text("note body", &input.body, MAX_NOTE_BODY_CHARS)
}

fn validate_note_patch(patch: &NotePatch) -> Result<()> {
    if let Some(title) = patch.title.as_deref() {
        validate_required_text("note title", title, MAX_TITLE_CHARS)?;
    }
    if let Some(body) = patch.body.as_deref() {
        validate_optional_text("note body", body, MAX_NOTE_BODY_CHARS)?;
    }
    if patch.clear_note_date && patch.note_date.is_some() {
        return Err(invalid("note date cannot be both set and cleared"));
    }
    Ok(())
}

fn note_patch_changes(current: &Note, patch: &NotePatch) -> bool {
    patch
        .note_type
        .is_some_and(|value| value != current.note_type)
        || patch
            .title
            .as_ref()
            .is_some_and(|value| value.trim() != current.title)
        || patch
            .body
            .as_ref()
            .is_some_and(|value| value.trim() != current.body)
        || patch
            .note_date
            .is_some_and(|value| Some(value) != current.note_date)
        || (patch.clear_note_date && current.note_date.is_some())
}

fn validate_required_text(field: &str, value: &str, max: usize) -> Result<()> {
    let length = value.trim().chars().count();
    if length == 0 || length > max {
        return Err(invalid(format!(
            "{field} must contain between 1 and {max} characters"
        )));
    }
    Ok(())
}

fn validate_optional_text(field: &str, value: &str, max: usize) -> Result<()> {
    if value.chars().count() > max {
        return Err(invalid(format!("{field} cannot exceed {max} characters")));
    }
    Ok(())
}

fn map_audit_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<MutationAuditEvent> {
    let operation: String = row.get(4)?;
    let before_json: Option<String> = row.get(9)?;
    let after_json: String = row.get(10)?;
    let version: i64 = row.get(8)?;
    Ok(MutationAuditEvent {
        id: row.get(0)?,
        idempotency_key: row.get(1)?,
        actor: row.get(2)?,
        request_id: row.get(3)?,
        operation: parse_operation(4, &operation)?,
        resource_type: row.get(5)?,
        resource_id: row.get(6)?,
        expected_version: row.get(7)?,
        resulting_version: u64::try_from(version).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(8, Type::Integer, Box::new(error))
        })?,
        before: before_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(9, Type::Text, Box::new(error))
            })?,
        after: serde_json::from_str(&after_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(10, Type::Text, Box::new(error))
        })?,
        result: row.get(11)?,
        approval_policy: row.get(12)?,
        created_at: row.get(13)?,
    })
}

fn parse_operation(index: usize, value: &str) -> rusqlite::Result<MutationOperation> {
    match value {
        "task.create" => Ok(MutationOperation::TaskCreate),
        "task.update" => Ok(MutationOperation::TaskUpdate),
        "note.create" => Ok(MutationOperation::NoteCreate),
        "note.update" => Ok(MutationOperation::NoteUpdate),
        "checklist.set_done" => Ok(MutationOperation::ChecklistSetDone),
        "memory.create" => Ok(MutationOperation::MemoryCreate),
        "memory.update" => Ok(MutationOperation::MemoryUpdate),
        "memory.delete" => Ok(MutationOperation::MemoryDelete),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Text,
            invalid(format!("unknown mutation operation: {value}")).into(),
        )),
    }
}
