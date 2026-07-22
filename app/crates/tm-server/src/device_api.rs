use axum::{
    Extension, Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{HOST, ORIGIN, SET_COOKIE},
    },
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tm_core::{DevicePairing, Error as CoreError, RegisteredDevice};
use url::Url;

use crate::{
    ApiEnvelope, ApiError, AppState, AuthenticatedSession, AuthenticatedSubject, RequestId,
    auth::{DEVICE_TOKEN_PREFIX, generate_pairing_code, generate_secret, sha256_hex},
};

pub(super) const DEVICE_COOKIE: &str = "__Host-tm_device";
const CSRF_COOKIE: &str = "__Host-tm_csrf";
const CSRF_HEADER: &str = "x-tm-csrf";
const ADMIN_CONFIRM_HEADER: &str = "x-tm-confirm-device-admin";
const DEVICE_COOKIE_MAX_AGE_SECONDS: i64 = 90 * 24 * 60 * 60;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartPairingBody {
    device_label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompletePairingBody {
    polling_secret: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovePairingBody {
    code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingStartData {
    pairing: DevicePairing,
    code: String,
    polling_secret: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PairingCollection {
    items: Vec<DevicePairing>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceCollection {
    items: Vec<RegisteredDevice>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RevokeAllResult {
    revoked_count: usize,
}

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/device-pairings", post(start_pairing))
        .route(
            "/api/v1/device-pairings/{id}/complete",
            post(complete_pairing),
        )
        .route("/api/v1/device/self", get(device_self))
        .route("/api/v1/device/logout", post(logout))
        .route("/api/v1/admin/device-pairings", get(list_pairings))
        .route(
            "/api/v1/admin/device-pairings/{id}/approve",
            post(approve_pairing),
        )
        .route("/api/v1/admin/devices", get(list_devices))
        .route("/api/v1/admin/devices/revoke-all", post(revoke_all_devices))
        .route("/api/v1/admin/devices/{id}/revoke", post(revoke_device))
}

pub(super) fn is_public_path(path: &str) -> bool {
    matches!(path, "/healthz" | "/readyz" | "/api/v1/device-pairings")
        || (path.starts_with("/api/v1/device-pairings/") && path.ends_with("/complete"))
        || path == "/mobile"
        || path.starts_with("/mobile/")
}

pub(super) fn device_route_allowed(method: &Method, path: &str) -> bool {
    if matches!(path, "/api/v1/auth/status" | "/api/v1/device/self") {
        return method == Method::GET;
    }
    if path == "/api/v1/device/logout" {
        return method == Method::POST;
    }
    if path == "/api/v1/assistant/query" {
        return method == Method::POST;
    }
    if path == "/api/v1/assistant/task-report" {
        return method == Method::POST;
    }
    if path == "/api/v1/assistant/task-reports/latest" {
        return method == Method::GET;
    }
    if path.starts_with("/api/v1/assistant/task-reports/") && path.ends_with("/feedback") {
        return method == Method::POST;
    }
    if path == "/api/v1/assistant/actions" {
        return method == Method::GET;
    }
    if path.starts_with("/api/v1/assistant/actions/") {
        return method == Method::GET || method == Method::POST;
    }
    if path.starts_with("/api/v1/assistant/memories") {
        return method == Method::GET;
    }
    match path {
        "/api/v1/projects" | "/api/v1/tags" | "/api/v1/sessions" | "/api/v1/worklogs" => {
            method == Method::GET
        }
        "/api/v1/tasks" | "/api/v1/notes" => method == Method::GET || method == Method::POST,
        "/api/v1/checklist" => method == Method::GET,
        _ if path.starts_with("/api/v1/tasks/")
            || path.starts_with("/api/v1/notes/")
            || path.starts_with("/api/v1/checklist/") =>
        {
            method == Method::PATCH
        }
        _ => false,
    }
}

pub(super) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for header in headers.get_all("cookie") {
        let value = header.to_str().ok()?;
        for part in value.split(';') {
            let (cookie_name, cookie_value) = part.trim().split_once('=')?;
            if cookie_name == name {
                if found.is_some() || cookie_value.is_empty() {
                    return None;
                }
                found = Some(cookie_value);
            }
        }
    }
    found
}

pub(super) fn valid_device_mutation_headers(headers: &HeaderMap, csrf_sha256: &str) -> bool {
    same_origin(headers)
        && headers
            .get(CSRF_HEADER)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty() && value.len() <= 128)
            .is_some_and(|value| constant_time_equal(&sha256_hex(value), csrf_sha256))
}

async fn start_pairing(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    payload: Result<Json<StartPairingBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<PairingStartData>>, ApiError> {
    require_same_origin(&headers, &request_id)?;
    let body = parse_json(payload, &request_id)?.0;
    let code = generate_pairing_code();
    let polling_secret = generate_secret("poll_");
    let code_sha256 = sha256_hex(&code);
    let polling_sha256 = sha256_hex(&polling_secret);
    let core = state.core;
    let secrets = tokio::task::spawn_blocking(move || {
        core.start_device_pairing(
            &body.device_label,
            &code,
            &code_sha256,
            &polling_secret,
            &polling_sha256,
            Utc::now(),
        )
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: PairingStartData {
            pairing: secrets.pairing,
            code: secrets.code,
            polling_secret: secrets.polling_secret,
        },
    }))
}

async fn complete_pairing(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Path(pairing_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<CompletePairingBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_same_origin(&headers, &request_id)?;
    let body = parse_json(payload, &request_id)?.0;
    let device_token = generate_secret(DEVICE_TOKEN_PREFIX);
    let csrf_token = generate_secret("tm_csrf_v1_");
    let polling_sha256 = sha256_hex(&body.polling_secret);
    let token_sha256 = sha256_hex(&device_token);
    let csrf_sha256 = sha256_hex(&csrf_token);
    let core = state.core;
    let device = tokio::task::spawn_blocking(move || {
        core.complete_device_pairing(
            &pairing_id,
            &polling_sha256,
            &token_sha256,
            &csrf_sha256,
            Utc::now(),
        )
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    let mut response = Json(ApiEnvelope {
        request_id: request_id.0,
        data: device,
    })
    .into_response();
    append_cookie(
        &mut response,
        &format!(
            "{DEVICE_COOKIE}={device_token}; Path=/; Max-Age={DEVICE_COOKIE_MAX_AGE_SECONDS}; Secure; HttpOnly; SameSite=Strict"
        ),
    )?;
    append_cookie(
        &mut response,
        &format!(
            "{CSRF_COOKIE}={csrf_token}; Path=/; Max-Age={DEVICE_COOKIE_MAX_AGE_SECONDS}; Secure; SameSite=Strict"
        ),
    )?;
    Ok(response)
}

async fn list_pairings(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
) -> Result<Json<ApiEnvelope<PairingCollection>>, ApiError> {
    require_admin(&session, &request_id)?;
    let items = tokio::task::spawn_blocking(move || state.core.list_device_pairings(Utc::now()))
        .await
        .map_err(|_| worker_error(&request_id))?
        .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: PairingCollection { items },
    }))
}

async fn approve_pairing(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    Path(pairing_id): Path<String>,
    headers: HeaderMap,
    payload: Result<Json<ApprovePairingBody>, JsonRejection>,
) -> Result<Json<ApiEnvelope<DevicePairing>>, ApiError> {
    require_admin(&session, &request_id)?;
    require_admin_confirmation(&headers, "approve", &request_id)?;
    let body = parse_json(payload, &request_id)?.0;
    if body.code.len() != 6 || !body.code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_PAIRING_CODE",
            message: "pairing code must be exactly six digits".to_owned(),
            request_id: request_id.0,
        });
    }
    let code_sha256 = sha256_hex(&body.code);
    let core = state.core;
    let pairing = tokio::task::spawn_blocking(move || {
        core.approve_device_pairing(&pairing_id, &code_sha256, "primary-admin", Utc::now())
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: pairing,
    }))
}

async fn list_devices(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
) -> Result<Json<ApiEnvelope<DeviceCollection>>, ApiError> {
    require_admin(&session, &request_id)?;
    let items = tokio::task::spawn_blocking(move || state.core.list_registered_devices(Utc::now()))
        .await
        .map_err(|_| worker_error(&request_id))?
        .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: DeviceCollection { items },
    }))
}

async fn revoke_device(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    Path(device_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ApiEnvelope<RegisteredDevice>>, ApiError> {
    require_admin(&session, &request_id)?;
    require_admin_confirmation(&headers, "revoke", &request_id)?;
    let device = tokio::task::spawn_blocking(move || {
        state.core.revoke_registered_device(
            &device_id,
            "primary-admin",
            "device_revoked",
            Utc::now(),
        )
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: device,
    }))
}

async fn revoke_all_devices(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
) -> Result<Json<ApiEnvelope<RevokeAllResult>>, ApiError> {
    require_admin(&session, &request_id)?;
    require_admin_confirmation(&headers, "revoke-all", &request_id)?;
    let revoked_count = tokio::task::spawn_blocking(move || {
        state
            .core
            .revoke_all_registered_devices("primary-admin", Utc::now())
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: RevokeAllResult { revoked_count },
    }))
}

async fn device_self(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
) -> Result<Json<ApiEnvelope<RegisteredDevice>>, ApiError> {
    let device_id = require_device(&session, &request_id)?.to_owned();
    let device = tokio::task::spawn_blocking(move || state.core.get_registered_device(&device_id))
        .await
        .map_err(|_| worker_error(&request_id))?
        .map_err(|error| map_core_error(error, &request_id))?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: device,
    }))
}

async fn logout(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
) -> Result<Response, ApiError> {
    let device_id = require_device(&session, &request_id)?.to_owned();
    let device = tokio::task::spawn_blocking(move || {
        state
            .core
            .revoke_registered_device(&device_id, "device-self", "device_logout", Utc::now())
    })
    .await
    .map_err(|_| worker_error(&request_id))?
    .map_err(|error| map_core_error(error, &request_id))?;
    let mut response = Json(ApiEnvelope {
        request_id: request_id.0,
        data: device,
    })
    .into_response();
    append_cookie(
        &mut response,
        &format!("{DEVICE_COOKIE}=; Path=/; Max-Age=0; Secure; HttpOnly; SameSite=Strict"),
    )?;
    append_cookie(
        &mut response,
        &format!("{CSRF_COOKIE}=; Path=/; Max-Age=0; Secure; SameSite=Strict"),
    )?;
    Ok(response)
}

fn require_admin(session: &AuthenticatedSession, request_id: &RequestId) -> Result<(), ApiError> {
    if matches!(session.subject, AuthenticatedSubject::PrimaryAdmin) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "PRIMARY_ADMIN_REQUIRED",
            message: "the primary administrator credential is required".to_owned(),
            request_id: request_id.0.clone(),
        })
    }
}

fn require_device<'a>(
    session: &'a AuthenticatedSession,
    request_id: &RequestId,
) -> Result<&'a str, ApiError> {
    match &session.subject {
        AuthenticatedSubject::Device { id, .. } => Ok(id),
        AuthenticatedSubject::PrimaryAdmin => Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "DEVICE_SESSION_REQUIRED",
            message: "a registered device session is required".to_owned(),
            request_id: request_id.0.clone(),
        }),
    }
}

fn require_admin_confirmation(
    headers: &HeaderMap,
    expected: &str,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    if headers
        .get(ADMIN_CONFIRM_HEADER)
        .and_then(|value| value.to_str().ok())
        == Some(expected)
    {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "DEVICE_ADMIN_CONFIRMATION_REQUIRED",
            message: format!("set {ADMIN_CONFIRM_HEADER} to {expected}"),
            request_id: request_id.0.clone(),
        })
    }
}

fn require_same_origin(headers: &HeaderMap, request_id: &RequestId) -> Result<(), ApiError> {
    if same_origin(headers) {
        Ok(())
    } else {
        Err(ApiError {
            status: StatusCode::FORBIDDEN,
            code: "SAME_ORIGIN_REQUIRED",
            message: "request origin must exactly match the TM HTTPS host".to_owned(),
            request_id: request_id.0.clone(),
        })
    }
}

fn same_origin(headers: &HeaderMap) -> bool {
    if headers.get_all(HOST).iter().count() != 1 || headers.get_all(ORIGIN).iter().count() != 1 {
        return false;
    }
    let Some(host) = headers.get(HOST).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Some(origin) = headers.get(ORIGIN).and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let Ok(url) = Url::parse(origin) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/"
        && url.query().is_none()
        && url.fragment().is_none()
        && url.host_str().is_some()
        && url
            .host_str()
            .map(|url_host| {
                let expected = url
                    .port()
                    .map_or_else(|| url_host.to_owned(), |port| format!("{url_host}:{port}"));
                expected.eq_ignore_ascii_case(host)
            })
            .unwrap_or(false)
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .as_bytes()
            .iter()
            .zip(right.as_bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn append_cookie(response: &mut Response, value: &str) -> Result<(), ApiError> {
    let header = HeaderValue::from_str(value).map_err(|_| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "DEVICE_COOKIE_FAILED",
        message: "device cookie could not be created".to_owned(),
        request_id: "cookie".to_owned(),
    })?;
    response.headers_mut().append(SET_COOKIE, header);
    Ok(())
}

fn parse_json<T>(
    payload: Result<Json<T>, JsonRejection>,
    request_id: &RequestId,
) -> Result<Json<T>, ApiError> {
    payload.map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_DEVICE_JSON",
        message: "device request body does not match the API contract".to_owned(),
        request_id: request_id.0.clone(),
    })
}

fn worker_error(request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "DEVICE_WORKER_FAILED",
        message: "device operation worker was unavailable".to_owned(),
        request_id: request_id.0.clone(),
    }
}

fn map_core_error(error: CoreError, request_id: &RequestId) -> ApiError {
    match error {
        CoreError::InvalidInput(message) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_DEVICE_REQUEST",
            message,
            request_id: request_id.0.clone(),
        },
        CoreError::NotFound { entity, id } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "DEVICE_NOT_FOUND",
            message: format!("{entity} not found: {id}"),
            request_id: request_id.0.clone(),
        },
        CoreError::Conflict(message) => ApiError {
            status: StatusCode::CONFLICT,
            code: "DEVICE_CONFLICT",
            message,
            request_id: request_id.0.clone(),
        },
        other => {
            tracing::error!(%other, request_id = %request_id.0, "device operation failed");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "DEVICE_OPERATION_FAILED",
                message: "device operation could not be completed".to_owned(),
                request_id: request_id.0.clone(),
            }
        }
    }
}
