use axum::{
    Extension, Json,
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tm_core::{
    AssistantActionExecution, AssistantActionPayload, AssistantActionRequest,
    AssistantActionStatus, CreateMemoryInput, Error as CoreError, MemoryPatch, MutationOperation,
    Task,
};

use super::{ApiEnvelope, ApiError, AppState, RequestId, read_api::TaskDto};

pub(super) const MAX_ACTION_BODY_BYTES: usize = 16 * 1024;
const IDEMPOTENCY_KEY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
const ACTION_CONFIRM_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-action");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ApproveActionBody {
    expected_revision: u64,
    payload_sha256: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct FinishActionBody {
    expected_revision: u64,
    payload_sha256: String,
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActionCollection {
    items: Vec<ActionDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActionDto {
    id: String,
    operation: &'static str,
    status: AssistantActionStatus,
    revision: u64,
    payload_sha256: String,
    preview: ActionPreview,
    created_at: String,
    expires_at: String,
    approved_at: Option<String>,
    completed_at: Option<String>,
    terminal_at: Option<String>,
    failure_code: Option<String>,
    result: Option<ActionResultDto>,
    approval: ApprovalContract,
}

#[derive(Debug, Serialize)]
#[serde(tag = "operation")]
enum ActionPreview {
    #[serde(rename = "task.create")]
    TaskCreate { input: TaskCreatePreview },
    #[serde(rename = "memory.create")]
    MemoryCreate { input: CreateMemoryInput },
    #[serde(rename = "memory.update")]
    MemoryUpdate {
        memory_id: String,
        expected_revision: u64,
        patch: MemoryPatch,
    },
    #[serde(rename = "memory.delete")]
    MemoryDelete {
        memory_id: String,
        expected_revision: u64,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskCreatePreview {
    project_id: Option<String>,
    title: String,
    description: String,
    status: tm_core::TaskStatus,
    priority: u8,
    due_date: Option<chrono::NaiveDate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionResultDto {
    resource_id: String,
    version: u64,
    item: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalContract {
    one_time: bool,
    executes_immediately_after_approval: bool,
    automatic_without_approval: bool,
    required_confirmation: String,
    requires_idempotency_key: bool,
    payload_locked: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ActionExecutionDto {
    action: ActionDto,
    mutation_replayed: bool,
}

pub(super) fn body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_ACTION_BODY_BYTES)
}

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ApiEnvelope<ActionCollection>>, ApiError> {
    let actions = tokio::task::spawn_blocking(move || state.core.list_assistant_actions())
        .await
        .map_err(|_| internal_error(&request_id))?
        .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    let items = actions
        .into_iter()
        .map(|action| action_dto(action, &request_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: ActionCollection { items },
    }))
}

pub(super) async fn get(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(action_id): Path<String>,
) -> Result<Json<ApiEnvelope<ActionDto>>, ApiError> {
    let action = tokio::task::spawn_blocking(move || state.core.get_assistant_action(&action_id))
        .await
        .map_err(|_| internal_error(&request_id))?
        .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    let data = action_dto(action, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn approve(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(action_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<ApproveActionBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<ActionExecutionDto>>, ApiError> {
    let confirm_core = state.core.clone();
    let confirm_action_id = action_id.clone();
    let action =
        tokio::task::spawn_blocking(move || confirm_core.get_assistant_action(&confirm_action_id))
            .await
            .map_err(|_| internal_error(&request_id))?
            .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    require_confirmation(&headers, action.operation.as_str(), &request_id)?;
    let idempotency_key =
        required_header(&headers, &IDEMPOTENCY_KEY_HEADER, &request_id)?.to_owned();
    let body = parse_json(payload, &request_id)?.0;
    let core = state.core;
    let origin_request_id = request_id.0.clone();
    let execution = tokio::task::spawn_blocking(move || {
        core.approve_and_execute_assistant_action(
            &action_id,
            body.expected_revision,
            &body.payload_sha256,
            &idempotency_key,
            &origin_request_id,
        )
    })
    .await
    .map_err(|_| internal_error(&request_id))?
    .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    execution_response(execution, request_id)
}

pub(super) async fn reject(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(action_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<FinishActionBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<ActionDto>>, ApiError> {
    require_confirmation(&headers, "reject", &request_id)?;
    let body = parse_json(payload, &request_id)?.0;
    let core = state.core;
    let origin_request_id = request_id.0.clone();
    let action = tokio::task::spawn_blocking(move || {
        core.reject_assistant_action(
            &action_id,
            body.expected_revision,
            &body.payload_sha256,
            &origin_request_id,
            body.reason.as_deref(),
        )
    })
    .await
    .map_err(|_| internal_error(&request_id))?
    .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    let data = action_dto(action, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(action_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<FinishActionBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<ActionDto>>, ApiError> {
    require_confirmation(&headers, "cancel", &request_id)?;
    let body = parse_json(payload, &request_id)?.0;
    let core = state.core;
    let origin_request_id = request_id.0.clone();
    let action = tokio::task::spawn_blocking(move || {
        core.cancel_assistant_action(
            &action_id,
            body.expected_revision,
            &body.payload_sha256,
            &origin_request_id,
            body.reason.as_deref(),
        )
    })
    .await
    .map_err(|_| internal_error(&request_id))?
    .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    let data = action_dto(action, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

fn execution_response(
    execution: AssistantActionExecution,
    request_id: RequestId,
) -> Result<Json<ApiEnvelope<ActionExecutionDto>>, ApiError> {
    let action = action_dto(execution.action, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: ActionExecutionDto {
            action,
            mutation_replayed: execution.mutation_replayed,
        },
    }))
}

fn action_dto(
    action: AssistantActionRequest,
    request_id: &RequestId,
) -> Result<ActionDto, ApiError> {
    let operation = action.operation;
    let preview = match action.payload {
        AssistantActionPayload::TaskCreate { input } => ActionPreview::TaskCreate {
            input: TaskCreatePreview {
                project_id: input.project_id,
                title: input.title,
                description: input.description,
                status: input.status,
                priority: input.priority,
                due_date: input.due_date,
            },
        },
        AssistantActionPayload::MemoryCreate { input } => ActionPreview::MemoryCreate { input },
        AssistantActionPayload::MemoryUpdate {
            memory_id,
            expected_revision,
            patch,
        } => ActionPreview::MemoryUpdate {
            memory_id,
            expected_revision,
            patch,
        },
        AssistantActionPayload::MemoryDelete {
            memory_id,
            expected_revision,
        } => ActionPreview::MemoryDelete {
            memory_id,
            expected_revision,
        },
    };
    let result = action
        .result
        .map(|result| {
            let item = if operation == MutationOperation::TaskCreate {
                let task: Task = serde_json::from_value(result.entity)
                    .map_err(|_| response_error(request_id))?;
                serde_json::to_value(TaskDto::from(task)).map_err(|_| response_error(request_id))?
            } else {
                result.entity
            };
            Ok(ActionResultDto {
                resource_id: result.resource_id,
                version: result.version,
                item,
            })
        })
        .transpose()?;
    Ok(ActionDto {
        id: action.id,
        operation: operation.as_str(),
        status: action.status,
        revision: action.revision,
        payload_sha256: action.payload_sha256,
        preview,
        created_at: action.created_at,
        expires_at: action.expires_at,
        approved_at: action.approved_at,
        completed_at: action.completed_at,
        terminal_at: action.terminal_at,
        failure_code: action.failure_code,
        result,
        approval: ApprovalContract {
            one_time: true,
            executes_immediately_after_approval: true,
            automatic_without_approval: false,
            required_confirmation: operation.as_str().to_owned(),
            requires_idempotency_key: true,
            payload_locked: true,
        },
    })
}

fn require_confirmation(
    headers: &HeaderMap,
    expected: &str,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    let actual = required_header(headers, &ACTION_CONFIRM_HEADER, request_id)?;
    if actual != expected {
        return Err(ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "ASSISTANT_ACTION_CONFIRMATION_REQUIRED",
            message: format!("set x-tm-confirm-action to {expected}"),
            request_id: request_id.0.clone(),
        });
    }
    Ok(())
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
            code: "ASSISTANT_ACTION_PRECONDITION_REQUIRED",
            message: format!("required header is missing or invalid: {name}"),
            request_id: request_id.0.clone(),
        })
}

fn parse_json<T>(
    payload: Result<Json<T>, JsonRejection>,
    request_id: &RequestId,
) -> Result<Json<T>, ApiError> {
    payload.map_err(|rejection| ApiError {
        status: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        },
        code: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            "ASSISTANT_ACTION_REQUEST_TOO_LARGE"
        } else {
            "INVALID_ASSISTANT_ACTION_JSON"
        },
        message: "assistant action body does not match the API contract".to_owned(),
        request_id: request_id.0.clone(),
    })
}

fn map_core_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidInput(message) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_ASSISTANT_ACTION",
            message,
            request_id,
        },
        CoreError::NotFound { entity, id } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "ASSISTANT_ACTION_NOT_FOUND",
            message: format!("{entity} not found: {id}"),
            request_id,
        },
        CoreError::Conflict(message) => ApiError {
            status: StatusCode::CONFLICT,
            code: "ASSISTANT_ACTION_CONFLICT",
            message,
            request_id,
        },
        _ => {
            tracing::error!(%request_id, "assistant action failed internally");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "ASSISTANT_ACTION_FAILED",
                message: "assistant action could not be completed".to_owned(),
                request_id,
            }
        }
    }
}

fn internal_error(request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "ASSISTANT_ACTION_WORKER_FAILED",
        message: "assistant action worker was unavailable".to_owned(),
        request_id: request_id.0.clone(),
    }
}

fn response_error(request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "ASSISTANT_ACTION_RESPONSE_INVALID",
        message: "assistant action response could not be serialized".to_owned(),
        request_id: request_id.0.clone(),
    }
}
