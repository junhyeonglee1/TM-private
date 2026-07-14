use std::{
    env, fmt,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
};

pub mod openai;

use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header::HeaderName},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;
use tm_core::{HealthReport, TmCore};
use uuid::Uuid;

use crate::openai::{OpenAiClient, OpenAiConfig, OpenAiError, OpenAiProbeResult};

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
pub const LOCAL_PROFILE: &str = "local";
pub const CLOUD_BOOTSTRAP_PROFILE: &str = "cloud-bootstrap";
const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");
const AI_CONFIRM_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-ai-call");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerProfile {
    Local,
    CloudBootstrap,
}

impl ServerProfile {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            LOCAL_PROFILE => Ok(Self::Local),
            CLOUD_BOOTSTRAP_PROFILE => Ok(Self::CloudBootstrap),
            _ => Err(format!(
                "TM_SERVER_PROFILE must be {LOCAL_PROFILE} or {CLOUD_BOOTSTRAP_PROFILE}"
            )),
        }
    }
}

impl fmt::Display for ServerProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local => formatter.write_str(LOCAL_PROFILE),
            Self::CloudBootstrap => formatter.write_str(CLOUD_BOOTSTRAP_PROFILE),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub profile: ServerProfile,
    pub bind_addr: SocketAddr,
    pub home: PathBuf,
    pub openai: OpenAiConfig,
}

impl ServerConfig {
    pub fn from_env() -> Result<Self, String> {
        let profile =
            optional_env("TM_SERVER_PROFILE")?.unwrap_or_else(|| LOCAL_PROFILE.to_owned());
        let profile = ServerProfile::parse(&profile)?;

        let home = required_path_env("TM_SERVER_HOME")?;
        if !home.is_absolute() {
            return Err("TM_SERVER_HOME must be an absolute path".to_owned());
        }

        let (bind_addr, openai) = match profile {
            ServerProfile::Local => {
                let bind_addr = optional_env("TM_SERVER_BIND")?
                    .unwrap_or_else(|| DEFAULT_BIND_ADDR.to_owned())
                    .parse::<SocketAddr>()
                    .map_err(|error| format!("TM_SERVER_BIND is invalid: {error}"))?;
                validate_bind_addr(profile, bind_addr)?;
                (bind_addr, OpenAiConfig::from_env()?)
            }
            ServerProfile::CloudBootstrap => {
                require_railway_environment()?;
                reject_cloud_bootstrap_overrides()?;

                let volume_mount = required_path_env("RAILWAY_VOLUME_MOUNT_PATH")?;
                validate_cloud_home(&home, &volume_mount)?;

                let port = required_env("PORT")?
                    .parse::<u16>()
                    .map_err(|error| format!("PORT is invalid: {error}"))?;
                if port == 0 {
                    return Err("PORT must be between 1 and 65535".to_owned());
                }
                let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port));
                validate_bind_addr(profile, bind_addr)?;
                (bind_addr, OpenAiConfig::default())
            }
        };

        Ok(Self {
            profile,
            bind_addr,
            home,
            openai,
        })
    }
}

#[derive(Clone)]
struct AppState {
    core: TmCore,
    openai: OpenAiClient,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiStatus {
    provider: &'static str,
    configured: bool,
    model: String,
    api_base: String,
    response_storage: &'static str,
}

pub fn build_router(core: TmCore) -> Router {
    build_router_with_openai(core, OpenAiClient::disabled())
}

pub fn build_router_with_openai(core: TmCore, openai: OpenAiClient) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/v1/ai/status", get(ai_status))
        .route("/api/v1/ai/probe", post(ai_probe))
        .fallback(not_found)
        .with_state(AppState { core, openai })
        .layer(middleware::from_fn(assign_request_id))
}

pub fn build_cloud_bootstrap_router(core: TmCore) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .fallback(not_found)
        .with_state(AppState {
            core,
            openai: OpenAiClient::disabled(),
        })
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

async fn ai_status(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Json<ApiEnvelope<AiStatus>> {
    let config = state.openai.config();
    Json(ApiEnvelope {
        request_id: request_id.0,
        data: AiStatus {
            provider: "openai",
            configured: config.configured(),
            model: config.model().to_owned(),
            api_base: config.base_url().to_owned(),
            response_storage: "disabled",
        },
    })
}

async fn ai_probe(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
) -> Result<Json<ApiEnvelope<OpenAiProbeResult>>, ApiError> {
    if headers
        .get(&AI_CONFIRM_HEADER)
        .and_then(|value| value.to_str().ok())
        != Some("probe")
    {
        return Err(ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "AI_CALL_CONFIRMATION_REQUIRED",
            message: "set x-tm-confirm-ai-call to probe for this billable request".to_owned(),
            request_id: request_id.0,
        });
    }

    let error_request_id = request_id.0.clone();
    let result = state
        .openai
        .probe()
        .await
        .map_err(|error| openai_api_error(error, error_request_id))?;

    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: result,
    }))
}

fn validate_bind_addr(profile: ServerProfile, bind_addr: SocketAddr) -> Result<(), String> {
    match profile {
        ServerProfile::Local if !bind_addr.ip().is_loopback() => Err(
            "TM_SERVER_BIND must use a loopback address until server authentication is implemented"
                .to_owned(),
        ),
        ServerProfile::CloudBootstrap if !bind_addr.ip().is_unspecified() => Err(
            "cloud-bootstrap must listen on Railway PORT using an unspecified address".to_owned(),
        ),
        _ => Ok(()),
    }
}

fn validate_cloud_home(home: &Path, volume_mount: &Path) -> Result<(), String> {
    if !volume_mount.is_absolute() {
        return Err("RAILWAY_VOLUME_MOUNT_PATH must be an absolute path".to_owned());
    }
    if !home.starts_with(volume_mount) {
        return Err(
            "TM_SERVER_HOME must be inside RAILWAY_VOLUME_MOUNT_PATH in cloud-bootstrap".to_owned(),
        );
    }
    Ok(())
}

fn require_railway_environment() -> Result<(), String> {
    for name in [
        "RAILWAY_PROJECT_ID",
        "RAILWAY_ENVIRONMENT_ID",
        "RAILWAY_SERVICE_ID",
    ] {
        required_env(name)?;
    }
    Ok(())
}

fn reject_cloud_bootstrap_overrides() -> Result<(), String> {
    let forbidden = [
        "TM_SERVER_BIND",
        "OPENAI_API_KEY",
        "TM_OPENAI_MODEL",
        "TM_OPENAI_BASE_URL",
        "TM_OPENAI_TIMEOUT_SECS",
    ];
    for name in forbidden {
        if optional_env(name)?.is_some() {
            return Err(format!(
                "{name} must not be set in cloud-bootstrap; AI and custom bind settings are disabled"
            ));
        }
    }
    Ok(())
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value)),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(format!("{name} must contain valid Unicode")),
    }
}

fn required_env(name: &str) -> Result<String, String> {
    optional_env(name)?.ok_or_else(|| format!("{name} must be set in cloud-bootstrap"))
}

fn required_path_env(name: &str) -> Result<PathBuf, String> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} must be set to an absolute path"))
}

fn openai_api_error(error: OpenAiError, request_id: String) -> ApiError {
    tracing::warn!(?error, %request_id, "OpenAI probe failed");
    match error {
        OpenAiError::NotConfigured => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "OPENAI_NOT_CONFIGURED",
            message: "OPENAI_API_KEY is not configured on the server".to_owned(),
            request_id,
        },
        OpenAiError::Authentication { .. } => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "OPENAI_AUTHENTICATION_FAILED",
            message: "OpenAI rejected the server credential".to_owned(),
            request_id,
        },
        OpenAiError::RateLimited { .. } => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "OPENAI_RATE_LIMITED",
            message: "OpenAI temporarily rate-limited the connectivity check".to_owned(),
            request_id,
        },
        OpenAiError::RequestRejected { .. } => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "OPENAI_REQUEST_REJECTED",
            message: "OpenAI rejected the connectivity check configuration".to_owned(),
            request_id,
        },
        OpenAiError::InvalidConfiguration => ApiError {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "OPENAI_CONFIGURATION_INVALID",
            message: "the OpenAI server configuration is invalid".to_owned(),
            request_id,
        },
        OpenAiError::Transport | OpenAiError::UpstreamUnavailable { .. } => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "OPENAI_UNAVAILABLE",
            message: "the OpenAI service could not be reached".to_owned(),
            request_id,
        },
        OpenAiError::InvalidResponse => ApiError {
            status: StatusCode::BAD_GATEWAY,
            code: "OPENAI_RESPONSE_INVALID",
            message: "OpenAI returned an unexpected response".to_owned(),
            request_id,
        },
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
    use std::{
        fs,
        path::Path,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        extract::State,
        http::{HeaderMap, HeaderValue, Request, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use serde_json::{Value, json};
    use tempfile::Builder;
    use tm_core::{DEFAULT_TM_HOME, TmCore, TmHome};
    use tower::ServiceExt;

    use super::{
        ServerProfile, build_cloud_bootstrap_router, build_router, build_router_with_openai,
        validate_bind_addr, validate_cloud_home,
    };
    use crate::openai::{OpenAiClient, OpenAiConfig};

    #[derive(Clone, Default)]
    struct MockOpenAiCapture {
        authorization: Arc<Mutex<Option<String>>>,
        payload: Arc<Mutex<Option<Value>>>,
    }

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

    async fn mock_openai_response(
        State(capture): State<MockOpenAiCapture>,
        headers: HeaderMap,
        Json(payload): Json<Value>,
    ) -> impl IntoResponse {
        *capture
            .authorization
            .lock()
            .expect("lock mock authorization") = headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        *capture.payload.lock().expect("lock mock payload") = Some(payload);

        let mut response_headers = HeaderMap::new();
        response_headers.insert("x-request-id", HeaderValue::from_static("openai-request-1"));
        (
            response_headers,
            Json(json!({
                "id": "resp_test_1",
                "status": "completed",
                "model": "gpt-test-1",
                "output": [{
                    "type": "message",
                    "content": [{
                        "type": "output_text",
                        "text": "TM_OPENAI_OK"
                    }]
                }],
                "usage": {
                    "input_tokens": 12,
                    "output_tokens": 4,
                    "total_tokens": 16
                }
            })),
        )
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

    #[tokio::test]
    async fn ai_status_is_safe_when_no_key_is_configured() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/ai/status")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call AI status route");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["provider"], "openai");
        assert_eq!(body["data"]["configured"], false);
        assert_eq!(body["data"]["responseStorage"], "disabled");
        assert!(body.to_string().find("apiKey").is_none());
    }

    #[tokio::test]
    async fn ai_probe_requires_a_server_side_key() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/probe")
                    .header("x-tm-confirm-ai-call", "probe")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call AI probe route");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "OPENAI_NOT_CONFIGURED");
    }

    #[tokio::test]
    async fn ai_probe_uses_bearer_auth_disables_storage_and_reports_usage() {
        let capture = MockOpenAiCapture::default();
        let mock_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock OpenAI server");
        let mock_address = mock_listener.local_addr().expect("read mock address");
        let mock_router = Router::new()
            .route("/v1/responses", post(mock_openai_response))
            .with_state(capture.clone());
        let mock_server = tokio::spawn(async move {
            axum::serve(mock_listener, mock_router)
                .await
                .expect("serve mock OpenAI response");
        });

        let config = OpenAiConfig::for_test(
            Some("test-api-key"),
            "gpt-test-1",
            &format!("http://{mock_address}/v1"),
            Duration::from_secs(5),
        )
        .expect("build mock OpenAI config");
        let client = OpenAiClient::new(config).expect("build OpenAI client");
        let (_temporary, core) = test_core();
        let response = build_router_with_openai(core, client)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/probe")
                    .header("x-tm-confirm-ai-call", "probe")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call configured AI probe route");

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_json(response).await;
        assert_eq!(body["data"]["responseId"], "resp_test_1");
        assert_eq!(body["data"]["upstreamRequestId"], "openai-request-1");
        assert_eq!(body["data"]["outputText"], "TM_OPENAI_OK");
        assert_eq!(body["data"]["matchedExpectedText"], true);
        assert_eq!(body["data"]["usage"]["totalTokens"], 16);
        assert_eq!(body["data"]["stored"], false);

        assert_eq!(
            capture
                .authorization
                .lock()
                .expect("lock captured authorization")
                .as_deref(),
            Some("Bearer test-api-key")
        );
        let captured_payload = capture.payload.lock().expect("lock captured payload");
        let captured_payload = captured_payload.as_ref().expect("captured payload");
        assert_eq!(captured_payload["model"], "gpt-test-1");
        assert_eq!(captured_payload["store"], false);
        assert_eq!(captured_payload["reasoning"]["effort"], "none");

        mock_server.abort();
    }

    #[test]
    fn openai_config_debug_output_never_contains_the_secret() {
        let config = OpenAiConfig::for_test(
            Some("top-secret-test-value"),
            "gpt-test-1",
            "https://api.openai.com/v1",
            Duration::from_secs(5),
        )
        .expect("build OpenAI config");

        let debug_output = format!("{config:?}");
        assert!(debug_output.contains("configured: true"));
        assert!(!debug_output.contains("top-secret-test-value"));
    }

    #[test]
    fn openai_config_rejects_non_official_https_hosts() {
        let result = OpenAiConfig::for_test(
            Some("test-api-key"),
            "gpt-test-1",
            "https://example.com/v1",
            Duration::from_secs(5),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ai_probe_requires_explicit_billable_call_confirmation() {
        let (_temporary, core) = test_core();
        let response = build_router(core)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/ai/probe")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("call unconfirmed AI probe route");

        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "AI_CALL_CONFIRMATION_REQUIRED");
    }

    #[tokio::test]
    async fn cloud_bootstrap_exposes_health_but_not_ai_routes() {
        let (_temporary, core) = test_core();
        let router = build_cloud_bootstrap_router(core);

        let health_response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("build health request"),
            )
            .await
            .expect("call cloud health route");
        assert_eq!(health_response.status(), StatusCode::OK);

        for (method, path) in [("GET", "/api/v1/ai/status"), ("POST", "/api/v1/ai/probe")] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .body(Body::empty())
                        .expect("build disabled route request"),
                )
                .await
                .expect("call disabled cloud route");
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let body = response_json(response).await;
            assert_eq!(body["error"]["code"], "NOT_FOUND");
        }
    }

    #[test]
    fn public_server_bind_addresses_are_rejected_before_authentication_exists() {
        assert!(
            validate_bind_addr(
                ServerProfile::Local,
                "127.0.0.1:8787".parse().expect("parse loopback")
            )
            .is_ok()
        );
        assert!(
            validate_bind_addr(
                ServerProfile::Local,
                "0.0.0.0:8787".parse().expect("parse public bind")
            )
            .is_err()
        );
        assert!(
            validate_bind_addr(
                ServerProfile::CloudBootstrap,
                "0.0.0.0:8787".parse().expect("parse Railway bind")
            )
            .is_ok()
        );
        assert!(
            validate_bind_addr(
                ServerProfile::CloudBootstrap,
                "127.0.0.1:8787".parse().expect("parse invalid cloud bind")
            )
            .is_err()
        );
    }

    #[test]
    fn cloud_home_must_be_on_the_railway_volume() {
        let volume = if cfg!(windows) {
            Path::new(r"C:\railway\tm")
        } else {
            Path::new("/var/lib/tm")
        };
        let nested_home = volume.join("app");

        assert!(validate_cloud_home(volume, volume).is_ok());
        assert!(validate_cloud_home(&nested_home, volume).is_ok());
        assert!(
            validate_cloud_home(Path::new("relative-home"), Path::new("relative-volume")).is_err()
        );
    }
}
