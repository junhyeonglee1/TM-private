use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use tm_core::{
    AssistantMemory, AssistantMemoryEvent, Error as CoreError, MemoryKind, MemorySearchFilter,
    MemorySearchResult,
};

use super::{ApiEnvelope, ApiError, AppState, RequestId};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ListQuery {
    include_deleted: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SearchQuery {
    query: String,
    kind: Option<MemoryKind>,
    openai_only: Option<bool>,
    limit: Option<usize>,
    max_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemoryCollection {
    items: Vec<AssistantMemory>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MemoryEventCollection {
    items: Vec<AssistantMemoryEvent>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiEnvelope<MemoryCollection>>, ApiError> {
    let include_deleted = query.include_deleted.unwrap_or(false);
    let items =
        tokio::task::spawn_blocking(move || state.core.list_assistant_memories(include_deleted))
            .await
            .map_err(|_| worker_error(&request_id))?
            .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: MemoryCollection { items },
    }))
}

pub(super) async fn get(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(memory_id): Path<String>,
) -> Result<Json<ApiEnvelope<AssistantMemory>>, ApiError> {
    let memory = tokio::task::spawn_blocking(move || state.core.get_assistant_memory(&memory_id))
        .await
        .map_err(|_| worker_error(&request_id))?
        .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: memory,
    }))
}

pub(super) async fn search(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<ApiEnvelope<MemorySearchResult>>, ApiError> {
    let filter = MemorySearchFilter {
        query: query.query,
        kind: query.kind,
        openai_only: query.openai_only.unwrap_or(false),
        max_items: query.limit.unwrap_or(12),
        max_bytes: query.max_bytes.unwrap_or(6 * 1024),
    };
    let result = tokio::task::spawn_blocking(move || state.core.search_assistant_memories(filter))
        .await
        .map_err(|_| worker_error(&request_id))?
        .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: result,
    }))
}

pub(super) async fn events(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(memory_id): Path<String>,
) -> Result<Json<ApiEnvelope<MemoryEventCollection>>, ApiError> {
    let items =
        tokio::task::spawn_blocking(move || state.core.list_assistant_memory_events(&memory_id))
            .await
            .map_err(|_| worker_error(&request_id))?
            .map_err(|error| map_core_error(error, request_id.0.clone()))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: MemoryEventCollection { items },
    }))
}

fn map_core_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidInput(message) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_MEMORY_QUERY",
            message,
            request_id,
        },
        CoreError::NotFound { entity, id } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "MEMORY_NOT_FOUND",
            message: format!("{entity} not found: {id}"),
            request_id,
        },
        CoreError::Conflict(message) => ApiError {
            status: StatusCode::CONFLICT,
            code: "MEMORY_CONFLICT",
            message,
            request_id,
        },
        _ => ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "MEMORY_READ_FAILED",
            message: "assistant memory could not be read".to_owned(),
            request_id,
        },
    }
}

fn worker_error(request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "MEMORY_WORKER_FAILED",
        message: "assistant memory worker was unavailable".to_owned(),
        request_id: request_id.0.clone(),
    }
}
