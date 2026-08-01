use axum::{
    Extension, Json, Router,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use tm_core::{
    ConfirmRecurringPaidInput, CreateRecurringExpenseInput, EncryptedExpenseText,
    Error as CoreError, ExpenseCategory, ExpenseEventKind, ExpenseImportPreview,
    ExpenseImportPreviewInput, ExpenseMonthSummary, ExpenseMutationCommand, ExpenseMutationRequest,
    ExpenseReview, ExpenseReviewFilter, ExpenseReviewPage, ExpenseReviewStatus, ExpenseSourceKind,
    ExpenseSourceStatus, ExpenseTransaction, ExpenseTransactionFilter, ExpenseTransactionPage,
    MatchRecurringExpenseInput, NormalizedExpenseImport, NormalizedExpenseRow,
    OverrideExpenseTransactionInput, RecurringAmountKind, RecurringDueRule, RecurringExpenseItem,
    RecurringExpenseOccurrence, RecurringExpenseStatus, ResolveExpenseReviewInput, TmCore,
    UpdateExpenseSourceStatusInput, UpdateRecurringExpenseInput, expense_text_aad,
};
use zeroize::Zeroizing;

use super::{
    ApiEnvelope, ApiError, AppState, AuthenticatedSession, AuthenticatedSubject, RequestId,
    expense_crypto::{EncryptedExpenseValue, ExpenseCrypto, ExpenseCryptoError},
};

pub(super) const MAX_EXPENSE_IMPORT_BODY_BYTES: usize = 8 * 1024 * 1024;
pub(super) const MAX_EXPENSE_PREVIEW_BODY_BYTES: usize = MAX_EXPENSE_IMPORT_BODY_BYTES;
pub(super) const MAX_EXPENSE_MUTATION_BODY_BYTES: usize = 64 * 1024;
const DEFAULT_PAGE_LIMIT: u32 = 50;
const MAX_PAGE_LIMIT: u32 = 100;
const MAX_CURSOR_BYTES: usize = 512;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 128;
const MAX_RECURRING_NAME_CHARS: usize = 120;
const MAX_VENDOR_CHARS: usize = 200;
const MAX_FINANCIAL_TEXT_CHARS: usize = 500;

const IDEMPOTENCY_KEY_HEADER: HeaderName = HeaderName::from_static("idempotency-key");
const MUTATION_CONFIRM_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-mutation");
const IDEMPOTENCY_REPLAYED_HEADER: HeaderName =
    HeaderName::from_static("x-tm-idempotency-replayed");

const CONFIRM_EXPENSE_IMPORT: &str = "expense-import";
const CONFIRM_EXPENSE_IMPORT_PREVIEW: &str = "expense-import-preview";
const CONFIRM_EXPENSE_SOURCE_UPDATE: &str = "expense-source-update";
const CONFIRM_EXPENSE_TRANSACTION_OVERRIDE: &str = "expense-transaction-override";
const CONFIRM_REVIEW_RESOLVE: &str = "expense-review-resolve";
const CONFIRM_RECURRING_CREATE: &str = "recurring-expense-create";
const CONFIRM_RECURRING_UPDATE: &str = "recurring-expense-update";
const CONFIRM_RECURRING_DELETE: &str = "recurring-expense-delete";
const CONFIRM_RECURRING_PAID: &str = "recurring-expense-confirm-paid";
const CONFIRM_RECURRING_MATCH: &str = "recurring-expense-match";

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/expenses/summary", get(summary))
        .route("/api/v1/expenses/sources", get(sources))
        .route(
            "/api/v1/expenses/sources/{id}",
            patch(update_source).layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route("/api/v1/expenses/transactions", get(transactions))
        .route(
            "/api/v1/expenses/transactions/{id}",
            patch(override_transaction)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route("/api/v1/expenses/reviews", get(reviews))
        .route(
            "/api/v1/expenses/reviews/{id}/resolve",
            post(resolve_review).layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/imports",
            post(import_expenses).layer(DefaultBodyLimit::max(MAX_EXPENSE_IMPORT_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/imports/preview",
            post(preview_import).layer(DefaultBodyLimit::max(MAX_EXPENSE_PREVIEW_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/recurring",
            get(list_recurring)
                .post(create_recurring)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/recurring/{id}",
            patch(update_recurring)
                .delete(delete_recurring)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/recurring/occurrences",
            get(recurring_occurrences),
        )
        .route(
            "/api/v1/expenses/recurring/occurrences/{key}/confirm-paid",
            post(confirm_recurring_paid)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/recurring/occurrences/{key}/match",
            post(match_recurring).layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/reports",
            post(super::expense_report::generate)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
        .route(
            "/api/v1/expenses/reports/latest",
            get(super::expense_report::latest),
        )
        .route(
            "/api/v1/expenses/reports/{id}/feedback",
            post(super::expense_report::feedback)
                .layer(DefaultBodyLimit::max(MAX_EXPENSE_MUTATION_BODY_BYTES)),
        )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MonthQuery {
    month: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransactionQuery {
    month: String,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ReviewQuery {
    month: Option<String>,
    status: Option<ExpenseReviewStatus>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExpenseTransactionDto {
    id: String,
    kind: ExpenseEventKind,
    category: ExpenseCategory,
    status: tm_core::ExpenseEventStatus,
    amount_minor: i64,
    currency: String,
    occurred_at: String,
    posted_date: NaiveDate,
    source_kind: Option<ExpenseSourceKind>,
    merchant: Option<String>,
    counterparty: Option<String>,
    memo: Option<String>,
    payment_method_fingerprint: Option<String>,
    exclusion_reason: Option<String>,
    duplicate_of_event_id: Option<String>,
    personal_amount_minor: Option<i64>,
    related_event_id: Option<String>,
    is_provisional: bool,
    pending_review_id: Option<String>,
    version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpenseTransactionPageDto {
    items: Vec<ExpenseTransactionDto>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpenseReviewDto {
    id: String,
    reason: tm_core::ExpenseReviewReason,
    status: ExpenseReviewStatus,
    transaction: ExpenseTransactionDto,
    recurring_expense_id: Option<String>,
    suggested_kind: Option<ExpenseEventKind>,
    suggested_category: Option<ExpenseCategory>,
    suggested_duplicate_of_event_id: Option<String>,
    created_at: String,
    resolved_at: Option<String>,
    version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpenseReviewPageDto {
    items: Vec<ExpenseReviewDto>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RecurringExpenseItemDto {
    id: String,
    name: String,
    category: ExpenseCategory,
    vendor: Option<String>,
    amount_minor: i64,
    currency: String,
    payment_method_fingerprint: Option<String>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    memo: Option<String>,
    reminder_days: u8,
    amount_kind: RecurringAmountKind,
    interval_months: u8,
    due_rule: RecurringDueRule,
    due_day: Option<u8>,
    status: RecurringExpenseStatus,
    auto_match_enabled: bool,
    created_at: String,
    updated_at: String,
    version: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RecurringExpenseOccurrenceDto {
    occurrence_key: String,
    recurring_expense_id: String,
    item_version: u64,
    name: String,
    category: ExpenseCategory,
    vendor: Option<String>,
    due_date: NaiveDate,
    expected_amount_minor: i64,
    actual_amount_minor: Option<i64>,
    currency: String,
    status: tm_core::RecurringOccurrenceStatus,
    reminder_days: u8,
    amount_changed: bool,
    actual_event_id: Option<String>,
    version: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlainNormalizedExpenseRow {
    stable_key: String,
    row_sha256: String,
    source_row_number: u32,
    occurred_at: String,
    posted_date: NaiveDate,
    direction: tm_core::ExpenseDirection,
    amount_minor: i64,
    currency: String,
    kind: ExpenseEventKind,
    category_hint: Option<ExpenseCategory>,
    merchant: Option<String>,
    counterparty: Option<String>,
    memo: Option<String>,
    payment_method_fingerprint: Option<String>,
    external_reference_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ExpenseImportBody {
    #[serde(default)]
    preview_session_id: Option<String>,
    adapter: tm_core::ExpenseImportAdapter,
    source_kind: ExpenseSourceKind,
    source_fingerprint: String,
    file_sha256: String,
    normalized_sha256: String,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    #[serde(default)]
    rejected_count: u32,
    rows: Vec<PlainNormalizedExpenseRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalPlainNormalizedExpenseRow<'a> {
    occurred_at: &'a str,
    posted_date: NaiveDate,
    direction: tm_core::ExpenseDirection,
    amount_minor: i64,
    currency: &'a str,
    kind: ExpenseEventKind,
    category_hint: Option<ExpenseCategory>,
    merchant: Option<&'a str>,
    counterparty: Option<&'a str>,
    memo: Option<&'a str>,
    payment_method_fingerprint: Option<&'a str>,
    external_reference_fingerprint: Option<&'a str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalPlainExpenseImport<'a> {
    adapter: tm_core::ExpenseImportAdapter,
    source_kind: ExpenseSourceKind,
    source_fingerprint: &'a str,
    file_sha256: &'a str,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    rejected_count: u32,
    rows: &'a [PlainNormalizedExpenseRow],
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResolveReviewBody {
    kind: ExpenseEventKind,
    category: ExpenseCategory,
    duplicate_of_event_id: Option<String>,
    related_event_id: Option<String>,
    personal_amount_minor: Option<i64>,
    #[serde(default)]
    create_rule: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UpdateExpenseSourceBody {
    required_for_complete_report: bool,
    is_active: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct OverrideExpenseTransactionBody {
    kind: ExpenseEventKind,
    category: ExpenseCategory,
    duplicate_of_event_id: Option<String>,
    related_event_id: Option<String>,
    personal_amount_minor: Option<i64>,
    #[serde(default)]
    clear_related_event: bool,
    #[serde(default)]
    clear_personal_amount: bool,
    #[serde(default)]
    create_rule: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct CreateRecurringBody {
    name: String,
    category: ExpenseCategory,
    vendor: Option<String>,
    amount_minor: i64,
    currency: String,
    payment_method_fingerprint: Option<String>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    memo: Option<String>,
    #[serde(default = "default_reminder_days")]
    reminder_days: u8,
    amount_kind: RecurringAmountKind,
    #[serde(default = "default_interval_months")]
    interval_months: u8,
    due_rule: RecurringDueRule,
    due_day: Option<u8>,
    #[serde(default = "default_recurring_status")]
    status: RecurringExpenseStatus,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UpdateRecurringBody {
    effective_from_month: NaiveDate,
    name: String,
    category: ExpenseCategory,
    vendor: Option<String>,
    amount_minor: i64,
    currency: String,
    payment_method_fingerprint: Option<String>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    memo: Option<String>,
    reminder_days: u8,
    amount_kind: RecurringAmountKind,
    interval_months: u8,
    due_rule: RecurringDueRule,
    due_day: Option<u8>,
    status: RecurringExpenseStatus,
    auto_match_enabled: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConfirmRecurringPaidBody {
    amount_minor: Option<i64>,
    paid_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct MatchRecurringBody {
    event_id: String,
    #[serde(default)]
    enable_future_auto_match: bool,
}

const fn default_reminder_days() -> u8 {
    7
}

const fn default_interval_months() -> u8 {
    1
}

const fn default_recurring_status() -> RecurringExpenseStatus {
    RecurringExpenseStatus::Active
}

pub(super) async fn summary(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<MonthQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<ExpenseMonthSummary>>, ApiError> {
    let month = parse_month_query(query, &request_id)?;
    let data = core_read(state.core.clone(), &request_id, move |core| {
        core.get_expense_month_summary(month)
    })
    .await?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn sources(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ApiEnvelope<Vec<ExpenseSourceStatus>>>, ApiError> {
    let data = core_read(state.core.clone(), &request_id, |core| {
        core.list_expense_sources()
    })
    .await?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn update_source(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(source_id): Path<String>,
    payload: Result<Json<UpdateExpenseSourceBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&source_id, "expense source ID", &request_id)?;
    expense_crypto(&state, &request_id)?;
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_EXPENSE_SOURCE_UPDATE,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let (source, replayed): (ExpenseSourceStatus, bool) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::UpdateSource {
            source_id,
            input: UpdateExpenseSourceStatusInput {
                expected_version: required_expected_version(&preconditions, &request_id)?,
                required_for_complete_report: body.required_for_complete_report,
                is_active: body.is_active,
            },
        },
    )
    .await?;
    let version = source.version;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        source,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn transactions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<TransactionQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<ExpenseTransactionPageDto>>, ApiError> {
    let query = parse_query(query, &request_id)?;
    let month_start = parse_month(&query.month, &request_id)?;
    let limit = page_limit(query.limit, &request_id)?;
    validate_cursor(query.cursor.as_deref(), &request_id)?;
    let page = core_read(state.core.clone(), &request_id, move |core| {
        core.list_expense_transactions(ExpenseTransactionFilter {
            month_start,
            cursor: query.cursor,
            limit,
        })
    })
    .await?;
    let data = decrypt_transaction_page(page, expense_crypto(&state, &request_id)?, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn override_transaction(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(event_id): Path<String>,
    payload: Result<Json<OverrideExpenseTransactionBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&event_id, "expense event ID", &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_EXPENSE_TRANSACTION_OVERRIDE,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let (transaction, replayed): (ExpenseTransaction, bool) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::OverrideTransaction {
            event_id,
            input: OverrideExpenseTransactionInput {
                expected_version: required_expected_version(&preconditions, &request_id)?,
                kind: body.kind,
                category: body.category,
                duplicate_of_event_id: body.duplicate_of_event_id,
                related_event_id: body.related_event_id,
                personal_amount_minor: body.personal_amount_minor,
                clear_related_event: body.clear_related_event,
                clear_personal_amount: body.clear_personal_amount,
                create_rule: body.create_rule,
            },
        },
    )
    .await?;
    let version = transaction.version;
    let dto = decrypt_transaction(transaction, &crypto, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn reviews(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<ReviewQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<ExpenseReviewPageDto>>, ApiError> {
    let query = parse_query(query, &request_id)?;
    let month_start = query
        .month
        .as_deref()
        .map(|month| parse_month(month, &request_id))
        .transpose()?;
    let limit = page_limit(query.limit, &request_id)?;
    validate_cursor(query.cursor.as_deref(), &request_id)?;
    let page = core_read(state.core.clone(), &request_id, move |core| {
        core.list_expense_reviews(ExpenseReviewFilter {
            month_start,
            status: query.status,
            cursor: query.cursor,
            limit,
        })
    })
    .await?;
    let data = decrypt_review_page(page, expense_crypto(&state, &request_id)?, &request_id)?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn import_expenses(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    payload: Result<Json<ExpenseImportBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_primary_admin(&session, &request_id)?;
    let mut body = parse_json(payload, MAX_EXPENSE_IMPORT_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_EXPENSE_IMPORT,
        ExpectedVersion::Absent,
        &request_id,
    )?;
    let preview_session_id = body.preview_session_id.take().ok_or_else(|| {
        input_error(
            "EXPENSE_PREVIEW_SESSION_REQUIRED",
            "previewSessionId is required for an expense import",
            &request_id,
        )
    })?;
    validate_opaque_id(
        &preview_session_id,
        "expense import preview session ID",
        &request_id,
    )?;
    normalize_and_bind_plain_import(&mut body, &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let input = encrypt_import(body, &crypto, &preconditions.idempotency_key, &request_id)?;
    let (result, replayed): (tm_core::ExpenseImportResult, bool) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::Import {
            preview_session_id,
            input,
        },
    )
    .await?;
    mutation_json_response(
        request_id,
        if replayed {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        result,
        None,
        replayed,
        &preconditions,
    )
}

pub(super) async fn preview_import(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    payload: Result<Json<ExpenseImportBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    require_primary_admin(&session, &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let mut body = parse_json(payload, MAX_EXPENSE_PREVIEW_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_EXPENSE_IMPORT_PREVIEW,
        ExpectedVersion::Absent,
        &request_id,
    )?;
    if body.preview_session_id.take().is_some() {
        return Err(input_error(
            "EXPENSE_PREVIEW_SESSION_NOT_ALLOWED",
            "previewSessionId must be omitted when creating an expense import preview",
            &request_id,
        ));
    }
    normalize_and_bind_plain_import(&mut body, &request_id)?;
    let input = redacted_preview_input(&body, &crypto);
    let (data, replayed): (ExpenseImportPreview, bool) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::PreviewImport(input),
    )
    .await?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        data,
        None,
        replayed,
        &preconditions,
    )
}

pub(super) async fn resolve_review(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(review_id): Path<String>,
    payload: Result<Json<ResolveReviewBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&review_id, "review ID", &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_REVIEW_RESOLVE,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let expected_version = required_expected_version(&preconditions, &request_id)?;
    let (review, replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::ResolveReview {
            review_id,
            input: ResolveExpenseReviewInput {
                expected_version,
                kind: body.kind,
                category: body.category,
                duplicate_of_event_id: body.duplicate_of_event_id,
                related_event_id: body.related_event_id,
                personal_amount_minor: body.personal_amount_minor,
                create_rule: body.create_rule,
            },
        },
    )
    .await?;
    let version = review.version;
    let dto = decrypt_review(review, &crypto, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn list_recurring(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
) -> Result<Json<ApiEnvelope<Vec<RecurringExpenseItemDto>>>, ApiError> {
    let items = core_read(state.core.clone(), &request_id, |core| {
        core.list_recurring_expenses()
    })
    .await?;
    let crypto = expense_crypto(&state, &request_id)?;
    let data = items
        .into_iter()
        .map(|item| decrypt_recurring(item, crypto, &request_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn create_recurring(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    payload: Result<Json<CreateRecurringBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_RECURRING_CREATE,
        ExpectedVersion::Absent,
        &request_id,
    )?;
    let recurring_expense_id = format!("rec-{}", sha256_hex(&preconditions.idempotency_key));
    let context = format!("recurring:{recurring_expense_id}");
    let input = encrypt_create_recurring(
        body,
        recurring_expense_id,
        &context,
        expense_crypto(&state, &request_id)?,
        &preconditions.idempotency_key,
        &request_id,
    )?;
    let (item, replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::CreateRecurring(input),
    )
    .await?;
    let version = item.version;
    let dto = decrypt_recurring(item, expense_crypto(&state, &request_id)?, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::CREATED,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn update_recurring(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
    payload: Result<Json<UpdateRecurringBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&item_id, "recurring expense ID", &request_id)?;
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_RECURRING_UPDATE,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let input = encrypt_update_recurring(
        body,
        required_expected_version(&preconditions, &request_id)?,
        &format!("recurring:{item_id}"),
        expense_crypto(&state, &request_id)?,
        &preconditions.idempotency_key,
        &request_id,
    )?;
    let (item, replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::UpdateRecurring {
            recurring_expense_id: item_id,
            input,
        },
    )
    .await?;
    let version = item.version;
    let dto = decrypt_recurring(item, expense_crypto(&state, &request_id)?, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn delete_recurring(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&item_id, "recurring expense ID", &request_id)?;
    expense_crypto(&state, &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_RECURRING_DELETE,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let expected_version = required_expected_version(&preconditions, &request_id)?;
    let ((), replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::DeleteRecurring {
            recurring_expense_id: item_id,
            expected_version,
        },
    )
    .await?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        DeletedResource { deleted: true },
        None,
        replayed,
        &preconditions,
    )
}

pub(super) async fn recurring_occurrences(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    query: Result<Query<MonthQuery>, QueryRejection>,
) -> Result<Json<ApiEnvelope<Vec<RecurringExpenseOccurrenceDto>>>, ApiError> {
    let month = parse_month_query(query, &request_id)?;
    let items = core_read(state.core.clone(), &request_id, move |core| {
        core.recurring_expense_occurrences(month)
    })
    .await?;
    let crypto = expense_crypto(&state, &request_id)?;
    let data = items
        .into_iter()
        .map(|item| decrypt_occurrence(item, crypto, &request_id))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data,
    }))
}

pub(super) async fn confirm_recurring_paid(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(occurrence_key): Path<String>,
    payload: Result<Json<ConfirmRecurringPaidBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&occurrence_key, "recurring occurrence key", &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_RECURRING_PAID,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let input = ConfirmRecurringPaidInput {
        expected_version: required_expected_version(&preconditions, &request_id)?,
        amount_minor: body.amount_minor,
        paid_date: body.paid_date,
    };
    let (item, replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::ConfirmRecurringPaid {
            occurrence_key,
            input,
        },
    )
    .await?;
    let version = item.version;
    let dto = decrypt_occurrence(item, &crypto, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

pub(super) async fn match_recurring(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    Extension(session): Extension<AuthenticatedSession>,
    headers: HeaderMap,
    Path(occurrence_key): Path<String>,
    payload: Result<Json<MatchRecurringBody>, JsonRejection>,
) -> Result<Response, ApiError> {
    validate_opaque_id(&occurrence_key, "recurring occurrence key", &request_id)?;
    let crypto = expense_crypto(&state, &request_id)?.clone();
    let body = parse_json(payload, MAX_EXPENSE_MUTATION_BODY_BYTES, &request_id)?.0;
    validate_opaque_id(&body.event_id, "expense event ID", &request_id)?;
    let preconditions = mutation_preconditions(
        &headers,
        CONFIRM_RECURRING_MATCH,
        ExpectedVersion::Exact,
        &request_id,
    )?;
    let input = MatchRecurringExpenseInput {
        expected_version: required_expected_version(&preconditions, &request_id)?,
        event_id: body.event_id,
        enable_future_auto_match: body.enable_future_auto_match,
    };
    let (item, replayed) = execute_expense_mutation(
        state.core.clone(),
        &request_id,
        &session,
        &preconditions,
        ExpenseMutationCommand::MatchRecurring {
            occurrence_key,
            input,
        },
    )
    .await?;
    let version = item.version;
    let dto = decrypt_occurrence(item, &crypto, &request_id)?;
    mutation_json_response(
        request_id,
        StatusCode::OK,
        dto,
        Some(version),
        replayed,
        &preconditions,
    )
}

#[derive(Debug, Serialize)]
struct DeletedResource {
    deleted: bool,
}

fn normalize_plain_import(
    body: &mut ExpenseImportBody,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    for row in &mut body.rows {
        row.merchant = normalize_optional_text(
            row.merchant.take(),
            "merchant",
            MAX_FINANCIAL_TEXT_CHARS,
            request_id,
        )?;
        row.counterparty = normalize_optional_text(
            row.counterparty.take(),
            "counterparty",
            MAX_FINANCIAL_TEXT_CHARS,
            request_id,
        )?;
        row.memo = normalize_optional_text(
            row.memo.take(),
            "memo",
            MAX_FINANCIAL_TEXT_CHARS,
            request_id,
        )?;
    }
    Ok(())
}

fn normalize_and_bind_plain_import(
    body: &mut ExpenseImportBody,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    normalize_plain_import(body, request_id)?;
    for row in &mut body.rows {
        row.row_sha256 = canonical_expense_row_sha256(row, request_id)?;
    }
    body.normalized_sha256 = canonical_expense_import_sha256(body, request_id)?;
    Ok(())
}

fn canonical_expense_row_sha256(
    row: &PlainNormalizedExpenseRow,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    sha256_serialized(
        &CanonicalPlainNormalizedExpenseRow {
            occurred_at: &row.occurred_at,
            posted_date: row.posted_date,
            direction: row.direction,
            amount_minor: row.amount_minor,
            currency: &row.currency,
            kind: row.kind,
            category_hint: row.category_hint,
            merchant: row.merchant.as_deref(),
            counterparty: row.counterparty.as_deref(),
            memo: row.memo.as_deref(),
            payment_method_fingerprint: row.payment_method_fingerprint.as_deref(),
            external_reference_fingerprint: row.external_reference_fingerprint.as_deref(),
        },
        request_id,
    )
}

fn canonical_expense_import_sha256(
    body: &ExpenseImportBody,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    sha256_serialized(
        &CanonicalPlainExpenseImport {
            adapter: body.adapter,
            source_kind: body.source_kind,
            source_fingerprint: &body.source_fingerprint,
            file_sha256: &body.file_sha256,
            coverage_start: body.coverage_start,
            coverage_end: body.coverage_end,
            rejected_count: body.rejected_count,
            rows: &body.rows,
        },
        request_id,
    )
}

fn sha256_serialized<T: Serialize>(value: &T, request_id: &RequestId) -> Result<String, ApiError> {
    let encoded = Zeroizing::new(
        serde_json::to_vec(value)
            .map_err(|_| internal_error("EXPENSE_CANONICAL_ENCODING_FAILED", request_id))?,
    );
    Ok(format!("{:x}", Sha256::digest(&*encoded)))
}

fn redacted_preview_input(
    body: &ExpenseImportBody,
    crypto: &ExpenseCrypto,
) -> ExpenseImportPreviewInput {
    let source_fingerprint = crypto.blind_index("source_fingerprint", &body.source_fingerprint);
    ExpenseImportPreviewInput {
        adapter: body.adapter,
        source_kind: body.source_kind,
        source_fingerprint,
        file_sha256: crypto.blind_index("file_digest", &body.file_sha256),
        normalized_sha256: crypto.blind_index("normalized_digest", &body.normalized_sha256),
        coverage_start: body.coverage_start,
        coverage_end: body.coverage_end,
        rejected_count: body.rejected_count,
        rows: body
            .rows
            .iter()
            .map(|row| tm_core::ExpenseImportPreviewRow {
                stable_key: crypto.blind_index("stable_key", &row.stable_key),
                row_sha256: crypto.blind_index("row_digest", &row.row_sha256),
                source_row_number: row.source_row_number,
                occurred_at: row.occurred_at.clone(),
                posted_date: row.posted_date,
                direction: row.direction,
                amount_minor: row.amount_minor,
                currency: row.currency.clone(),
                kind: row.kind,
                category_hint: row.category_hint,
                merchant_blind_index: row
                    .merchant
                    .as_deref()
                    .map(|merchant| crypto.blind_index("merchant", merchant)),
                payment_method_fingerprint: row
                    .payment_method_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("payment_method", value)),
                external_reference_fingerprint: row
                    .external_reference_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("external_reference", value)),
            })
            .collect(),
    }
}

#[cfg(test)]
pub(super) fn sign_test_expense_import_value(value: serde_json::Value) -> serde_json::Value {
    let request_id = RequestId("expense-canonical-test".to_owned());
    let mut body = serde_json::from_value::<ExpenseImportBody>(value)
        .expect("decode test expense import body");
    normalize_plain_import(&mut body, &request_id).expect("normalize test expense import body");
    for row in &mut body.rows {
        row.row_sha256 = canonical_expense_row_sha256(row, &request_id)
            .expect("hash canonical test expense row");
    }
    body.normalized_sha256 = canonical_expense_import_sha256(&body, &request_id)
        .expect("hash canonical test expense import");
    serde_json::to_value(body).expect("encode signed test expense import body")
}

fn encrypt_import(
    body: ExpenseImportBody,
    crypto: &ExpenseCrypto,
    idempotency_key: &str,
    request_id: &RequestId,
) -> Result<NormalizedExpenseImport, ApiError> {
    let source_fingerprint = crypto.blind_index("source_fingerprint", &body.source_fingerprint);
    let file_sha256 = crypto.blind_index("file_digest", &body.file_sha256);
    let normalized_sha256 = crypto.blind_index("normalized_digest", &body.normalized_sha256);
    let rows = body
        .rows
        .into_iter()
        .map(|row| {
            let stable_key = crypto.blind_index("stable_key", &row.stable_key);
            let context = format!("{source_fingerprint}:{stable_key}");
            Ok(NormalizedExpenseRow {
                stable_key,
                row_sha256: crypto.blind_index("row_digest", &row.row_sha256),
                source_row_number: row.source_row_number,
                occurred_at: row.occurred_at,
                posted_date: row.posted_date,
                direction: row.direction,
                amount_minor: row.amount_minor,
                currency: row.currency,
                kind: row.kind,
                category_hint: row.category_hint,
                merchant: encrypt_optional(
                    crypto,
                    &context,
                    "merchant",
                    row.merchant,
                    idempotency_key,
                    request_id,
                )?,
                counterparty: encrypt_optional(
                    crypto,
                    &context,
                    "counterparty",
                    row.counterparty,
                    idempotency_key,
                    request_id,
                )?,
                memo: encrypt_optional(
                    crypto,
                    &context,
                    "memo",
                    row.memo,
                    idempotency_key,
                    request_id,
                )?,
                payment_method_fingerprint: row
                    .payment_method_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("payment_method", value)),
                external_reference_fingerprint: row
                    .external_reference_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("external_reference", value)),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(NormalizedExpenseImport {
        adapter: body.adapter,
        source_kind: body.source_kind,
        source_fingerprint,
        file_sha256,
        normalized_sha256,
        coverage_start: body.coverage_start,
        coverage_end: body.coverage_end,
        rejected_count: body.rejected_count,
        rows,
    })
}

fn encrypt_create_recurring(
    body: CreateRecurringBody,
    id: String,
    context: &str,
    crypto: &ExpenseCrypto,
    idempotency_key: &str,
    request_id: &RequestId,
) -> Result<CreateRecurringExpenseInput, ApiError> {
    let name = normalize_required_text(
        body.name,
        "recurring expense name",
        MAX_RECURRING_NAME_CHARS,
        request_id,
    )?;
    let vendor = normalize_optional_text(
        body.vendor,
        "recurring expense vendor",
        MAX_VENDOR_CHARS,
        request_id,
    )?;
    let memo = normalize_optional_text(
        body.memo,
        "recurring expense memo",
        MAX_FINANCIAL_TEXT_CHARS,
        request_id,
    )?;
    Ok(CreateRecurringExpenseInput {
        id,
        name: encrypt_required(crypto, context, "name", name, idempotency_key, request_id)?,
        category: body.category,
        vendor: encrypt_optional(
            crypto,
            context,
            "vendor",
            vendor,
            idempotency_key,
            request_id,
        )?,
        amount_minor: body.amount_minor,
        currency: body.currency,
        payment_method_fingerprint: body.payment_method_fingerprint,
        start_date: body.start_date,
        end_date: body.end_date,
        memo: encrypt_optional(crypto, context, "memo", memo, idempotency_key, request_id)?,
        reminder_days: body.reminder_days,
        amount_kind: body.amount_kind,
        interval_months: body.interval_months,
        due_rule: body.due_rule,
        due_day: body.due_day,
        status: body.status,
    })
}

fn encrypt_update_recurring(
    body: UpdateRecurringBody,
    expected_version: u64,
    context: &str,
    crypto: &ExpenseCrypto,
    idempotency_key: &str,
    request_id: &RequestId,
) -> Result<UpdateRecurringExpenseInput, ApiError> {
    let name = normalize_required_text(
        body.name,
        "recurring expense name",
        MAX_RECURRING_NAME_CHARS,
        request_id,
    )?;
    let vendor = normalize_optional_text(
        body.vendor,
        "recurring expense vendor",
        MAX_VENDOR_CHARS,
        request_id,
    )?;
    let memo = normalize_optional_text(
        body.memo,
        "recurring expense memo",
        MAX_FINANCIAL_TEXT_CHARS,
        request_id,
    )?;
    Ok(UpdateRecurringExpenseInput {
        expected_version,
        effective_from_month: body.effective_from_month,
        name: encrypt_required(crypto, context, "name", name, idempotency_key, request_id)?,
        category: body.category,
        vendor: encrypt_optional(
            crypto,
            context,
            "vendor",
            vendor,
            idempotency_key,
            request_id,
        )?,
        amount_minor: body.amount_minor,
        currency: body.currency,
        payment_method_fingerprint: body.payment_method_fingerprint,
        start_date: body.start_date,
        end_date: body.end_date,
        memo: encrypt_optional(crypto, context, "memo", memo, idempotency_key, request_id)?,
        reminder_days: body.reminder_days,
        amount_kind: body.amount_kind,
        interval_months: body.interval_months,
        due_rule: body.due_rule,
        due_day: body.due_day,
        status: body.status,
        auto_match_enabled: body.auto_match_enabled,
    })
}

fn encrypt_optional(
    crypto: &ExpenseCrypto,
    context: &str,
    field: &str,
    value: Option<String>,
    idempotency_key: &str,
    request_id: &RequestId,
) -> Result<Option<EncryptedExpenseText>, ApiError> {
    value
        .map(|value| encrypt_required(crypto, context, field, value, idempotency_key, request_id))
        .transpose()
}

fn normalize_required_text(
    value: String,
    label: &str,
    maximum_chars: usize,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    let value = Zeroizing::new(value);
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > maximum_chars
        || value.chars().any(|character| character == '\0')
    {
        return Err(input_error(
            "INVALID_EXPENSE_SENSITIVE_TEXT",
            &format!("{label} must contain between 1 and {maximum_chars} characters"),
            request_id,
        ));
    }
    Ok(value.to_owned())
}

fn normalize_optional_text(
    value: Option<String>,
    label: &str,
    maximum_chars: usize,
    request_id: &RequestId,
) -> Result<Option<String>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = Zeroizing::new(value);
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    normalize_required_text(value.to_owned(), label, maximum_chars, request_id).map(Some)
}

fn encrypt_required(
    crypto: &ExpenseCrypto,
    context: &str,
    field: &str,
    value: String,
    idempotency_key: &str,
    request_id: &RequestId,
) -> Result<EncryptedExpenseText, ApiError> {
    let aad = expense_text_aad(context, field);
    let value = Zeroizing::new(value);
    let encrypted = crypto
        .encrypt_idempotent(field, aad.as_bytes(), &value, idempotency_key)
        .map_err(|error| map_crypto_error(error, request_id))?;
    Ok(core_envelope(encrypted, aad))
}

fn core_envelope(value: EncryptedExpenseValue, aad: String) -> EncryptedExpenseText {
    EncryptedExpenseText {
        key_version: value.key_version,
        nonce: value.nonce,
        ciphertext: value.ciphertext,
        aad,
        blind_index: value.blind_index,
    }
}

fn decrypt_transaction_page(
    page: ExpenseTransactionPage,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<ExpenseTransactionPageDto, ApiError> {
    Ok(ExpenseTransactionPageDto {
        items: page
            .items
            .into_iter()
            .map(|item| decrypt_transaction(item, crypto, request_id))
            .collect::<Result<Vec<_>, _>>()?,
        next_cursor: page.next_cursor,
    })
}

fn decrypt_review_page(
    page: ExpenseReviewPage,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<ExpenseReviewPageDto, ApiError> {
    Ok(ExpenseReviewPageDto {
        items: page
            .items
            .into_iter()
            .map(|item| decrypt_review(item, crypto, request_id))
            .collect::<Result<Vec<_>, _>>()?,
        next_cursor: page.next_cursor,
    })
}

fn decrypt_transaction(
    item: ExpenseTransaction,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<ExpenseTransactionDto, ApiError> {
    let crypto_context = item.crypto_context.clone();
    Ok(ExpenseTransactionDto {
        id: item.id,
        kind: item.kind,
        category: item.category,
        status: item.status,
        amount_minor: item.amount_minor,
        currency: item.currency,
        occurred_at: item.occurred_at,
        posted_date: item.posted_date,
        source_kind: item.source_kind,
        merchant: decrypt_optional(
            crypto,
            item.merchant,
            crypto_context.as_deref(),
            "merchant",
            request_id,
        )?,
        counterparty: decrypt_optional(
            crypto,
            item.counterparty,
            crypto_context.as_deref(),
            "counterparty",
            request_id,
        )?,
        memo: decrypt_optional(
            crypto,
            item.memo,
            crypto_context.as_deref(),
            "memo",
            request_id,
        )?,
        payment_method_fingerprint: item.payment_method_fingerprint,
        exclusion_reason: item.exclusion_reason,
        duplicate_of_event_id: item.duplicate_of_event_id,
        personal_amount_minor: item.personal_amount_minor,
        related_event_id: item.related_event_id,
        is_provisional: item.is_provisional,
        pending_review_id: item.pending_review_id,
        version: item.version,
    })
}

fn decrypt_review(
    item: ExpenseReview,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<ExpenseReviewDto, ApiError> {
    Ok(ExpenseReviewDto {
        id: item.id,
        reason: item.reason,
        status: item.status,
        transaction: decrypt_transaction(item.transaction, crypto, request_id)?,
        recurring_expense_id: item.recurring_expense_id,
        suggested_kind: item.suggested_kind,
        suggested_category: item.suggested_category,
        suggested_duplicate_of_event_id: item.suggested_duplicate_of_event_id,
        created_at: item.created_at,
        resolved_at: item.resolved_at,
        version: item.version,
    })
}

fn decrypt_recurring(
    item: RecurringExpenseItem,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<RecurringExpenseItemDto, ApiError> {
    let context = format!("recurring:{}", item.id);
    Ok(RecurringExpenseItemDto {
        id: item.id,
        name: decrypt_required(crypto, item.name, &context, "name", request_id)?,
        category: item.category,
        vendor: decrypt_optional(crypto, item.vendor, Some(&context), "vendor", request_id)?,
        amount_minor: item.amount_minor,
        currency: item.currency,
        payment_method_fingerprint: item.payment_method_fingerprint,
        start_date: item.start_date,
        end_date: item.end_date,
        memo: decrypt_optional(crypto, item.memo, Some(&context), "memo", request_id)?,
        reminder_days: item.reminder_days,
        amount_kind: item.amount_kind,
        interval_months: item.interval_months,
        due_rule: item.due_rule,
        due_day: item.due_day,
        status: item.status,
        auto_match_enabled: item.auto_match_enabled,
        created_at: item.created_at,
        updated_at: item.updated_at,
        version: item.version,
    })
}

pub(super) fn decrypt_occurrence(
    item: RecurringExpenseOccurrence,
    crypto: &ExpenseCrypto,
    request_id: &RequestId,
) -> Result<RecurringExpenseOccurrenceDto, ApiError> {
    let context = format!("recurring:{}", item.recurring_expense_id);
    Ok(RecurringExpenseOccurrenceDto {
        occurrence_key: item.occurrence_key,
        recurring_expense_id: item.recurring_expense_id,
        item_version: item.item_version,
        name: decrypt_required(crypto, item.name, &context, "name", request_id)?,
        category: item.category,
        vendor: decrypt_optional(crypto, item.vendor, Some(&context), "vendor", request_id)?,
        due_date: item.due_date,
        expected_amount_minor: item.expected_amount_minor,
        actual_amount_minor: item.actual_amount_minor,
        currency: item.currency,
        status: item.status,
        reminder_days: item.reminder_days,
        amount_changed: item.amount_changed,
        actual_event_id: item.actual_event_id,
        version: item.version,
    })
}

fn decrypt_optional(
    crypto: &ExpenseCrypto,
    value: Option<EncryptedExpenseText>,
    expected_context: Option<&str>,
    expected_field: &str,
    request_id: &RequestId,
) -> Result<Option<String>, ApiError> {
    value
        .map(|value| {
            let context = expected_context
                .ok_or_else(|| map_crypto_error(ExpenseCryptoError::InvalidEnvelope, request_id))?;
            decrypt_required(crypto, value, context, expected_field, request_id)
        })
        .transpose()
}

fn decrypt_required(
    crypto: &ExpenseCrypto,
    value: EncryptedExpenseText,
    expected_context: &str,
    expected_field: &str,
    request_id: &RequestId,
) -> Result<String, ApiError> {
    if value.aad != expense_text_aad(expected_context, expected_field) {
        return Err(map_crypto_error(
            ExpenseCryptoError::InvalidEnvelope,
            request_id,
        ));
    }
    crypto
        .decrypt(
            value.key_version,
            &value.nonce,
            &value.ciphertext,
            value.aad.as_bytes(),
        )
        .map_err(|error| map_crypto_error(error, request_id))
}

pub(super) fn expense_crypto<'a>(
    state: &'a AppState,
    request_id: &RequestId,
) -> Result<&'a ExpenseCrypto, ApiError> {
    state
        .expense_crypto
        .as_ref()
        .map_err(|error| map_crypto_error(*error, request_id))
}

fn map_crypto_error(error: ExpenseCryptoError, request_id: &RequestId) -> ApiError {
    tracing::error!(
        error_kind = ?error,
        request_id = %request_id.0,
        "expense data protection operation failed"
    );
    ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "EXPENSE_CRYPTO_UNAVAILABLE",
        message: "expense data protection is unavailable".to_owned(),
        request_id: request_id.0.clone(),
    }
}

#[derive(Debug)]
pub(super) struct MutationPreconditions {
    pub(super) idempotency_key: String,
    pub(super) expected_version: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ExpectedVersion {
    Absent,
    Exact,
}

pub(super) fn mutation_preconditions(
    headers: &HeaderMap,
    confirmation: &'static str,
    expected: ExpectedVersion,
    request_id: &RequestId,
) -> Result<MutationPreconditions, ApiError> {
    let idempotency_key = required_header(headers, &IDEMPOTENCY_KEY_HEADER, request_id)?;
    if idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || !idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(input_error(
            "INVALID_IDEMPOTENCY_KEY",
            "Idempotency-Key is invalid",
            request_id,
        ));
    }
    let supplied_confirmation = required_header(headers, &MUTATION_CONFIRM_HEADER, request_id)?;
    if supplied_confirmation != confirmation {
        return Err(ApiError {
            status: StatusCode::PRECONDITION_REQUIRED,
            code: "EXPENSE_CONFIRMATION_REQUIRED",
            message: format!("set x-tm-confirm-mutation to {confirmation}"),
            request_id: request_id.0.clone(),
        });
    }
    let expected_version = match expected {
        ExpectedVersion::Absent => {
            if required_header(headers, &header::IF_NONE_MATCH, request_id)? != "*" {
                return Err(ApiError {
                    status: StatusCode::PRECONDITION_REQUIRED,
                    code: "EXPENSE_EXPECTED_VERSION_REQUIRED",
                    message: "create mutations require If-None-Match: *".to_owned(),
                    request_id: request_id.0.clone(),
                });
            }
            None
        }
        ExpectedVersion::Exact => {
            let value = required_header(headers, &header::IF_MATCH, request_id)?;
            Some(parse_if_match(value).ok_or_else(|| {
                input_error(
                    "INVALID_EXPECTED_VERSION",
                    "If-Match must contain one quoted positive integer version",
                    request_id,
                )
            })?)
        }
    };
    Ok(MutationPreconditions {
        idempotency_key: idempotency_key.to_owned(),
        expected_version,
    })
}

fn required_expected_version(
    preconditions: &MutationPreconditions,
    request_id: &RequestId,
) -> Result<u64, ApiError> {
    preconditions.expected_version.ok_or_else(|| ApiError {
        status: StatusCode::PRECONDITION_REQUIRED,
        code: "EXPENSE_EXPECTED_VERSION_REQUIRED",
        message: "an exact expense resource version is required".to_owned(),
        request_id: request_id.0.clone(),
    })
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
            code: "EXPENSE_PRECONDITION_REQUIRED",
            message: format!("{} header is required", name.as_str()),
            request_id: request_id.0.clone(),
        })
}

fn parse_if_match(value: &str) -> Option<u64> {
    let version = value.strip_prefix('"')?.strip_suffix('"')?.parse().ok()?;
    (version > 0).then_some(version)
}

pub(super) fn mutation_json_response<T: Serialize>(
    request_id: RequestId,
    status: StatusCode,
    data: T,
    version: Option<u64>,
    replayed: bool,
    _preconditions: &MutationPreconditions,
) -> Result<Response, ApiError> {
    let etag = version
        .map(|version| HeaderValue::from_str(&format!("\"{version}\"")))
        .transpose()
        .map_err(|_| internal_error("EXPENSE_RESPONSE_INVALID", &request_id))?;
    let mut response = (
        status,
        Json(ApiEnvelope {
            request_id: request_id.0,
            data,
        }),
    )
        .into_response();
    if let Some(etag) = etag {
        response.headers_mut().insert(header::ETAG, etag);
    }
    if replayed {
        response.headers_mut().insert(
            IDEMPOTENCY_REPLAYED_HEADER,
            HeaderValue::from_static("true"),
        );
    }
    Ok(response)
}

fn require_primary_admin(
    session: &AuthenticatedSession,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    if matches!(session.subject, AuthenticatedSubject::PrimaryAdmin) {
        return Ok(());
    }
    Err(ApiError {
        status: StatusCode::FORBIDDEN,
        code: "EXPENSE_IMPORT_PRIMARY_REQUIRED",
        message: "expense import is restricted to the primary Windows administrator".to_owned(),
        request_id: request_id.0.clone(),
    })
}

fn parse_month_query(
    query: Result<Query<MonthQuery>, QueryRejection>,
    request_id: &RequestId,
) -> Result<NaiveDate, ApiError> {
    let query = parse_query(query, request_id)?;
    parse_month(&query.month, request_id)
}

pub(super) fn parse_month(value: &str, request_id: &RequestId) -> Result<NaiveDate, ApiError> {
    if value.len() != 7 || value.as_bytes().get(4) != Some(&b'-') {
        return Err(input_error(
            "INVALID_EXPENSE_MONTH",
            "month must use YYYY-MM format",
            request_id,
        ));
    }
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").map_err(|_| {
        input_error(
            "INVALID_EXPENSE_MONTH",
            "month must use YYYY-MM format",
            request_id,
        )
    })
}

fn page_limit(limit: Option<u32>, request_id: &RequestId) -> Result<u32, ApiError> {
    let limit = limit.unwrap_or(DEFAULT_PAGE_LIMIT);
    if !(1..=MAX_PAGE_LIMIT).contains(&limit) {
        return Err(input_error(
            "INVALID_EXPENSE_PAGE_LIMIT",
            "limit must be between 1 and 100",
            request_id,
        ));
    }
    Ok(limit)
}

fn validate_cursor(cursor: Option<&str>, request_id: &RequestId) -> Result<(), ApiError> {
    let invalid = cursor.is_some_and(|value| {
        if value.is_empty() || value.len() > MAX_CURSOR_BYTES {
            return true;
        }
        let Some((timestamp, id)) = value.split_once('|') else {
            return true;
        };
        timestamp.len() > 128
            || chrono::DateTime::parse_from_rfc3339(timestamp).is_err()
            || id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    });
    if invalid {
        return Err(input_error(
            "INVALID_EXPENSE_CURSOR",
            "cursor is invalid",
            request_id,
        ));
    }
    Ok(())
}

pub(super) fn validate_opaque_id(
    value: &str,
    label: &str,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(input_error(
            "INVALID_EXPENSE_RESOURCE_ID",
            &format!("{label} is invalid"),
            request_id,
        ));
    }
    Ok(())
}

fn parse_query<T>(
    query: Result<Query<T>, QueryRejection>,
    request_id: &RequestId,
) -> Result<T, ApiError> {
    query.map(|Query(value)| value).map_err(|_| {
        input_error(
            "INVALID_EXPENSE_QUERY",
            "expense query does not match the API contract",
            request_id,
        )
    })
}

fn parse_json<T>(
    payload: Result<Json<T>, JsonRejection>,
    maximum_bytes: usize,
    request_id: &RequestId,
) -> Result<Json<T>, ApiError> {
    payload.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                code: "EXPENSE_REQUEST_TOO_LARGE",
                message: format!("expense request exceeds {maximum_bytes} bytes"),
                request_id: request_id.0.clone(),
            }
        } else {
            input_error(
                "INVALID_EXPENSE_JSON",
                "expense request body does not match the API contract",
                request_id,
            )
        }
    })
}

pub(super) async fn core_read<T, F>(
    core: TmCore,
    request_id: &RequestId,
    operation: F,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&TmCore) -> tm_core::Result<T> + Send + 'static,
{
    let error_request_id = request_id.0.clone();
    tokio::task::spawn_blocking(move || operation(&core))
        .await
        .map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "EXPENSE_WORKER_FAILED",
            message: "expense data worker was unavailable".to_owned(),
            request_id: error_request_id.clone(),
        })?
        .map_err(|error| map_core_error(error, error_request_id))
}

pub(super) async fn execute_expense_mutation<T>(
    core: TmCore,
    request_id: &RequestId,
    session: &AuthenticatedSession,
    preconditions: &MutationPreconditions,
    command: ExpenseMutationCommand,
) -> Result<(T, bool), ApiError>
where
    T: DeserializeOwned + Send + 'static,
{
    let request = ExpenseMutationRequest {
        idempotency_key: preconditions.idempotency_key.clone(),
        actor: session.audit_actor(),
        command,
    };
    let result = core_read(core, request_id, move |core| {
        core.execute_expense_mutation(request)
    })
    .await?;
    let value = serde_json::from_value(result.value).map_err(|_| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "EXPENSE_MUTATION_RESPONSE_INVALID",
        message: "expense mutation response could not be decoded".to_owned(),
        request_id: request_id.0.clone(),
    })?;
    Ok((value, result.replayed))
}

fn map_core_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidInput(_) => ApiError {
            status: StatusCode::BAD_REQUEST,
            code: "INVALID_EXPENSE_REQUEST",
            message: "expense request failed validation".to_owned(),
            request_id,
        },
        CoreError::NotFound { .. } => ApiError {
            status: StatusCode::NOT_FOUND,
            code: "EXPENSE_RESOURCE_NOT_FOUND",
            message: "expense resource was not found".to_owned(),
            request_id,
        },
        CoreError::Conflict(_) => ApiError {
            status: StatusCode::CONFLICT,
            code: "EXPENSE_CONFLICT",
            message: "expense data changed or conflicts with an existing record".to_owned(),
            request_id,
        },
        _ => {
            // Expense errors can carry resource IDs, local paths, or database values. Keep
            // operational telemetry to a fixed family so financial metadata never reaches logs.
            tracing::error!(
                error_family = expense_core_error_family(&error),
                %request_id,
                "expense operation failed internally"
            );
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "EXPENSE_OPERATION_FAILED",
                message: "expense operation could not be completed".to_owned(),
                request_id,
            }
        }
    }
}

fn expense_core_error_family(error: &CoreError) -> &'static str {
    match error {
        CoreError::Database(_) => "database",
        CoreError::Io(_) => "io",
        CoreError::Json(_) => "json",
        CoreError::Zip(_) => "zip",
        CoreError::WalkDir(_) => "walkdir",
        CoreError::InvalidInput(_) => "invalid_input",
        CoreError::NotFound { .. } => "not_found",
        CoreError::Conflict(_) => "conflict",
        CoreError::AiBudgetExceeded { .. } => "ai_budget",
        CoreError::AiDailyLimitExceeded { .. } => "ai_daily_limit",
        CoreError::Invariant(_) => "invariant",
        CoreError::InvalidBackup(_) => "invalid_backup",
    }
}

fn input_error(code: &'static str, message: &str, request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        code,
        message: message.to_owned(),
        request_id: request_id.0.clone(),
    }
}

fn internal_error(code: &'static str, request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code,
        message: "expense response could not be produced".to_owned(),
        request_id: request_id.0.clone(),
    }
}

fn sha256_hex(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn route_allowed_for_device(method: &Method, path: &str) -> bool {
    match path {
        "/api/v1/expenses/summary"
        | "/api/v1/expenses/sources"
        | "/api/v1/expenses/transactions"
        | "/api/v1/expenses/reviews"
        | "/api/v1/expenses/recurring"
        | "/api/v1/expenses/recurring/occurrences" => {
            *method == Method::GET
                || (path == "/api/v1/expenses/recurring" && *method == Method::POST)
        }
        "/api/v1/expenses/reports/latest" => *method == Method::GET,
        "/api/v1/expenses/reports" => *method == Method::POST,
        "/api/v1/expenses/imports" => false,
        "/api/v1/expenses/imports/preview" => false,
        value if single_path_parameter(value, "/api/v1/expenses/reviews/", "/resolve") => {
            *method == Method::POST
        }
        value if single_path_parameter(value, "/api/v1/expenses/sources/", "") => {
            *method == Method::PATCH
        }
        value if single_path_parameter(value, "/api/v1/expenses/transactions/", "") => {
            *method == Method::PATCH
        }
        value
            if single_path_parameter(
                value,
                "/api/v1/expenses/recurring/occurrences/",
                "/confirm-paid",
            ) || single_path_parameter(
                value,
                "/api/v1/expenses/recurring/occurrences/",
                "/match",
            ) =>
        {
            *method == Method::POST
        }
        value if single_path_parameter(value, "/api/v1/expenses/recurring/", "") => {
            matches!(*method, Method::PATCH | Method::DELETE)
        }
        value if single_path_parameter(value, "/api/v1/expenses/reports/", "/feedback") => {
            *method == Method::POST
        }
        _ => false,
    }
}

fn single_path_parameter(value: &str, prefix: &str, suffix: &str) -> bool {
    value
        .strip_prefix(prefix)
        .and_then(|value| value.strip_suffix(suffix))
        .is_some_and(|parameter| !parameter.is_empty() && !parameter.contains('/'))
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue};

    use super::{
        CONFIRM_RECURRING_UPDATE, ExpectedVersion, MUTATION_CONFIRM_HEADER, MonthQuery,
        MutationPreconditions, mutation_preconditions, normalize_optional_text,
        normalize_required_text, parse_if_match, parse_month, route_allowed_for_device,
        sign_test_expense_import_value, validate_cursor,
    };
    use crate::RequestId;

    #[test]
    fn expense_month_is_strict() {
        let request_id = RequestId("test".to_owned());
        assert_eq!(
            parse_month("2026-08", &request_id)
                .expect("valid month")
                .to_string(),
            "2026-08-01"
        );
        assert!(parse_month("2026-8", &request_id).is_err());
        assert!(parse_month("2026-13", &request_id).is_err());
        let _ = MonthQuery {
            month: "2026-08".to_owned(),
        };
    }

    #[test]
    fn exact_mutation_headers_require_idempotency_confirmation_and_quoted_version() {
        let request_id = RequestId("test".to_owned());
        let mut headers = HeaderMap::new();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_static("expense-test-1"),
        );
        headers.insert(
            MUTATION_CONFIRM_HEADER,
            HeaderValue::from_static(CONFIRM_RECURRING_UPDATE),
        );
        headers.insert("if-match", HeaderValue::from_static("\"7\""));
        let MutationPreconditions {
            expected_version, ..
        } = mutation_preconditions(
            &headers,
            CONFIRM_RECURRING_UPDATE,
            ExpectedVersion::Exact,
            &request_id,
        )
        .expect("valid mutation headers");
        assert_eq!(expected_version, Some(7));
        assert_eq!(parse_if_match("7"), None);
        assert_eq!(parse_if_match("\"0\""), None);
    }

    #[test]
    fn device_scope_never_allows_import() {
        assert!(!route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/imports"
        ));
        assert!(route_allowed_for_device(
            &axum::http::Method::GET,
            "/api/v1/expenses/summary"
        ));
        assert!(route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/recurring/occurrences/opaque/match"
        ));
        assert!(!route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/recurring/occurrences/opaque/future-operation"
        ));
        assert!(!route_allowed_for_device(
            &axum::http::Method::PATCH,
            "/api/v1/expenses/recurring/occurrences/opaque/future-operation"
        ));
        assert!(!route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/reviews/nested/value/resolve"
        ));
        assert!(route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/reports/report-id/feedback"
        ));
        assert!(!route_allowed_for_device(
            &axum::http::Method::POST,
            "/api/v1/expenses/imports/preview"
        ));
    }

    #[test]
    fn sensitive_text_is_trimmed_and_character_bounded_before_encryption() {
        let request_id = RequestId("test".to_owned());
        assert_eq!(
            normalize_required_text("  정기 구독  ".to_owned(), "name", 120, &request_id)
                .expect("normalize recurring name"),
            "정기 구독"
        );
        assert_eq!(
            normalize_optional_text(Some("   ".to_owned()), "memo", 500, &request_id)
                .expect("normalize empty optional memo"),
            None
        );
        assert!(normalize_required_text("가".repeat(501), "memo", 500, &request_id).is_err());
    }

    #[test]
    fn cursor_accepts_only_the_encoded_rfc3339_and_opaque_id_contract() {
        let request_id = RequestId("test".to_owned());
        let cursor = "2026-08-01T12:34:56.789+09:00|0198f00d-1234-7000-8000-123456789abc";
        assert!(validate_cursor(Some(cursor), &request_id).is_ok());
        assert!(validate_cursor(Some(&format!("{cursor}|extra")), &request_id).is_err());
        assert!(
            validate_cursor(Some("2026-08-01T12:34:56.789 09:00|0198f00d"), &request_id).is_err()
        );
        assert!(validate_cursor(Some(&"a".repeat(513)), &request_id).is_err());
    }

    #[test]
    fn canonical_expense_hashes_match_the_desktop_known_vector() {
        let signed = sign_test_expense_import_value(serde_json::json!({
            "adapter": "kb_card_usage_v1",
            "sourceKind": "card",
            "sourceFingerprint": "11".repeat(32),
            "fileSha256": "22".repeat(32),
            "normalizedSha256": "00".repeat(32),
            "coverageStart": "2026-08-01",
            "coverageEnd": "2026-08-31",
            "rejectedCount": 0,
            "rows": [{
                "stableKey": "row-1",
                "rowSha256": "00".repeat(32),
                "sourceRowNumber": 1,
                "occurredAt": "2026-08-03T09:00:00+09:00",
                "postedDate": "2026-08-03",
                "direction": "debit",
                "amountMinor": 4_500,
                "currency": "KRW",
                "kind": "purchase",
                "categoryHint": "cafe",
                "merchant": "  Example Merchant  ",
                "counterparty": null,
                "memo": null,
                "paymentMethodFingerprint": null,
                "externalReferenceFingerprint": null
            }]
        }));
        assert_eq!(
            signed["rows"][0]["rowSha256"],
            "ab5676fcd2563529627d63b144a4ecd41f3a77223f2c0dfdf7788b0105d068d8"
        );
        assert_eq!(
            signed["normalizedSha256"],
            "18dae05ec5ab243ae4643a74486585e4921c444770275920f503952be09093c8"
        );
    }
}
