use axum::{
    Extension, Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tm_core::{
    ChecklistItem, CreateNoteInput, CreateTaskInput, Error as CoreError, MutationApprovalPolicy,
    MutationCommand, MutationExpectedVersion, MutationOperation, MutationRequest, MutationResult,
    Note, NotePatch, NoteType, Task, TaskPatch, TaskStatus,
};

use super::{
    ApiEnvelope, ApiError, AppState, RequestId,
    read_api::{ChecklistItemDto, NoteDto, TaskDto},
};

pub(super) const MAX_MUTATION_BODY_BYTES: usize = 64 * 1024;
const IDEMPOTENCY_KEY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
const MUTATION_CONFIRM_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-mutation");
const IDEMPOTENCY_REPLAYED_HEADER: HeaderName =
    HeaderName::from_static("x-tm-idempotency-replayed");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateTaskBody {
    project_id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    status: Option<TaskStatus>,
    priority: Option<u8>,
    due_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UpdateTaskBody {
    title: Option<String>,
    description: Option<String>,
    status: Option<TaskStatus>,
    priority: Option<u8>,
    project_id: Option<String>,
    #[serde(default)]
    clear_project: bool,
    due_date: Option<NaiveDate>,
    #[serde(default)]
    clear_due_date: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateNoteBody {
    note_type: NoteType,
    title: String,
    #[serde(default)]
    body: String,
    note_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UpdateNoteBody {
    note_type: Option<NoteType>,
    title: Option<String>,
    body: Option<String>,
    note_date: Option<NaiveDate>,
    #[serde(default)]
    clear_note_date: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SetChecklistDoneBody {
    is_done: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MutationResponseData {
    operation: MutationOperation,
    resource_type: String,
    resource_id: String,
    version: u64,
    replayed: bool,
    item: Value,
}

pub(super) fn body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_MUTATION_BODY_BYTES)
}

pub(super) async fn create_task(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    payload: Result<Json<CreateTaskBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, &request_id)?.0;
    let operation = MutationOperation::TaskCreate;
    let metadata = mutation_metadata(
        &headers,
        operation,
        ExpectedVersionKind::Absent,
        &request_id,
    )?;
    let command = MutationCommand::TaskCreate {
        input: CreateTaskInput {
            project_id: body.project_id,
            title: body.title,
            description: body.description,
            status: body.status.unwrap_or(TaskStatus::Inbox),
            priority: body.priority.unwrap_or(0),
            due_date: body.due_date,
        },
    };
    execute_mutation(state, request_id, metadata, command, StatusCode::CREATED).await
}

pub(super) async fn update_task(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(task_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<UpdateTaskBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, &request_id)?.0;
    let operation = MutationOperation::TaskUpdate;
    let metadata = mutation_metadata(&headers, operation, ExpectedVersionKind::Exact, &request_id)?;
    let command = MutationCommand::TaskUpdate {
        task_id,
        patch: TaskPatch {
            title: body.title,
            description: body.description,
            status: body.status,
            priority: body.priority,
            project_id: body.project_id,
            clear_project: body.clear_project,
            due_date: body.due_date,
            clear_due_date: body.clear_due_date,
        },
    };
    execute_mutation(state, request_id, metadata, command, StatusCode::OK).await
}

pub(super) async fn create_note(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    payload: Result<Json<CreateNoteBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, &request_id)?.0;
    let operation = MutationOperation::NoteCreate;
    let metadata = mutation_metadata(
        &headers,
        operation,
        ExpectedVersionKind::Absent,
        &request_id,
    )?;
    let command = MutationCommand::NoteCreate {
        input: CreateNoteInput {
            note_type: body.note_type,
            title: body.title,
            body: body.body,
            note_date: body.note_date,
        },
    };
    execute_mutation(state, request_id, metadata, command, StatusCode::CREATED).await
}

pub(super) async fn update_note(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(note_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<UpdateNoteBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, &request_id)?.0;
    let operation = MutationOperation::NoteUpdate;
    let metadata = mutation_metadata(&headers, operation, ExpectedVersionKind::Exact, &request_id)?;
    let command = MutationCommand::NoteUpdate {
        note_id,
        patch: NotePatch {
            note_type: body.note_type,
            title: body.title,
            body: body.body,
            note_date: body.note_date,
            clear_note_date: body.clear_note_date,
        },
    };
    execute_mutation(state, request_id, metadata, command, StatusCode::OK).await
}

pub(super) async fn set_checklist_done(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(item_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<SetChecklistDoneBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, &request_id)?.0;
    let operation = MutationOperation::ChecklistSetDone;
    let metadata = mutation_metadata(&headers, operation, ExpectedVersionKind::Exact, &request_id)?;
    let command = MutationCommand::ChecklistSetDone {
        item_id,
        is_done: body.is_done,
    };
    execute_mutation(state, request_id, metadata, command, StatusCode::OK).await
}

#[derive(Debug)]
struct MutationMetadata {
    idempotency_key: String,
    expected_version: MutationExpectedVersion,
    approval_policy: MutationApprovalPolicy,
}

#[derive(Debug, Clone, Copy)]
enum ExpectedVersionKind {
    Absent,
    Exact,
}

fn mutation_metadata(
    headers: &HeaderMap,
    operation: MutationOperation,
    expected_kind: ExpectedVersionKind,
    request_id: &RequestId,
) -> Result<MutationMetadata, ApiError> {
    let idempotency_key = required_header(headers, &IDEMPOTENCY_KEY_HEADER, request_id)?.to_owned();
    let confirmation = required_header(headers, &MUTATION_CONFIRM_HEADER, request_id)?;
    if confirmation != operation.as_str() {
        return Err(ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "MUTATION_CONFIRMATION_REQUIRED",
            message: format!(
                "set x-tm-confirm-mutation to {} for this operation",
                operation.as_str()
            ),
            request_id: request_id.0.clone(),
        });
    }

    let expected_version = match expected_kind {
        ExpectedVersionKind::Absent => {
            let if_none_match = required_header(headers, &header::IF_NONE_MATCH, request_id)?;
            if if_none_match != "*" {
                return Err(ApiError {
                    status: StatusCode::PRECONDITION_REQUIRED,
                    code: "EXPECTED_VERSION_REQUIRED",
                    message: "create mutations require If-None-Match: *".to_owned(),
                    request_id: request_id.0.clone(),
                });
            }
            MutationExpectedVersion::Absent
        }
        ExpectedVersionKind::Exact => {
            let if_match = required_header(headers, &header::IF_MATCH, request_id)?;
            let version = parse_if_match_version(if_match).ok_or_else(|| ApiError {
                status: StatusCode::BAD_REQUEST,
                code: "INVALID_EXPECTED_VERSION",
                message: "If-Match must contain one quoted positive integer version".to_owned(),
                request_id: request_id.0.clone(),
            })?;
            MutationExpectedVersion::Exact(version)
        }
    };

    Ok(MutationMetadata {
        idempotency_key,
        expected_version,
        approval_policy: approval_policy(operation),
    })
}

const fn approval_policy(_operation: MutationOperation) -> MutationApprovalPolicy {
    MutationApprovalPolicy::ExplicitUserConfirmation
}

fn required_header<'a>(
    headers: &'a HeaderMap,
    name: &HeaderName,
    request_id: &RequestId,
) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "MUTATION_PRECONDITION_REQUIRED",
            message: format!("{} header is required", name.as_str()),
            request_id: request_id.0.clone(),
        })
}

fn parse_if_match_version(value: &str) -> Option<u64> {
    let version = value.strip_prefix('"')?.strip_suffix('"')?;
    let version = version.parse::<u64>().ok()?;
    (version > 0).then_some(version)
}

fn parse_json<T>(
    payload: Result<Json<T>, JsonRejection>,
    request_id: &RequestId,
) -> Result<Json<T>, ApiError> {
    payload.map_err(|rejection| {
        let too_large = rejection.status() == StatusCode::PAYLOAD_TOO_LARGE;
        ApiError {
            status: if too_large {
                StatusCode::PAYLOAD_TOO_LARGE
            } else {
                StatusCode::BAD_REQUEST
            },
            code: if too_large {
                "MUTATION_REQUEST_TOO_LARGE"
            } else {
                "INVALID_MUTATION_JSON"
            },
            message: if too_large {
                format!("mutation request exceeds {MAX_MUTATION_BODY_BYTES} bytes")
            } else {
                "mutation request body does not match the API contract".to_owned()
            },
            request_id: request_id.0.clone(),
        }
    })
}

async fn execute_mutation(
    state: AppState,
    request_id: RequestId,
    metadata: MutationMetadata,
    command: MutationCommand,
    success_status: StatusCode,
) -> Result<Response, ApiError> {
    let error_request_id = request_id.0.clone();
    let core_request = MutationRequest {
        idempotency_key: metadata.idempotency_key,
        expected_version: metadata.expected_version,
        actor: "single-user".to_owned(),
        request_id: request_id.0.clone(),
        approval_policy: metadata.approval_policy,
        command,
    };
    let result =
        tokio::task::spawn_blocking(move || state.core.execute_remote_mutation(core_request))
            .await
            .map_err(|_| ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "MUTATION_WORKER_FAILED",
                message: "TM mutation worker failed".to_owned(),
                request_id: error_request_id.clone(),
            })?
            .map_err(|error| map_core_error(error, error_request_id))?;

    tracing::info!(
        request_id = %request_id.0,
        operation = result.operation.as_str(),
        resource_type = %result.resource_type,
        resource_id = %result.resource_id,
        version = result.version,
        replayed = result.replayed,
        "controlled TM mutation completed"
    );

    mutation_response(request_id, result, success_status)
}

fn mutation_response(
    request_id: RequestId,
    result: MutationResult,
    status: StatusCode,
) -> Result<Response, ApiError> {
    let item = match result.operation {
        MutationOperation::TaskCreate | MutationOperation::TaskUpdate => {
            let task: Task = serde_json::from_value(result.entity)
                .map_err(|_| internal_response_error(&request_id))?;
            serde_json::to_value(TaskDto::from(task))
                .map_err(|_| internal_response_error(&request_id))?
        }
        MutationOperation::NoteCreate | MutationOperation::NoteUpdate => {
            let note: Note = serde_json::from_value(result.entity)
                .map_err(|_| internal_response_error(&request_id))?;
            serde_json::to_value(NoteDto::from(note))
                .map_err(|_| internal_response_error(&request_id))?
        }
        MutationOperation::ChecklistSetDone => {
            let item: ChecklistItem = serde_json::from_value(result.entity)
                .map_err(|_| internal_response_error(&request_id))?;
            serde_json::to_value(ChecklistItemDto::from(item))
                .map_err(|_| internal_response_error(&request_id))?
        }
        MutationOperation::MemoryCreate
        | MutationOperation::MemoryUpdate
        | MutationOperation::MemoryDelete => {
            return Err(internal_response_error(&request_id));
        }
    };
    let etag = HeaderValue::from_str(&format!("\"{}\"", result.version))
        .map_err(|_| internal_response_error(&request_id))?;
    let replayed = result.replayed;
    let mut response = (
        status,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data: MutationResponseData {
                operation: result.operation,
                resource_type: result.resource_type,
                resource_id: result.resource_id,
                version: result.version,
                replayed,
                item,
            },
        }),
    )
        .into_response();
    response.headers_mut().insert(header::ETAG, etag);
    if replayed {
        response.headers_mut().insert(
            IDEMPOTENCY_REPLAYED_HEADER,
            HeaderValue::from_static("true"),
        );
    }
    Ok(response)
}

fn map_core_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidInput(message) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_MUTATION",
            message,
            request_id,
        },
        CoreError::NotFound { entity, id } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "MUTATION_RESOURCE_NOT_FOUND",
            message: format!("{entity} not found: {id}"),
            request_id,
        },
        CoreError::Conflict(message) => ApiError {
            status: StatusCode::CONFLICT,
            code: "MUTATION_CONFLICT",
            message,
            request_id,
        },
        _ => {
            tracing::error!(%request_id, "controlled TM mutation failed internally");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "MUTATION_FAILED",
                message: "TM mutation could not be completed".to_owned(),
                request_id,
            }
        }
    }
}

fn internal_response_error(request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "MUTATION_RESPONSE_INVALID",
        message: "TM mutation response could not be serialized".to_owned(),
        request_id: request_id.0.clone(),
    }
}
