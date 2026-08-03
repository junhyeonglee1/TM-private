use std::collections::HashMap;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::State;
use tm_core::{
    CalendarMonth, ConfirmRecurringPaidInput, CreateRecurringExpenseInput, EncryptedExpenseText,
    ExpenseCategory, ExpenseDirection, ExpenseEventKind, ExpenseImportAdapter, ExpenseImportResult,
    ExpenseMutationCommand, ExpenseMutationRequest, ExpenseReview, ExpenseReviewFilter,
    ExpenseReviewPage, ExpenseReviewScope, ExpenseReviewStatus, ExpenseSourceKind,
    ExpenseTransaction, ExpenseTransactionFilter, ExpenseTransactionPage,
    MatchRecurringExpenseInput, NormalizedExpenseImport, NormalizedExpenseRow,
    OverrideExpenseTransactionInput, RecurringAmountKind, RecurringDueRule, RecurringExpenseItem,
    RecurringExpenseOccurrence, RecurringExpenseStatus, ResolveExpenseReviewInput,
    UpdateExpenseSourceStatusInput, UpdateRecurringExpenseInput, expense_text_aad,
};
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{
    AppState,
    cloud_client::DataMode,
    expense_crypto::ExpenseCrypto,
    expense_decryptor::decrypt_ooxml,
    expense_import::{
        ExpenseAdapter, ParsedExpenseFile, ParsedExpenseRow, PreviewExpenseImportInput, hex_sha256,
        parse_expense_file, password_required_preview, verify_source_unchanged,
    },
};

type CommandResult<T> = std::result::Result<T, String>;
const DESKTOP_EXPENSE_ACTOR: &str = "desktop-user";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListExpenseTransactionsInput {
    month: String,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ListExpenseReviewsInput {
    month: Option<String>,
    status: Option<ExpenseReviewStatus>,
    scope: Option<ExpenseReviewScope>,
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlainCreateRecurringExpenseInput {
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PlainUpdateRecurringExpenseInput {
    expected_version: u64,
    effective_from_month: String,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlainNormalizedRow {
    stable_key: String,
    row_sha256: String,
    source_row_number: u32,
    occurred_at: String,
    posted_date: NaiveDate,
    direction: ExpenseDirection,
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

#[derive(Debug, Clone)]
struct PreparedImport {
    adapter: ExpenseImportAdapter,
    source_kind: ExpenseSourceKind,
    source_fingerprint: String,
    file_sha256: String,
    normalized_sha256: String,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    rejected_count: u32,
    rows: Vec<PlainNormalizedRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalPlainNormalizedExpenseRow<'a> {
    occurred_at: &'a str,
    posted_date: NaiveDate,
    direction: ExpenseDirection,
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
    adapter: ExpenseImportAdapter,
    source_kind: ExpenseSourceKind,
    source_fingerprint: &'a str,
    file_sha256: &'a str,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    rejected_count: u32,
    rows: &'a [PlainNormalizedRow],
}

#[tauri::command]
pub(crate) async fn preview_expense_import(
    mut input: PreviewExpenseImportInput,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    let parsed_result = parse_expense_file(&input, decrypt_ooxml);
    if let Some(password) = input.password.as_mut() {
        password.zeroize();
    }
    let parsed = match parsed_result? {
        Some(value) => value,
        None => return serde_json::to_value(password_required_preview()).map_err(command_error),
    };
    let prepared = prepare_import(&parsed)?;
    let counts = if state.cloud.mode() == DataMode::Cloud {
        state
            .cloud
            .clone()
            .preview_expenses(cloud_preview_value(&prepared))
            .await?
    } else {
        let normalized = local_import_value(&prepared, local_crypto(&state)?, None)?;
        serde_json::to_value(
            state
                .core
                .preview_expense_import(&normalized)
                .map_err(command_error)?,
        )
        .map_err(command_error)?
    };
    let session_id = preview_session_id(&counts)?;
    state
        .expense_imports
        .insert(session_id.clone(), parsed.clone())?;
    let mut preview = parsed.preview(session_id);
    preview.counts.new = preview_count(&counts, "newCount")?;
    preview.counts.duplicate = preview_count(&counts, "duplicateCount")?;
    preview.counts.settlement_candidate = preview_count(&counts, "settlementCandidateCount")?;
    preview.counts.unconfirmed = preview_count(&counts, "unconfirmedCount")?;
    preview.counts.rejected = preview_count(&counts, "rejectedCount")?;
    serde_json::to_value(preview).map_err(command_error)
}

#[tauri::command]
pub(crate) async fn commit_expense_import(
    session_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    validate_session_id(&session_id)?;
    let parsed = state.expense_imports.get(&session_id)?;
    verify_source_unchanged(&parsed)?;
    let prepared = prepare_import(&parsed)?;

    let value = if state.cloud.mode() == DataMode::Cloud {
        state
            .cloud
            .clone()
            .import_expenses(cloud_import_value(&prepared, &session_id), &session_id)
            .await?
    } else {
        let crypto = local_crypto(&state)?;
        let idempotency_key = format!("desktop-expense-import:{session_id}");
        let input = local_import_value(&prepared, crypto, Some(&idempotency_key))?;
        let result = execute_local_mutation(
            &state,
            ExpenseMutationCommand::Import {
                preview_session_id: session_id.clone(),
                input,
            },
            Some(idempotency_key),
        )?;
        serde_json::from_value::<ExpenseImportResult>(result.clone()).map_err(command_error)?;
        result
    };
    state.expense_imports.remove(&session_id)?;
    Ok(value)
}

#[tauri::command]
pub(crate) fn get_expense_summary(
    month: String,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    serde_json::to_value(
        state
            .core
            .get_expense_month_summary(parse_month(&month)?)
            .map_err(command_error)?,
    )
    .map_err(command_error)
}

#[tauri::command]
pub(crate) fn list_expense_transactions(
    input: ListExpenseTransactionsInput,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let page = state
        .core
        .list_expense_transactions(ExpenseTransactionFilter {
            month_start: parse_month(&input.month)?,
            cursor: input.cursor,
            limit: input.limit.unwrap_or(50),
        })
        .map_err(command_error)?;
    transaction_page_value(local_crypto(&state)?, page)
}

#[tauri::command]
pub(crate) fn list_expense_sources(state: State<'_, AppState>) -> CommandResult<Value> {
    require_local(&state)?;
    serde_json::to_value(state.core.list_expense_sources().map_err(command_error)?)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn update_expense_source(
    source_id: String,
    input: UpdateExpenseSourceStatusInput,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    execute_local_mutation(
        &state,
        ExpenseMutationCommand::UpdateSource { source_id, input },
        idempotency_key,
    )
}

#[tauri::command]
pub(crate) fn override_expense_transaction(
    event_id: String,
    input: OverrideExpenseTransactionInput,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let result = execute_local_mutation(
        &state,
        ExpenseMutationCommand::OverrideTransaction { event_id, input },
        idempotency_key,
    )?;
    let transaction: ExpenseTransaction = serde_json::from_value(result).map_err(command_error)?;
    transaction_value(local_crypto(&state)?, &transaction)
}

#[tauri::command]
pub(crate) fn list_expense_reviews(
    input: ListExpenseReviewsInput,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let page = state
        .core
        .list_expense_reviews_scoped(
            ExpenseReviewFilter {
                month_start: input.month.as_deref().map(parse_month).transpose()?,
                status: input.status,
                cursor: input.cursor,
                limit: input.limit.unwrap_or(50),
            },
            input.scope.unwrap_or(ExpenseReviewScope::All),
        )
        .map_err(command_error)?;
    review_page_value(local_crypto(&state)?, page)
}

#[tauri::command]
pub(crate) fn resolve_expense_review(
    review_id: String,
    input: ResolveExpenseReviewInput,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let result = execute_local_mutation(
        &state,
        ExpenseMutationCommand::ResolveReview { review_id, input },
        idempotency_key,
    )?;
    let review: ExpenseReview = serde_json::from_value(result).map_err(command_error)?;
    review_value(local_crypto(&state)?, &review)
}

#[tauri::command]
pub(crate) fn list_recurring_expenses(state: State<'_, AppState>) -> CommandResult<Value> {
    require_local(&state)?;
    let crypto = local_crypto(&state)?;
    let values = state
        .core
        .list_recurring_expenses()
        .map_err(command_error)?
        .iter()
        .map(|item| recurring_item_value(crypto, item))
        .collect::<CommandResult<Vec<_>>>()?;
    Ok(Value::Array(values))
}

#[tauri::command]
pub(crate) fn list_recurring_expense_occurrences(
    month: String,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let crypto = local_crypto(&state)?;
    let values = state
        .core
        .recurring_expense_occurrences(parse_month(&month)?)
        .map_err(command_error)?
        .iter()
        .map(|item| recurring_occurrence_value(crypto, item))
        .collect::<CommandResult<Vec<_>>>()?;
    Ok(Value::Array(values))
}

#[tauri::command]
pub(crate) fn create_recurring_expense(
    input: PlainCreateRecurringExpenseInput,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let crypto = local_crypto(&state)?;
    let idempotency_key = local_expense_idempotency_key(idempotency_key)?;
    let recurring_expense_id = format!(
        "recurring-{}",
        &hex_sha256(idempotency_key.as_bytes())[..32]
    );
    let context = format!("recurring:{recurring_expense_id}");
    let command = ExpenseMutationCommand::CreateRecurring(CreateRecurringExpenseInput {
        id: recurring_expense_id,
        name: encrypt_required(
            crypto,
            &context,
            "name",
            &input.name,
            Some(&idempotency_key),
        )?,
        category: input.category,
        vendor: encrypt_optional(
            crypto,
            &context,
            "vendor",
            input.vendor.as_deref(),
            Some(&idempotency_key),
        )?,
        amount_minor: input.amount_minor,
        currency: input.currency,
        payment_method_fingerprint: empty_to_none(input.payment_method_fingerprint),
        start_date: input.start_date,
        end_date: input.end_date,
        memo: encrypt_optional(
            crypto,
            &context,
            "memo",
            input.memo.as_deref(),
            Some(&idempotency_key),
        )?,
        reminder_days: input.reminder_days,
        amount_kind: input.amount_kind,
        interval_months: input.interval_months,
        due_rule: input.due_rule,
        due_day: input.due_day,
        status: input.status,
    });
    let item: RecurringExpenseItem = serde_json::from_value(execute_local_mutation(
        &state,
        command,
        Some(idempotency_key),
    )?)
    .map_err(command_error)?;
    recurring_item_value(crypto, &item)
}

#[tauri::command]
pub(crate) fn update_recurring_expense(
    recurring_expense_id: String,
    input: PlainUpdateRecurringExpenseInput,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let idempotency_key = local_expense_idempotency_key(idempotency_key)?;
    let crypto = local_crypto(&state)?;
    let context = format!("recurring:{recurring_expense_id}");
    let command = ExpenseMutationCommand::UpdateRecurring {
        recurring_expense_id,
        input: UpdateRecurringExpenseInput {
            expected_version: input.expected_version,
            effective_from_month: parse_date(&input.effective_from_month)?,
            name: encrypt_required(
                crypto,
                &context,
                "name",
                &input.name,
                Some(&idempotency_key),
            )?,
            category: input.category,
            vendor: encrypt_optional(
                crypto,
                &context,
                "vendor",
                input.vendor.as_deref(),
                Some(&idempotency_key),
            )?,
            amount_minor: input.amount_minor,
            currency: input.currency,
            payment_method_fingerprint: empty_to_none(input.payment_method_fingerprint),
            start_date: input.start_date,
            end_date: input.end_date,
            memo: encrypt_optional(
                crypto,
                &context,
                "memo",
                input.memo.as_deref(),
                Some(&idempotency_key),
            )?,
            reminder_days: input.reminder_days,
            amount_kind: input.amount_kind,
            interval_months: input.interval_months,
            due_rule: input.due_rule,
            due_day: input.due_day,
            status: input.status,
            auto_match_enabled: input.auto_match_enabled,
        },
    };
    let item: RecurringExpenseItem = serde_json::from_value(execute_local_mutation(
        &state,
        command,
        Some(idempotency_key),
    )?)
    .map_err(command_error)?;
    recurring_item_value(crypto, &item)
}

#[tauri::command]
pub(crate) fn delete_recurring_expense(
    recurring_expense_id: String,
    expected_version: u64,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    execute_local_mutation(
        &state,
        ExpenseMutationCommand::DeleteRecurring {
            recurring_expense_id,
            expected_version,
        },
        idempotency_key,
    )
}

#[tauri::command]
pub(crate) fn confirm_recurring_expense_paid(
    occurrence_key: String,
    amount_minor: Option<i64>,
    paid_date: Option<String>,
    expected_version: u64,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let result = execute_local_mutation(
        &state,
        ExpenseMutationCommand::ConfirmRecurringPaid {
            occurrence_key,
            input: ConfirmRecurringPaidInput {
                expected_version,
                amount_minor,
                paid_date: paid_date.as_deref().map(parse_date).transpose()?,
            },
        },
        idempotency_key,
    )?;
    let item: RecurringExpenseOccurrence = serde_json::from_value(result).map_err(command_error)?;
    recurring_occurrence_value(local_crypto(&state)?, &item)
}

#[tauri::command]
pub(crate) fn match_recurring_expense_occurrence(
    occurrence_key: String,
    event_id: String,
    enable_future_auto_match: bool,
    expected_version: u64,
    idempotency_key: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    require_local(&state)?;
    let result = execute_local_mutation(
        &state,
        ExpenseMutationCommand::MatchRecurring {
            occurrence_key,
            input: MatchRecurringExpenseInput {
                expected_version,
                event_id,
                enable_future_auto_match,
            },
        },
        idempotency_key,
    )?;
    let item: RecurringExpenseOccurrence = serde_json::from_value(result).map_err(command_error)?;
    recurring_occurrence_value(local_crypto(&state)?, &item)
}

#[tauri::command]
pub(crate) fn generate_expense_report(_month: String) -> CommandResult<Value> {
    Err("AI 지출 해설은 TM 클라우드 모드에서만 생성할 수 있습니다.".to_owned())
}

#[tauri::command]
pub(crate) fn latest_expense_report(_month: String) -> CommandResult<Value> {
    Ok(Value::Null)
}

#[tauri::command]
pub(crate) fn rate_expense_report(_report_id: String, _helpful: bool) -> CommandResult<Value> {
    Err("AI 지출 해설 피드백은 TM 클라우드 모드에서만 기록할 수 있습니다.".to_owned())
}

pub(crate) fn calendar_month_value(
    state: &State<'_, AppState>,
    month: CalendarMonth,
) -> CommandResult<Value> {
    let crypto = local_crypto(state)?;
    let occurrence_values = month
        .expense_occurrences
        .iter()
        .map(|item| recurring_occurrence_value(crypto, item))
        .collect::<CommandResult<Vec<_>>>()?;
    let mut value = serde_json::to_value(month).map_err(command_error)?;
    value
        .as_object_mut()
        .ok_or_else(|| "캘린더 응답을 만들 수 없습니다.".to_owned())?
        .insert(
            "expenseOccurrences".to_owned(),
            Value::Array(occurrence_values),
        );
    Ok(value)
}

fn prepare_import(parsed: &ParsedExpenseFile) -> CommandResult<PreparedImport> {
    let adapter = match parsed.adapter {
        ExpenseAdapter::KbCardUsageV1 => ExpenseImportAdapter::KbCardUsageV1,
        ExpenseAdapter::KbAccountHistoryV1 => ExpenseImportAdapter::KbAccountHistoryV1,
        ExpenseAdapter::KakaoPayMoneyV1 => ExpenseImportAdapter::KakaopayMoneyV1,
    };
    let source_kind = match parsed.adapter {
        ExpenseAdapter::KbCardUsageV1 => ExpenseSourceKind::Card,
        ExpenseAdapter::KbAccountHistoryV1 => ExpenseSourceKind::Account,
        ExpenseAdapter::KakaoPayMoneyV1 => ExpenseSourceKind::Wallet,
    };
    let source_fingerprint = hex_sha256(
        format!(
            "tm-expense:source:v1:{}:{}",
            parsed.adapter.as_str(),
            parsed.source_discriminator_fingerprint
        )
        .as_bytes(),
    );
    let mut rows = Vec::with_capacity(parsed.rows.len());
    let mut fingerprint_occurrences = HashMap::<&str, u32>::new();
    for row in &parsed.rows {
        let occurrence = fingerprint_occurrences
            .entry(row.row_fingerprint.as_str())
            .and_modify(|value| *value = value.saturating_add(1))
            .or_insert(1);
        rows.push(prepare_row(parsed.adapter, row, *occurrence)?);
    }
    let rejected_count = u32::try_from(parsed.rejected_rows)
        .map_err(|_| "지출 거부 행 수가 허용 범위를 초과했습니다.".to_owned())?;
    let normalized_sha256 = hex_sha256(
        &serde_json::to_vec(&CanonicalPlainExpenseImport {
            adapter,
            source_kind,
            source_fingerprint: &source_fingerprint,
            file_sha256: &parsed.file_sha256,
            coverage_start: parsed.period_start,
            coverage_end: parsed.period_end,
            rejected_count,
            rows: &rows,
        })
        .map_err(command_error)?,
    );
    Ok(PreparedImport {
        adapter,
        source_kind,
        source_fingerprint,
        file_sha256: parsed.file_sha256.clone(),
        normalized_sha256,
        coverage_start: parsed.period_start,
        coverage_end: parsed.period_end,
        rejected_count,
        rows,
    })
}

fn prepare_row(
    adapter: ExpenseAdapter,
    row: &ParsedExpenseRow,
    occurrence: u32,
) -> CommandResult<PlainNormalizedRow> {
    let occurred_at = if row.occurred_at.ends_with('Z')
        || row
            .occurred_at
            .get(10..)
            .is_some_and(|value| value.contains('+'))
    {
        row.occurred_at.clone()
    } else {
        format!("{}+09:00", row.occurred_at)
    };
    let posted_date = row
        .occurred_at
        .get(..10)
        .ok_or_else(|| "지출 거래 일시가 올바르지 않습니다.".to_owned())
        .and_then(parse_date)?;
    let kind = match row.kind.as_str() {
        "purchase" => ExpenseEventKind::Purchase,
        "refund" => ExpenseEventKind::Refund,
        "card_settlement" => ExpenseEventKind::CardPayment,
        "wallet_topup" => ExpenseEventKind::WalletTopup,
        "wallet_withdrawal" | "transfer_reversal" => ExpenseEventKind::InternalTransfer,
        "p2p_out" | "p2p_in" | "unknown" => ExpenseEventKind::UnknownP2p,
        "bank_out" | "bank_in" => ExpenseEventKind::ExternalTransfer,
        _ => return Err("지원하지 않는 정규화 지출 종류입니다.".to_owned()),
    };
    let direction = match row.direction.as_str() {
        "out" => ExpenseDirection::Debit,
        "in" => ExpenseDirection::Credit,
        _ if matches!(kind, ExpenseEventKind::UnknownP2p) => ExpenseDirection::Debit,
        _ => return Err("지출 입출금 방향이 올바르지 않습니다.".to_owned()),
    };
    let category_hint = match (kind, row.category.as_str()) {
        (ExpenseEventKind::Refund, _) => Some(ExpenseCategory::RefundIncome),
        (
            ExpenseEventKind::CardPayment
            | ExpenseEventKind::WalletTopup
            | ExpenseEventKind::InternalTransfer,
            _,
        ) => Some(ExpenseCategory::TransferSettlement),
        (ExpenseEventKind::UnknownP2p | ExpenseEventKind::ExternalTransfer, _) => {
            Some(ExpenseCategory::Unconfirmed)
        }
        (_, "food") => Some(ExpenseCategory::Food),
        (_, "delivery") => Some(ExpenseCategory::Delivery),
        (_, "cafe") => Some(ExpenseCategory::Cafe),
        (_, "groceries") => Some(ExpenseCategory::Groceries),
        _ => Some(ExpenseCategory::Other),
    };
    let stable_key = format!("{}:{}:{occurrence}", adapter.as_str(), row.row_fingerprint);
    let mut prepared = PlainNormalizedRow {
        stable_key,
        row_sha256: String::new(),
        source_row_number: u32::try_from(row.row_number)
            .map_err(|_| "지출 원본 행 번호가 너무 큽니다.".to_owned())?,
        occurred_at,
        posted_date,
        direction,
        amount_minor: row.amount_minor,
        currency: row.currency.clone(),
        kind,
        category_hint,
        merchant: row.merchant.clone(),
        counterparty: row.counterparty.clone(),
        memo: row.note.clone(),
        payment_method_fingerprint: row.payment_method_fingerprint.clone(),
        external_reference_fingerprint: None,
    };
    prepared.row_sha256 = canonical_expense_row_sha256(&prepared)?;
    Ok(prepared)
}

fn canonical_expense_row_sha256(row: &PlainNormalizedRow) -> CommandResult<String> {
    let canonical = CanonicalPlainNormalizedExpenseRow {
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
    };
    serde_json::to_vec(&canonical)
        .map(|value| hex_sha256(&value))
        .map_err(command_error)
}

fn cloud_preview_value(input: &PreparedImport) -> Value {
    json!({
        "adapter": input.adapter,
        "sourceKind": input.source_kind,
        "sourceFingerprint": input.source_fingerprint,
        "fileSha256": input.file_sha256,
        "normalizedSha256": input.normalized_sha256,
        "coverageStart": input.coverage_start,
        "coverageEnd": input.coverage_end,
        "rejectedCount": input.rejected_count,
        "rows": input.rows,
    })
}

fn cloud_import_value(input: &PreparedImport, preview_session_id: &str) -> Value {
    let mut value = cloud_preview_value(input);
    value
        .as_object_mut()
        .expect("expense import JSON is always an object")
        .insert(
            "previewSessionId".to_owned(),
            Value::String(preview_session_id.to_owned()),
        );
    value
}

fn local_import_value(
    input: &PreparedImport,
    crypto: &ExpenseCrypto,
    idempotency_key: Option<&str>,
) -> CommandResult<NormalizedExpenseImport> {
    let source_fingerprint = crypto.blind_index("source_fingerprint", &input.source_fingerprint)?;
    let file_sha256 = crypto.blind_index("file_digest", &input.file_sha256)?;
    let normalized_sha256 = crypto.blind_index("normalized_digest", &input.normalized_sha256)?;
    let rows = input
        .rows
        .iter()
        .map(|row| {
            let stable_key = crypto.blind_index("stable_key", &row.stable_key)?;
            let context = format!("{source_fingerprint}:{stable_key}");
            Ok(NormalizedExpenseRow {
                stable_key,
                row_sha256: crypto.blind_index("row_digest", &row.row_sha256)?,
                source_row_number: row.source_row_number,
                occurred_at: row.occurred_at.clone(),
                posted_date: row.posted_date,
                direction: row.direction,
                amount_minor: row.amount_minor,
                currency: row.currency.clone(),
                kind: row.kind,
                category_hint: row.category_hint,
                merchant: encrypt_optional(
                    crypto,
                    &context,
                    "merchant",
                    row.merchant.as_deref(),
                    idempotency_key,
                )?,
                counterparty: encrypt_optional(
                    crypto,
                    &context,
                    "counterparty",
                    row.counterparty.as_deref(),
                    idempotency_key,
                )?,
                memo: encrypt_optional(
                    crypto,
                    &context,
                    "memo",
                    row.memo.as_deref(),
                    idempotency_key,
                )?,
                payment_method_fingerprint: row
                    .payment_method_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("payment_method", value))
                    .transpose()?,
                external_reference_fingerprint: row
                    .external_reference_fingerprint
                    .as_deref()
                    .map(|value| crypto.blind_index("external_reference", value))
                    .transpose()?,
            })
        })
        .collect::<CommandResult<Vec<_>>>()?;
    Ok(NormalizedExpenseImport {
        adapter: input.adapter,
        source_kind: input.source_kind,
        source_fingerprint,
        file_sha256,
        normalized_sha256,
        coverage_start: input.coverage_start,
        coverage_end: input.coverage_end,
        rejected_count: input.rejected_count,
        rows,
    })
}

fn preview_session_id(value: &Value) -> CommandResult<String> {
    let session_id = value
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| "지출 가져오기 미리보기 세션을 받지 못했습니다.".to_owned())?;
    validate_session_id(session_id)?;
    Ok(session_id.to_owned())
}

fn execute_local_mutation(
    state: &State<'_, AppState>,
    command: ExpenseMutationCommand,
    idempotency_key: Option<String>,
) -> CommandResult<Value> {
    let idempotency_key = local_expense_idempotency_key(idempotency_key)?;
    state
        .core
        .execute_expense_mutation(ExpenseMutationRequest {
            idempotency_key,
            actor: DESKTOP_EXPENSE_ACTOR.to_owned(),
            command,
        })
        .map(|result| result.value)
        .map_err(command_error)
}

fn local_expense_idempotency_key(value: Option<String>) -> CommandResult<String> {
    let value = value.unwrap_or_else(|| format!("desktop-expense:{}", Uuid::now_v7()));
    if value.is_empty()
        || value.len() > 200
        || value.chars().any(|character| character.is_control())
    {
        return Err("지출 변경 멱등성 키 형식이 올바르지 않습니다.".to_owned());
    }
    Ok(value)
}

fn transaction_page_value(
    crypto: &ExpenseCrypto,
    page: ExpenseTransactionPage,
) -> CommandResult<Value> {
    Ok(json!({
        "items": page.items.iter().map(|item| transaction_value(crypto, item)).collect::<CommandResult<Vec<_>>>()?,
        "nextCursor": page.next_cursor,
    }))
}

fn review_page_value(crypto: &ExpenseCrypto, page: ExpenseReviewPage) -> CommandResult<Value> {
    Ok(json!({
        "items": page.items.iter().map(|item| review_value(crypto, item)).collect::<CommandResult<Vec<_>>>()?,
        "nextCursor": page.next_cursor,
    }))
}

fn transaction_value(crypto: &ExpenseCrypto, item: &ExpenseTransaction) -> CommandResult<Value> {
    let mut value = serde_json::to_value(item).map_err(command_error)?;
    let context = item.crypto_context.as_deref();
    replace_sensitive(
        &mut value,
        "merchant",
        crypto,
        item.merchant.as_ref(),
        context,
    )?;
    replace_sensitive(
        &mut value,
        "counterparty",
        crypto,
        item.counterparty.as_ref(),
        context,
    )?;
    replace_sensitive(&mut value, "memo", crypto, item.memo.as_ref(), context)?;
    Ok(value)
}

fn review_value(crypto: &ExpenseCrypto, item: &ExpenseReview) -> CommandResult<Value> {
    let mut value = serde_json::to_value(item).map_err(command_error)?;
    value
        .as_object_mut()
        .ok_or_else(|| "지출 확인 응답을 만들 수 없습니다.".to_owned())?
        .insert(
            "transaction".to_owned(),
            transaction_value(crypto, &item.transaction)?,
        );
    Ok(value)
}

fn recurring_item_value(
    crypto: &ExpenseCrypto,
    item: &RecurringExpenseItem,
) -> CommandResult<Value> {
    let mut value = serde_json::to_value(item).map_err(command_error)?;
    let context = format!("recurring:{}", item.id);
    replace_sensitive(&mut value, "name", crypto, Some(&item.name), Some(&context))?;
    replace_sensitive(
        &mut value,
        "vendor",
        crypto,
        item.vendor.as_ref(),
        Some(&context),
    )?;
    replace_sensitive(
        &mut value,
        "memo",
        crypto,
        item.memo.as_ref(),
        Some(&context),
    )?;
    Ok(value)
}

fn recurring_occurrence_value(
    crypto: &ExpenseCrypto,
    item: &RecurringExpenseOccurrence,
) -> CommandResult<Value> {
    let mut value = serde_json::to_value(item).map_err(command_error)?;
    let context = format!("recurring:{}", item.recurring_expense_id);
    replace_sensitive(&mut value, "name", crypto, Some(&item.name), Some(&context))?;
    replace_sensitive(
        &mut value,
        "vendor",
        crypto,
        item.vendor.as_ref(),
        Some(&context),
    )?;
    Ok(value)
}

fn replace_sensitive(
    object: &mut Value,
    field: &str,
    crypto: &ExpenseCrypto,
    encrypted: Option<&EncryptedExpenseText>,
    record_context: Option<&str>,
) -> CommandResult<()> {
    let plaintext = encrypted
        .map(|value| {
            let context = record_context
                .ok_or_else(|| "지출 암호문의 레코드 컨텍스트를 확인할 수 없습니다.".to_owned())?;
            crypto.decrypt_text(value, &expense_text_aad(context, field))
        })
        .transpose()?;
    object
        .as_object_mut()
        .ok_or_else(|| "지출 응답을 만들 수 없습니다.".to_owned())?
        .insert(
            field.to_owned(),
            plaintext.map_or(Value::Null, Value::String),
        );
    Ok(())
}

fn encrypt_required(
    crypto: &ExpenseCrypto,
    context: &str,
    field: &str,
    value: &str,
    idempotency_key: Option<&str>,
) -> CommandResult<EncryptedExpenseText> {
    let aad = expense_text_aad(context, field);
    match idempotency_key {
        Some(key) => crypto.encrypt_text_idempotent(field, value, aad, key),
        None => crypto.encrypt_text(field, value, aad),
    }
}

fn encrypt_optional(
    crypto: &ExpenseCrypto,
    context: &str,
    field: &str,
    value: Option<&str>,
    idempotency_key: Option<&str>,
) -> CommandResult<Option<EncryptedExpenseText>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| encrypt_required(crypto, context, field, value, idempotency_key))
        .transpose()
}

fn local_crypto<'a>(state: &'a State<'_, AppState>) -> CommandResult<&'a ExpenseCrypto> {
    state.expense_crypto.as_ref().map_err(Clone::clone)
}

fn require_local(state: &State<'_, AppState>) -> CommandResult<()> {
    if state.cloud.mode() == DataMode::Local {
        Ok(())
    } else {
        Err("로컬 지출 명령은 TM 클라우드 모드에서 사용할 수 없습니다.".to_owned())
    }
}

fn parse_month(value: &str) -> CommandResult<NaiveDate> {
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .map_err(|_| "지출 월은 YYYY-MM 형식이어야 합니다.".to_owned())
}

fn parse_date(value: &str) -> CommandResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| "지출 날짜는 YYYY-MM-DD 형식이어야 합니다.".to_owned())
}

fn validate_session_id(value: &str) -> CommandResult<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| "지출 가져오기 세션 ID가 올바르지 않습니다.".to_owned())
}

fn preview_count(value: &Value, field: &str) -> CommandResult<usize> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok())
        .ok_or_else(|| format!("지출 미리보기 응답에 {field} 값이 없습니다."))
}

fn empty_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
}

fn command_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::{
        CanonicalPlainExpenseImport, ExpenseCategory, ExpenseDirection, ExpenseEventKind,
        ExpenseImportAdapter, ExpenseSourceKind, PlainNormalizedRow, canonical_expense_row_sha256,
        hex_sha256, parse_month, validate_session_id,
    };

    #[test]
    fn validates_month_and_session_boundaries() {
        assert!(parse_month("2026-08").is_ok());
        assert!(parse_month("2026-13").is_err());
        assert!(validate_session_id("not-a-session").is_err());
    }

    #[test]
    fn canonical_expense_hashes_match_the_cloud_known_vector() {
        let date = NaiveDate::from_ymd_opt(2026, 8, 3).expect("valid date");
        let mut row = PlainNormalizedRow {
            stable_key: "row-1".to_owned(),
            row_sha256: String::new(),
            source_row_number: 1,
            occurred_at: "2026-08-03T09:00:00+09:00".to_owned(),
            posted_date: date,
            direction: ExpenseDirection::Debit,
            amount_minor: 4_500,
            currency: "KRW".to_owned(),
            kind: ExpenseEventKind::Purchase,
            category_hint: Some(ExpenseCategory::Cafe),
            merchant: Some("Example Merchant".to_owned()),
            counterparty: None,
            memo: None,
            payment_method_fingerprint: None,
            external_reference_fingerprint: None,
        };
        row.row_sha256 = canonical_expense_row_sha256(&row).expect("hash row");
        assert_eq!(
            row.row_sha256,
            "ab5676fcd2563529627d63b144a4ecd41f3a77223f2c0dfdf7788b0105d068d8"
        );
        let rows = vec![row];
        let normalized = serde_json::to_vec(&CanonicalPlainExpenseImport {
            adapter: ExpenseImportAdapter::KbCardUsageV1,
            source_kind: ExpenseSourceKind::Card,
            source_fingerprint: &"11".repeat(32),
            file_sha256: &"22".repeat(32),
            coverage_start: NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid start"),
            coverage_end: NaiveDate::from_ymd_opt(2026, 8, 31).expect("valid end"),
            rejected_count: 0,
            rows: &rows,
        })
        .expect("encode import");
        assert_eq!(
            hex_sha256(&normalized),
            "18dae05ec5ab243ae4643a74486585e4921c444770275920f503952be09093c8"
        );
    }
}
