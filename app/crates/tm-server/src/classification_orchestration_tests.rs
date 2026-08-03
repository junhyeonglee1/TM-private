use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderValue, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, Utc};
use chrono_tz::Asia::Seoul;
use serde_json::{Value, json};
use tempfile::TempDir;
use tm_core::{
    AiBudgetPolicy, AiTokenUsage, ClaimExpenseAiClassificationBatchInput,
    ExpenseAiClassificationBatchStatus, ExpenseAiClassificationBindingInput,
    ExpenseAiClassificationGroupInput, ExpenseAiClassificationSuggestion, ExpenseCategory,
    StageExpenseAiClassificationBatchInput, TmCore, TmHome,
};
use tokio::{sync::Notify, task::JoinHandle};
use tower::ServiceExt;

use super::{
    ExpenseRolloutConfig, IncidentMode, RailwayUsageClient, StockConfig,
    auth::{AuthConfig, TOKEN_PREFIX, sha256_hex},
    build_cloud_authenticated_router_with_feature_controls_costs_stock_and_expenses, expense_api,
    expense_classification::{
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD, EXPENSE_CLASSIFICATION_MODEL,
        EXPENSE_CLASSIFICATION_MONTHLY_ATTEMPT_LIMIT,
        EXPENSE_CLASSIFICATION_MONTHLY_HARD_LIMIT_MICROUSD, EXPENSE_CLASSIFICATION_OPERATION,
        EXPENSE_CLASSIFICATION_PROMPT_VERSION,
    },
    openai::{OpenAiClient, OpenAiConfig},
};

const TEST_AUTH_SECRET: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[derive(Clone, Copy)]
enum ProviderMode {
    Success,
    BlockedSuccess,
    Unavailable,
}

#[derive(Clone)]
struct MockClassificationProvider {
    calls: Arc<AtomicUsize>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    mode: ProviderMode,
}

impl MockClassificationProvider {
    fn new(mode: ProviderMode) -> Self {
        Self {
            calls: Arc::new(AtomicUsize::new(0)),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            mode,
        }
    }
}

async fn mock_classification_provider(
    State(provider): State<MockClassificationProvider>,
    Json(payload): Json<Value>,
) -> Response {
    provider.calls.fetch_add(1, Ordering::SeqCst);
    provider.entered.notify_one();
    if matches!(provider.mode, ProviderMode::BlockedSuccess) {
        provider.release.notified().await;
    }
    if matches!(provider.mode, ProviderMode::Unavailable) {
        let mut response = (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": {"message": "temporarily unavailable"}})),
        )
            .into_response();
        response.headers_mut().insert(
            "x-request-id",
            HeaderValue::from_static("openai-classification-unavailable"),
        );
        return response;
    }

    let input: Value = serde_json::from_str(
        payload["input"][0]["content"]
            .as_str()
            .expect("classification prompt input"),
    )
    .expect("parse classification prompt input");
    let items = input["items"]
        .as_array()
        .expect("classification prompt items")
        .iter()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "itemId": item["itemId"],
                "category": if index == 0 { "cafe" } else { "food" },
                "confidence": if index == 0 { 95 } else { 80 },
            })
        })
        .collect::<Vec<_>>();
    let output_text =
        serde_json::to_string(&json!({"items": items})).expect("serialize provider result");
    let mut response = Json(json!({
        "id": "resp_expense_classification_orchestration",
        "status": "completed",
        "model": EXPENSE_CLASSIFICATION_MODEL,
        "output": [{
            "type": "message",
            "content": [{"type": "output_text", "text": output_text}]
        }],
        "usage": {
            "input_tokens": 100,
            "input_tokens_details": {"cached_tokens": 0},
            "output_tokens": 20,
            "total_tokens": 120
        }
    }))
    .into_response();
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_static("openai-classification-success"),
    );
    response
}

fn test_auth_token() -> String {
    format!("{TOKEN_PREFIX}{TEST_AUTH_SECRET}")
}

fn test_auth_config() -> AuthConfig {
    AuthConfig::for_test(&test_auth_token(), Utc::now() + ChronoDuration::minutes(5))
}

fn test_core() -> (TempDir, TmCore) {
    let temporary = tempfile::Builder::new()
        .prefix("tm-classification-orchestration-")
        .tempdir()
        .expect("create classification orchestration test home");
    let core = TmCore::open(TmHome::new(temporary.path()))
        .expect("open classification orchestration test core");
    (temporary, core)
}

async fn test_router(
    core: TmCore,
    provider: MockClassificationProvider,
) -> (Router, AiBudgetPolicy, JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind classification provider");
    let address = listener.local_addr().expect("read provider address");
    let provider_router = Router::new()
        .route("/v1/responses", post(mock_classification_provider))
        .with_state(provider);
    let provider_server = tokio::spawn(async move {
        axum::serve(listener, provider_router)
            .await
            .expect("serve classification provider");
    });
    let config = OpenAiConfig::for_test(
        Some("test-api-key"),
        EXPENSE_CLASSIFICATION_MODEL,
        &format!("http://{address}/v1"),
        Duration::from_secs(5),
    )
    .expect("build classification provider config");
    let policy = config.budget_policy();
    let openai = OpenAiClient::new(config).expect("build classification provider client");
    let router = build_cloud_authenticated_router_with_feature_controls_costs_stock_and_expenses(
        core,
        test_auth_config(),
        openai,
        IncidentMode::Normal,
        true,
        true,
        RailwayUsageClient::disabled(),
        StockConfig::default(),
        false,
        true,
        ExpenseRolloutConfig::local_enabled(),
    );
    (router, policy, provider_server)
}

async fn response_json(response: Response) -> Value {
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&body).expect("parse response body")
}

fn classification_request(idempotency_key: &str, month: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/expenses/classifications:run")
        .header("authorization", format!("Bearer {}", test_auth_token()))
        .header("content-type", "application/json")
        .header("x-tm-confirm-ai-call", "expense-classification")
        .header("x-tm-confirm-mutation", "expense-classification-run")
        .header("idempotency-key", idempotency_key)
        .header("if-none-match", "*")
        .body(Body::from(json!({"month": month}).to_string()))
        .expect("build classification request")
}

fn operation_request_id(idempotency_key: &str) -> String {
    format!("expense-classification:{}", sha256_hex(idempotency_key))
}

async fn import_classification_candidates(router: &Router) {
    let import = expense_api::sign_test_expense_import_value(json!({
        "adapter": "kb_card_usage_v1",
        "sourceKind": "card",
        "sourceFingerprint": "11".repeat(32),
        "fileSha256": "22".repeat(32),
        "normalizedSha256": "33".repeat(32),
        "coverageStart": "2026-08-01",
        "coverageEnd": "2026-08-31",
        "rejectedCount": 0,
        "rows": [
            {
                "stableKey": "classification-row-one",
                "rowSha256": "01".repeat(32),
                "sourceRowNumber": 1,
                "occurredAt": "2026-08-03T09:00:00+09:00",
                "postedDate": "2026-08-03",
                "direction": "debit",
                "amountMinor": 4_500,
                "currency": "KRW",
                "kind": "purchase",
                "categoryHint": "cafe",
                "merchant": "First Cafe",
                "counterparty": null,
                "memo": null,
                "paymentMethodFingerprint": "aa".repeat(32),
                "externalReferenceFingerprint": null
            },
            {
                "stableKey": "classification-row-two",
                "rowSha256": "02".repeat(32),
                "sourceRowNumber": 2,
                "occurredAt": "2026-08-04T10:30:00+09:00",
                "postedDate": "2026-08-04",
                "direction": "debit",
                "amountMinor": 8_900,
                "currency": "KRW",
                "kind": "purchase",
                "categoryHint": "food",
                "merchant": "Second Restaurant",
                "counterparty": null,
                "memo": null,
                "paymentMethodFingerprint": "bb".repeat(32),
                "externalReferenceFingerprint": null
            }
        ]
    }));
    let preview = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/expenses/imports/preview")
                .header("authorization", format!("Bearer {}", test_auth_token()))
                .header("content-type", "application/json")
                .header("idempotency-key", "classification-orchestration-preview")
                .header("if-none-match", "*")
                .header("x-tm-confirm-mutation", "expense-import-preview")
                .body(Body::from(import.to_string()))
                .expect("build classification fixture preview"),
        )
        .await
        .expect("preview classification fixture");
    assert_eq!(preview.status(), StatusCode::OK);
    let preview = response_json(preview).await;
    let session_id = preview["data"]["sessionId"]
        .as_str()
        .expect("classification fixture preview session");
    let mut committed_import = import;
    committed_import
        .as_object_mut()
        .expect("classification import object")
        .insert(
            "previewSessionId".to_owned(),
            Value::String(session_id.to_owned()),
        );
    let imported = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/expenses/imports")
                .header("authorization", format!("Bearer {}", test_auth_token()))
                .header("content-type", "application/json")
                .header("idempotency-key", "classification-orchestration-import")
                .header("if-none-match", "*")
                .header("x-tm-confirm-mutation", "expense-import")
                .body(Body::from(committed_import.to_string()))
                .expect("build classification fixture import"),
        )
        .await
        .expect("import classification fixture");
    assert_eq!(imported.status(), StatusCode::CREATED);
}

fn claim_input(
    core: &TmCore,
    idempotency_key: &str,
    target_month_start: NaiveDate,
) -> ClaimExpenseAiClassificationBatchInput {
    let groups = core
        .list_expense_ai_classification_candidates(target_month_start, 250)
        .expect("list classification candidates")
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| ExpenseAiClassificationGroupInput {
            item_id: format!("item-{:03}", index + 1),
            privacy_skipped: false,
            bindings: vec![ExpenseAiClassificationBindingInput {
                event_id: candidate.event_id,
                event_version: candidate.event_version,
                review_id: candidate.review_id,
                review_version: candidate.review_version,
            }],
        })
        .collect::<Vec<_>>();
    assert_eq!(groups.len(), 2);
    ClaimExpenseAiClassificationBatchInput {
        request_id: operation_request_id(idempotency_key),
        quota_month_start: Utc::now()
            .with_timezone(&Seoul)
            .date_naive()
            .with_day(1)
            .expect("valid quota month"),
        target_month_start,
        input_sha256: "ab".repeat(32),
        prompt_version: EXPENSE_CLASSIFICATION_PROMPT_VERSION.to_owned(),
        model: EXPENSE_CLASSIFICATION_MODEL.to_owned(),
        max_attempts: EXPENSE_CLASSIFICATION_MONTHLY_ATTEMPT_LIMIT,
        groups,
    }
}

#[tokio::test]
async fn concurrent_same_key_calls_provider_and_budget_once_and_other_month_conflicts() {
    let provider = MockClassificationProvider::new(ProviderMode::BlockedSuccess);
    let (_temporary, core) = test_core();
    let (router, _policy, provider_server) = test_router(core.clone(), provider.clone()).await;
    import_classification_candidates(&router).await;

    let idempotency_key = "classification-concurrent-same-key";
    let first_router = router.clone();
    let first = tokio::spawn(async move {
        first_router
            .oneshot(classification_request(idempotency_key, "2026-08"))
            .await
            .expect("call first concurrent classification request")
    });
    tokio::time::timeout(Duration::from_secs(5), provider.entered.notified())
        .await
        .expect("first request reached classification provider");
    let concurrent = router
        .clone()
        .oneshot(classification_request(idempotency_key, "2026-08"))
        .await
        .expect("call second concurrent classification request");
    assert_eq!(concurrent.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(concurrent).await["error"]["code"],
        "EXPENSE_CLASSIFICATION_ALREADY_STARTED"
    );
    let in_flight_request_id = operation_request_id(idempotency_key);
    assert_eq!(
        core.get_ai_budget_reservation(&in_flight_request_id)
            .expect("read in-flight budget reservation")
            .expect("in-flight budget reservation")
            .reserved_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );
    assert!(
        core.get_ai_budget_settlement(&in_flight_request_id)
            .expect("read in-flight budget settlement")
            .is_none()
    );
    provider.release.notify_one();
    let completed = first.await.expect("join first classification request");
    assert_eq!(completed.status(), StatusCode::OK);
    assert_eq!(response_json(completed).await["data"]["status"], "applied");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    let request_id = operation_request_id(idempotency_key);
    let reservation = core
        .get_ai_budget_reservation(&request_id)
        .expect("read concurrent budget reservation")
        .expect("concurrent budget reservation");
    assert_eq!(
        reservation.reserved_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );
    let settlement = core
        .get_ai_budget_settlement(&request_id)
        .expect("read concurrent budget settlement")
        .expect("concurrent budget settlement");
    assert_eq!(settlement.actual_cost_microusd, 45);
    assert_eq!(settlement.outcome, "succeeded");
    assert_eq!(
        core.ai_operation_budget_status(
            EXPENSE_CLASSIFICATION_OPERATION,
            EXPENSE_CLASSIFICATION_MONTHLY_HARD_LIMIT_MICROUSD,
        )
        .expect("read concurrent classification budget")
        .committed_microusd,
        45
    );

    let other_month = router
        .oneshot(classification_request(idempotency_key, "2026-07"))
        .await
        .expect("call same key for another month");
    assert_eq!(other_month.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(other_month).await["error"]["code"],
        "EXPENSE_CLASSIFICATION_IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    provider_server.abort();
}

#[tokio::test]
async fn staged_batch_recovery_applies_without_provider_recall_and_settles_exact_cost() {
    let provider = MockClassificationProvider::new(ProviderMode::Success);
    let (_temporary, core) = test_core();
    let (router, policy, provider_server) = test_router(core.clone(), provider.clone()).await;
    import_classification_candidates(&router).await;

    let idempotency_key = "classification-staged-recovery";
    let target_month_start = NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid target month");
    let claim = claim_input(&core, idempotency_key, target_month_start);
    let claimed = core
        .claim_expense_ai_classification_batch(claim.clone())
        .expect("claim staged recovery batch");
    assert_eq!(claimed.status, ExpenseAiClassificationBatchStatus::Claimed);
    let reservation = core
        .reserve_ai_budget_with_operation_limit(
            &claim.request_id,
            "openai",
            EXPENSE_CLASSIFICATION_MODEL,
            EXPENSE_CLASSIFICATION_OPERATION,
            EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD,
            policy,
            EXPENSE_CLASSIFICATION_MONTHLY_HARD_LIMIT_MICROUSD,
        )
        .expect("reserve staged recovery budget");
    let usage = AiTokenUsage {
        input_tokens: 100,
        cached_input_tokens: 0,
        output_tokens: 20,
        total_tokens: 120,
    };
    let staged = core
        .stage_expense_ai_classification_batch(StageExpenseAiClassificationBatchInput {
            request_id: claim.request_id.clone(),
            suggestions: vec![
                ExpenseAiClassificationSuggestion {
                    item_id: "item-001".to_owned(),
                    category: ExpenseCategory::Cafe,
                    confidence: 95,
                },
                ExpenseAiClassificationSuggestion {
                    item_id: "item-002".to_owned(),
                    category: ExpenseCategory::Food,
                    confidence: 80,
                },
            ],
            usage,
            cost_microusd: 45,
            latency_ms: 25,
        })
        .expect("stage durable classification result");
    assert_eq!(staged.status, ExpenseAiClassificationBatchStatus::Staged);
    assert!(
        core.get_ai_budget_settlement(&reservation.request_id)
            .expect("read pre-recovery settlement")
            .is_none()
    );

    let recovered = router
        .oneshot(classification_request(idempotency_key, "2026-08"))
        .await
        .expect("recover staged classification result");
    assert_eq!(recovered.status(), StatusCode::OK);
    assert_eq!(
        recovered
            .headers()
            .get("x-tm-idempotency-replayed")
            .and_then(|value| value.to_str().ok()),
        Some("true")
    );
    let recovered = response_json(recovered).await;
    assert_eq!(recovered["data"]["status"], "applied");
    assert_eq!(recovered["data"]["cached"], true);
    assert_eq!(recovered["data"]["costMicrousd"], 45);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    let applied = core
        .get_expense_ai_classification_batch(&claim.request_id)
        .expect("read recovered classification batch")
        .expect("recovered classification batch");
    assert_eq!(applied.status, ExpenseAiClassificationBatchStatus::Applied);
    assert_eq!(applied.usage, usage);
    let settlement = core
        .get_ai_budget_settlement(&reservation.request_id)
        .expect("read recovered budget settlement")
        .expect("recovered budget settlement");
    assert_eq!(settlement.actual_cost_microusd, 45);
    assert_eq!(settlement.outcome, "succeeded");
    assert_eq!(
        core.ai_operation_budget_status(
            EXPENSE_CLASSIFICATION_OPERATION,
            EXPENSE_CLASSIFICATION_MONTHLY_HARD_LIMIT_MICROUSD,
        )
        .expect("read recovered classification budget")
        .committed_microusd,
        45
    );
    provider_server.abort();
}

#[tokio::test]
async fn ambiguous_provider_failure_is_terminal_fully_reserved_and_never_auto_retried() {
    let provider = MockClassificationProvider::new(ProviderMode::Unavailable);
    let (_temporary, core) = test_core();
    let (router, _policy, provider_server) = test_router(core.clone(), provider.clone()).await;
    import_classification_candidates(&router).await;

    let idempotency_key = "classification-ambiguous-failure";
    let failed = router
        .clone()
        .oneshot(classification_request(idempotency_key, "2026-08"))
        .await
        .expect("call unavailable classification provider");
    assert_eq!(failed.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response_json(failed).await["error"]["code"],
        "EXPENSE_CLASSIFICATION_OPENAI_UNAVAILABLE"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    let request_id = operation_request_id(idempotency_key);
    let batch = core
        .get_expense_ai_classification_batch(&request_id)
        .expect("read failed classification batch")
        .expect("failed classification batch");
    assert_eq!(batch.status, ExpenseAiClassificationBatchStatus::Failed);
    assert_eq!(batch.failure_code.as_deref(), Some("upstream_unavailable"));
    let reservation = core
        .get_ai_budget_reservation(&request_id)
        .expect("read failed budget reservation")
        .expect("failed budget reservation");
    assert_eq!(
        reservation.reserved_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );
    let settlement = core
        .get_ai_budget_settlement(&request_id)
        .expect("read failed budget settlement")
        .expect("failed budget settlement");
    assert_eq!(
        settlement.actual_cost_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );
    assert_eq!(settlement.outcome, "upstream_cost_estimate");
    assert_eq!(
        core.ai_operation_budget_status(
            EXPENSE_CLASSIFICATION_OPERATION,
            EXPENSE_CLASSIFICATION_MONTHLY_HARD_LIMIT_MICROUSD,
        )
        .expect("read failed classification budget")
        .committed_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );

    let replay = router
        .oneshot(classification_request(idempotency_key, "2026-08"))
        .await
        .expect("replay failed classification request");
    assert_eq!(replay.status(), StatusCode::CONFLICT);
    assert_eq!(
        response_json(replay).await["error"]["code"],
        "EXPENSE_CLASSIFICATION_PREVIOUSLY_FAILED"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        core.get_ai_budget_settlement(&request_id)
            .expect("read replayed failed settlement")
            .expect("replayed failed settlement")
            .actual_cost_microusd,
        EXPENSE_CLASSIFICATION_MAXIMUM_COST_MICROUSD
    );
    provider_server.abort();
}
