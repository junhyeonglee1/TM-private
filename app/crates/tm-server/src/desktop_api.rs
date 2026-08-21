use std::str::FromStr;

use axum::{
    Extension, Json,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, HeaderName, StatusCode},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tm_core::{
    CalendarEvent, CalendarMonth, CalendarOccurrence, DesktopCommand, Error as CoreError,
    execute_desktop_command,
};

use super::{ApiEnvelope, ApiError, AppState, RequestId, expense_api};

const CONFIRM_COMMAND_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-desktop-command");
const MAX_DESKTOP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DesktopCommandBody {
    #[serde(default = "empty_args")]
    args: Value,
}

fn empty_args() -> Value {
    Value::Object(serde_json::Map::new())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CalendarMonthDto {
    month: String,
    month_start: chrono::NaiveDate,
    month_end: chrono::NaiveDate,
    events: Vec<CalendarEvent>,
    occurrences: Vec<CalendarOccurrence>,
    expense_occurrences: Vec<expense_api::RecurringExpenseOccurrenceDto>,
}

pub(super) async fn invoke(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(command_name): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<DesktopCommandBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<Value>>, ApiError> {
    let command = DesktopCommand::from_str(&command_name).map_err(|_| ApiError {
        status: StatusCode::NOT_FOUND,
        code: "DESKTOP_COMMAND_NOT_FOUND",
        message: "the requested desktop command is not available".to_owned(),
        request_id: request_id.0.clone(),
    })?;
    if matches!(
        command,
        DesktopCommand::GetExpenseSummary
            | DesktopCommand::ListExpenseTransactions
            | DesktopCommand::ListExpenseReviews
            | DesktopCommand::ListRecurringExpenses
            | DesktopCommand::GetRecurringExpenseOccurrences
            | DesktopCommand::PreviewExpenseImport
            | DesktopCommand::ExecuteExpenseMutation
    ) {
        return Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "EXPENSE_REST_API_REQUIRED",
            message: "expense operations must use the dedicated expense API".to_owned(),
            request_id: request_id.0,
        });
    }
    if !command.is_read_only() {
        let confirmation = headers
            .get(&CONFIRM_COMMAND_HEADER)
            .and_then(|value| value.to_str().ok());
        if confirmation != Some(command.as_str()) {
            return Err(ApiError {
                status: StatusCode::PRECONDITION_REQUIRED,
                code: "DESKTOP_COMMAND_CONFIRMATION_REQUIRED",
                message: format!("set x-tm-confirm-desktop-command to {}", command.as_str()),
                request_id: request_id.0,
            });
        }
    }

    let body = payload.map_err(|rejection| ApiError {
        status: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        },
        code: if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            "DESKTOP_COMMAND_TOO_LARGE"
        } else {
            "INVALID_DESKTOP_COMMAND_JSON"
        },
        message: "desktop command body is invalid".to_owned(),
        request_id: request_id.0.clone(),
    })?;

    let error_request_id = request_id.0.clone();
    let args = body.0.args;
    let core = state.core.clone();
    let result = tokio::task::spawn_blocking(move || execute_desktop_command(&core, command, args))
        .await
        .map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "DESKTOP_COMMAND_WORKER_FAILED",
            message: "desktop command worker failed".to_owned(),
            request_id: error_request_id.clone(),
        })?
        .map_err(|error| map_core_error(error, error_request_id))?;
    let result = if command == DesktopCommand::GetCalendarMonth {
        let month = serde_json::from_value::<CalendarMonth>(result).map_err(|_| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "CALENDAR_RESPONSE_INVALID",
            message: "calendar response could not be decoded".to_owned(),
            request_id: request_id.0.clone(),
        })?;
        let crypto = expense_api::expense_crypto(&state, &request_id)?;
        let expense_occurrences = month
            .expense_occurrences
            .into_iter()
            .map(|item| expense_api::decrypt_occurrence(item, crypto, &request_id))
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_value(CalendarMonthDto {
            month: month.month,
            month_start: month.month_start,
            month_end: month.month_end,
            events: month.events,
            occurrences: month.occurrences,
            expense_occurrences,
        })
        .map_err(|_| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "CALENDAR_RESPONSE_INVALID",
            message: "calendar response could not be encoded".to_owned(),
            request_id: request_id.0.clone(),
        })?
    } else {
        result
    };
    if serde_json::to_vec(&result)
        .map_err(|_| ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "DESKTOP_RESPONSE_INVALID",
            message: "desktop command response could not be serialized".to_owned(),
            request_id: request_id.0.clone(),
        })?
        .len()
        > MAX_DESKTOP_RESPONSE_BYTES
    {
        return Err(ApiError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "DESKTOP_RESPONSE_TOO_LARGE",
            message: "desktop command response exceeded the safety limit".to_owned(),
            request_id: request_id.0,
        });
    }

    tracing::info!(
        request_id = %request_id.0,
        command = command.as_str(),
        read_only = command.is_read_only(),
        "first-party desktop command completed"
    );

    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: result,
    }))
}

fn map_core_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidInput(_) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_DESKTOP_COMMAND",
            message: "desktop command input was rejected".to_owned(),
            request_id,
        },
        CoreError::NotFound { .. } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "DESKTOP_RESOURCE_NOT_FOUND",
            message: "the requested TM item was not found".to_owned(),
            request_id,
        },
        CoreError::Conflict(_) => ApiError {
            status: StatusCode::CONFLICT,
            code: "DESKTOP_COMMAND_CONFLICT",
            message: "the TM item changed before the command completed".to_owned(),
            request_id,
        },
        _ => {
            tracing::error!(%request_id, "desktop command failed internally");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "DESKTOP_COMMAND_FAILED",
                message: "desktop command could not be completed".to_owned(),
                request_id,
            }
        }
    }
}
