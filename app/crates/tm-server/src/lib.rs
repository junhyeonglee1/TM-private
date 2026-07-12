use std::{env, net::SocketAddr, path::PathBuf};

use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header::HeaderName},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Serialize;
use tm_core::{HealthReport, TmCore};
use uuid::Uuid;

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub bind_addr: SocketAddr,
    pub home: PathBuf,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let home = env::var_os("TM_SERVER_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| "TM_SERVER_HOME must be set to an absolute path".to_owned())?;
        if !home.is_absolute() {
            return Err("TM_SERVER_HOME must be an absolute path".to_owned());
        }

        let bind_addr = env::var("TM_SERVER_BIND")
            .unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned())
            .parse::<SocketAddr>()
            .map_err(|error| format!("TM_SERVER_BIND is invalid: {error}"))?;

        Ok(Self { bind_addr, home })
    }
}

#[derive(Clone)]
struct AppState {
    core: TmCore,
}

#[derive(Debug, Clone)]
struct RequestId(String);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiEnvelope<T> {
    request_id: String,
    data: T,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    request_id: String,
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    request_id: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorEnvelope {
            request_id: self.request_id,
            error: ErrorBody {
                code: self.code,
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Liveness {
    status: &'static str,
    service: &'static str,
    version: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Readiness {
    status: &'static str,
    schema_version: i64,
    sqlite_version: String,
    journal_mode: String,
    checked_at: String,
}

pub fn build_router(core: TmCore) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .fallback(not_found)
        .with_state(AppState { core })
        .layer(middleware::from_fn(assign_request_id))
}

async fn healthz(Extension(request_id): Extension<RequestId>) -> impl IntoResponse {
    Json(ApiEnvelope {
        request_id: request_id.0,
        data: Liveness {
            status: "ok",
            service: "tm-server",
            version: env!("CARGO_PKG_VERSION"),
        },
    })
}

async fn readyz(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ApiEnvelope<Readiness>>, ApiError> {
    let error_request_id = request_id.0.clone();
    let health = tokio::task::spawn_blocking(move || state.core.health())
        .await
        .map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "READINESS_WORKER_FAILED",
            message: "database readiness worker failed".to_owned(),
            request_id: error_request_id.clone(),
        })?
        .map_err(|error| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "DATABASE_UNAVAILABLE",
            message: error.to_string(),
            request_id: error_request_id.clone(),
        })?;

    if !health.ok {
        return Err(readiness_error(health, error_request_id));
    }

    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: Readiness {
            status: "ready",
            schema_version: health.schema_version,
            sqlite_version: health.sqlite_version,
            journal_mode: health.journal_mode,
            checked_at: health.checked_at,
        },
    }))
}

fn readiness_error(health: HealthReport, request_id: String) -> ApiError {
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "DATABASE_NOT_READY",
        message: format!(
            "database failed readiness checks: schema={}, journal={}, integrity={}",
            health.schema_version, health.journal_mode, health.integrity_check
        ),
        request_id,
    }
}

async fn not_found(Extension(request_id): Extension<RequestId>) -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "NOT_FOUND",
        message: "route not found".to_owned(),
        request_id: request_id.0,
    }
}

async fn assign_request_id(mut request: Request<Body>, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_request_id(value))
        .map_or_else(|| Uuid::now_v7().to_string(), ToOwned::to_owned);

    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tempfile::Builder;
    use tm_core::{DEFAULT_TM_HOME, TmCore, TmHome};
    use tower::ServiceExt;

    use super::build_router;

    fn test_core() -> (tempfile::TempDir, TmCore) {
        let test_runs = Path::new(DEFAULT_TM_HOME).join("dist").join("test-runs");
        fs::create_dir_all(&test_runs).expect("create test-runs directory");
        let temporary = Builder::new()
            .prefix("tm-server-")
            .tempdir_in(test_runs)
            .expect("create temporary TM home");
        let core = TmCore::open(TmHome::new(temporary.path())).expect("open temporary TM core");
        (temporary, core)
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read response body");
        serde_json::from_slice(&body).expect("parse response JSON")
    }

    #[tokio::test]
    async fn liveness_is_process_only_and_returns_request_id() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call liveness route");

        assert_eq!(response.status(), StatusCode::OK);
        let header_request_id = response
            .headers()
            .get("x-request-id")
            .expect("request ID header")
            .to_str()
            .expect("request ID text")
            .to_owned();
        let body = response_json(response).await;
        assert_eq!(body["requestId"], header_request_id);
        assert_eq!(body["data"]["status"], "ok");
        assert_eq!(body["data"]["service"], "tm-server");
    }

    #[tokio::test]
    async fn readiness_checks_the_temporary_database() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call readiness route");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["status"], "ready");
        assert_eq!(body["data"]["schemaVersion"], 3);
        assert_eq!(body["data"]["journalMode"], "wal");
    }

    #[tokio::test]
    async fn request_id_is_echoed_when_valid() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .header("x-request-id", "tm-client-123")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call liveness route");

        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .expect("request ID header"),
            "tm-client-123"
        );
        let body = response_json(response).await;
        assert_eq!(body["requestId"], "tm-client-123");
    }

    #[tokio::test]
    async fn unknown_routes_return_structured_errors() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .uri("/missing")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call missing route");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let header_request_id = response
            .headers()
            .get("x-request-id")
            .expect("request ID header")
            .to_str()
            .expect("request ID text")
            .to_owned();
        let body = response_json(response).await;
        assert_eq!(body["requestId"], header_request_id);
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }
}
