use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt,
    str::FromStr,
};

use chrono::{DateTime, Datelike, Duration, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Error, Result, TmCore,
    database::{new_id, now_utc, today_seoul},
    error::{invalid, not_found},
};

const MAX_IMPORT_ROWS: usize = 5_000;
const MAX_PAGE_SIZE: u32 = 200;
const MAX_SAFE_AMOUNT_MINOR: i64 = 9_007_199_254_740_991;
const EXPENSE_AAD_PREFIX: &str = "tm-expense:v1:";
const EXPENSE_CRYPTO_PROBE_AAD: &str = "tm-expense:v1:crypto-probe:value";
const EXPENSE_REPORT_FAILURE_CODES: &[&str] = &[
    "transport",
    "authentication",
    "rate_limited",
    "request_rejected",
    "upstream_unavailable",
    "invalid_response",
    "timeout",
    "persistence_failed",
];

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(Error::Invariant(format!(
                        "unknown {} value: {other}", stringify!($name)
                    ))),
                }
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseImportAdapter {
    KbCardUsageV1,
    KbAccountHistoryV1,
    KakaopayMoneyV1,
}

string_enum!(ExpenseImportAdapter {
    KbCardUsageV1 => "kb_card_usage_v1",
    KbAccountHistoryV1 => "kb_account_history_v1",
    KakaopayMoneyV1 => "kakaopay_money_v1",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseSourceKind {
    Card,
    Account,
    Wallet,
}

string_enum!(ExpenseSourceKind {
    Card => "card",
    Account => "account",
    Wallet => "wallet",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseSourceStatus {
    pub id: String,
    pub adapter: ExpenseImportAdapter,
    pub source_kind: ExpenseSourceKind,
    pub required_for_complete_report: bool,
    pub is_active: bool,
    pub coverage_start: Option<NaiveDate>,
    pub coverage_end: Option<NaiveDate>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateExpenseSourceStatusInput {
    pub expected_version: u64,
    pub required_for_complete_report: bool,
    pub is_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseDirection {
    Debit,
    Credit,
}

string_enum!(ExpenseDirection {
    Debit => "debit",
    Credit => "credit",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseEventKind {
    Purchase,
    Refund,
    CardPayment,
    WalletTopup,
    InternalTransfer,
    SettlementReceived,
    SettlementSent,
    Fee,
    ExternalTransfer,
    UnknownP2p,
    ManualRecurring,
}

string_enum!(ExpenseEventKind {
    Purchase => "purchase",
    Refund => "refund",
    CardPayment => "card_payment",
    WalletTopup => "wallet_topup",
    InternalTransfer => "internal_transfer",
    SettlementReceived => "settlement_received",
    SettlementSent => "settlement_sent",
    Fee => "fee",
    ExternalTransfer => "external_transfer",
    UnknownP2p => "unknown_p2p",
    ManualRecurring => "manual_recurring",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseCategory {
    Food,
    Delivery,
    Cafe,
    Groceries,
    HousingUtilities,
    Transportation,
    OttSubscriptions,
    Shopping,
    Health,
    Leisure,
    Education,
    Travel,
    InsuranceFinanceTax,
    GiftsDues,
    RefundIncome,
    TransferSettlement,
    Other,
    Unconfirmed,
}

string_enum!(ExpenseCategory {
    Food => "food",
    Delivery => "delivery",
    Cafe => "cafe",
    Groceries => "groceries",
    HousingUtilities => "housing_utilities",
    Transportation => "transportation",
    OttSubscriptions => "ott_subscriptions",
    Shopping => "shopping",
    Health => "health",
    Leisure => "leisure",
    Education => "education",
    Travel => "travel",
    InsuranceFinanceTax => "insurance_finance_tax",
    GiftsDues => "gifts_dues",
    RefundIncome => "refund_income",
    TransferSettlement => "transfer_settlement",
    Other => "other",
    Unconfirmed => "unconfirmed",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseEventStatus {
    Confirmed,
    Unconfirmed,
    Excluded,
}

string_enum!(ExpenseEventStatus {
    Confirmed => "confirmed",
    Unconfirmed => "unconfirmed",
    Excluded => "excluded",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseReportStatus {
    Confirmed,
    Provisional,
    Incomplete,
}

string_enum!(ExpenseReportStatus {
    Confirmed => "confirmed",
    Provisional => "provisional",
    Incomplete => "incomplete",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseReviewStatus {
    Pending,
    Resolved,
}

string_enum!(ExpenseReviewStatus {
    Pending => "pending",
    Resolved => "resolved",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseReviewReason {
    UnknownP2p,
    AmbiguousMirror,
    RecurringMatchCandidate,
    RecurringRegistrationCandidate,
    CategoryConfirmation,
    ImportRejected,
    ManualOverride,
}

string_enum!(ExpenseReviewReason {
    UnknownP2p => "unknown_p2p",
    AmbiguousMirror => "ambiguous_mirror",
    RecurringMatchCandidate => "recurring_match_candidate",
    RecurringRegistrationCandidate => "recurring_registration_candidate",
    CategoryConfirmation => "category_confirmation",
    ImportRejected => "import_rejected",
    ManualOverride => "manual_override",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurringAmountKind {
    Fixed,
    Estimate,
    Limit,
}

string_enum!(RecurringAmountKind {
    Fixed => "fixed",
    Estimate => "estimate",
    Limit => "limit",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurringDueRule {
    SpecificDay,
    FirstDay,
    LastDay,
}

string_enum!(RecurringDueRule {
    SpecificDay => "specific_day",
    FirstDay => "first_day",
    LastDay => "last_day",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurringExpenseStatus {
    Active,
    Paused,
    Ended,
}

string_enum!(RecurringExpenseStatus {
    Active => "active",
    Paused => "paused",
    Ended => "ended",
});

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurringOccurrenceStatus {
    Scheduled,
    DueToday,
    DueSoon,
    Overdue,
    Paid,
    Matched,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EncryptedExpenseText {
    pub key_version: u32,
    pub nonce: String,
    pub ciphertext: String,
    pub aad: String,
    pub blind_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseCryptoProbe {
    pub key_version: u32,
    pub nonce: String,
    pub ciphertext: String,
    pub aad: String,
}

/// Builds the stable AAD convention shared by Railway and Windows encryption callers.
/// `record_context` must itself be an opaque identifier or fingerprint, never plaintext.
#[must_use]
pub fn expense_text_aad(record_context: &str, field: &str) -> String {
    format!("{EXPENSE_AAD_PREFIX}{record_context}:{field}")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedExpenseRow {
    pub stable_key: String,
    pub row_sha256: String,
    pub source_row_number: u32,
    pub occurred_at: String,
    pub posted_date: NaiveDate,
    pub direction: ExpenseDirection,
    pub amount_minor: i64,
    pub currency: String,
    pub kind: ExpenseEventKind,
    pub category_hint: Option<ExpenseCategory>,
    pub merchant: Option<EncryptedExpenseText>,
    pub counterparty: Option<EncryptedExpenseText>,
    pub memo: Option<EncryptedExpenseText>,
    pub payment_method_fingerprint: Option<String>,
    pub external_reference_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NormalizedExpenseImport {
    pub adapter: ExpenseImportAdapter,
    pub source_kind: ExpenseSourceKind,
    pub source_fingerprint: String,
    pub file_sha256: String,
    pub normalized_sha256: String,
    pub coverage_start: NaiveDate,
    pub coverage_end: NaiveDate,
    #[serde(default)]
    pub rejected_count: u32,
    pub rows: Vec<NormalizedExpenseRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseImportResult {
    pub batch_id: String,
    pub source_id: String,
    pub row_count: u32,
    pub new_count: u32,
    pub duplicate_count: u32,
    pub rejected_count: u32,
    pub excluded_count: u32,
    pub review_count: u32,
    pub idempotent_replay: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseImportPreview {
    pub session_id: String,
    pub expires_at: String,
    pub items: Vec<NormalizedExpenseRow>,
    pub new_count: u32,
    pub duplicate_count: u32,
    pub settlement_candidate_count: u32,
    pub unconfirmed_count: u32,
    pub rejected_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseImportPreviewRow {
    pub stable_key: String,
    pub row_sha256: String,
    pub source_row_number: u32,
    pub occurred_at: String,
    pub posted_date: NaiveDate,
    pub direction: ExpenseDirection,
    pub amount_minor: i64,
    pub currency: String,
    pub kind: ExpenseEventKind,
    pub category_hint: Option<ExpenseCategory>,
    pub merchant_blind_index: Option<String>,
    pub payment_method_fingerprint: Option<String>,
    pub external_reference_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseImportPreviewInput {
    pub adapter: ExpenseImportAdapter,
    pub source_kind: ExpenseSourceKind,
    pub source_fingerprint: String,
    pub file_sha256: String,
    pub normalized_sha256: String,
    pub coverage_start: NaiveDate,
    pub coverage_end: NaiveDate,
    #[serde(default)]
    pub rejected_count: u32,
    pub rows: Vec<ExpenseImportPreviewRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseTransaction {
    pub id: String,
    pub kind: ExpenseEventKind,
    pub category: ExpenseCategory,
    pub status: ExpenseEventStatus,
    pub amount_minor: i64,
    pub currency: String,
    pub occurred_at: String,
    pub posted_date: NaiveDate,
    pub source_kind: Option<ExpenseSourceKind>,
    pub merchant: Option<EncryptedExpenseText>,
    pub counterparty: Option<EncryptedExpenseText>,
    pub memo: Option<EncryptedExpenseText>,
    pub payment_method_fingerprint: Option<String>,
    pub exclusion_reason: Option<String>,
    pub duplicate_of_event_id: Option<String>,
    pub is_provisional: bool,
    pub pending_review_id: Option<String>,
    pub personal_amount_minor: Option<i64>,
    pub related_event_id: Option<String>,
    pub version: u64,
    /// Opaque local-only context used to authenticate encrypted transaction fields.
    #[serde(skip)]
    pub crypto_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseTransactionFilter {
    pub month_start: NaiveDate,
    pub cursor: Option<String>,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

fn default_page_size() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseTransactionPage {
    pub items: Vec<ExpenseTransaction>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReview {
    pub id: String,
    pub reason: ExpenseReviewReason,
    pub status: ExpenseReviewStatus,
    pub transaction: ExpenseTransaction,
    pub recurring_expense_id: Option<String>,
    pub suggested_kind: Option<ExpenseEventKind>,
    pub suggested_category: Option<ExpenseCategory>,
    pub suggested_duplicate_of_event_id: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseReviewFilter {
    pub month_start: Option<NaiveDate>,
    pub status: Option<ExpenseReviewStatus>,
    pub cursor: Option<String>,
    #[serde(default = "default_page_size")]
    pub limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReviewPage {
    pub items: Vec<ExpenseReview>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolveExpenseReviewInput {
    pub expected_version: u64,
    pub kind: ExpenseEventKind,
    pub category: ExpenseCategory,
    pub duplicate_of_event_id: Option<String>,
    #[serde(default)]
    pub related_event_id: Option<String>,
    #[serde(default)]
    pub personal_amount_minor: Option<i64>,
    #[serde(default)]
    pub create_rule: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OverrideExpenseTransactionInput {
    /// CAS version of the economic event, not a review version.
    pub expected_version: u64,
    pub kind: ExpenseEventKind,
    pub category: ExpenseCategory,
    pub duplicate_of_event_id: Option<String>,
    #[serde(default)]
    pub related_event_id: Option<String>,
    #[serde(default)]
    pub personal_amount_minor: Option<i64>,
    /// `None` preserves an existing split unless this explicit clear flag is set.
    #[serde(default)]
    pub clear_personal_amount: bool,
    /// `None` preserves an existing settlement link unless this explicit clear flag is set.
    #[serde(default)]
    pub clear_related_event: bool,
    #[serde(default)]
    pub create_rule: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseCurrencySummary {
    pub currency: String,
    pub net_personal_spend_minor: i64,
    pub gross_purchase_minor: i64,
    pub refunds_minor: i64,
    pub settlement_received_minor: i64,
    pub settlement_sent_minor: i64,
    pub fees_minor: i64,
    pub unconfirmed_outflow_minor: i64,
    pub recurring_expected_minor: i64,
    pub recurring_paid_minor: i64,
    pub recurring_remaining_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseCategoryTotal {
    pub category: ExpenseCategory,
    pub currency: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseDailyTotal {
    pub date: NaiveDate,
    pub currency: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseDataCompleteness {
    pub active_source_count: u32,
    pub covered_source_count: u32,
    pub pending_review_count: u32,
    pub rejected_row_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseMonthSummary {
    pub month: String,
    pub month_start: NaiveDate,
    pub month_end: NaiveDate,
    pub status: ExpenseReportStatus,
    pub currencies: Vec<ExpenseCurrencySummary>,
    pub categories: Vec<ExpenseCategoryTotal>,
    pub daily: Vec<ExpenseDailyTotal>,
    pub completeness: ExpenseDataCompleteness,
    pub recurring_candidates: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpenseMonthCurrencySnapshot<'a> {
    month_start: NaiveDate,
    month_end: NaiveDate,
    status: ExpenseReportStatus,
    currency: &'a ExpenseCurrencySummary,
    categories: Vec<&'a ExpenseCategoryTotal>,
    daily: Vec<&'a ExpenseDailyTotal>,
    completeness: &'a ExpenseDataCompleteness,
    recurring_candidates: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecurringExpenseItem {
    pub id: String,
    pub name: EncryptedExpenseText,
    pub category: ExpenseCategory,
    pub vendor: Option<EncryptedExpenseText>,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method_fingerprint: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub memo: Option<EncryptedExpenseText>,
    pub reminder_days: u8,
    pub amount_kind: RecurringAmountKind,
    pub interval_months: u8,
    pub due_rule: RecurringDueRule,
    pub due_day: Option<u8>,
    pub status: RecurringExpenseStatus,
    pub auto_match_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateRecurringExpenseInput {
    pub id: String,
    pub name: EncryptedExpenseText,
    pub category: ExpenseCategory,
    pub vendor: Option<EncryptedExpenseText>,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method_fingerprint: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub memo: Option<EncryptedExpenseText>,
    #[serde(default = "default_reminder_days")]
    pub reminder_days: u8,
    pub amount_kind: RecurringAmountKind,
    #[serde(default = "default_interval_months")]
    pub interval_months: u8,
    pub due_rule: RecurringDueRule,
    pub due_day: Option<u8>,
    #[serde(default = "default_recurring_status")]
    pub status: RecurringExpenseStatus,
}

fn default_reminder_days() -> u8 {
    7
}

fn default_interval_months() -> u8 {
    1
}

fn default_recurring_status() -> RecurringExpenseStatus {
    RecurringExpenseStatus::Active
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateRecurringExpenseInput {
    pub expected_version: u64,
    pub effective_from_month: NaiveDate,
    pub name: EncryptedExpenseText,
    pub category: ExpenseCategory,
    pub vendor: Option<EncryptedExpenseText>,
    pub amount_minor: i64,
    pub currency: String,
    pub payment_method_fingerprint: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub memo: Option<EncryptedExpenseText>,
    pub reminder_days: u8,
    pub amount_kind: RecurringAmountKind,
    pub interval_months: u8,
    pub due_rule: RecurringDueRule,
    pub due_day: Option<u8>,
    pub status: RecurringExpenseStatus,
    /// Existing consent may be revoked here; enabling still requires a confirmed manual match.
    pub auto_match_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecurringExpenseOccurrence {
    pub occurrence_key: String,
    pub recurring_expense_id: String,
    pub item_version: u64,
    pub name: EncryptedExpenseText,
    pub category: ExpenseCategory,
    pub vendor: Option<EncryptedExpenseText>,
    pub due_date: NaiveDate,
    pub expected_amount_minor: i64,
    pub actual_amount_minor: Option<i64>,
    pub currency: String,
    pub status: RecurringOccurrenceStatus,
    pub reminder_days: u8,
    pub amount_changed: bool,
    pub actual_event_id: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmRecurringPaidInput {
    pub expected_version: u64,
    pub amount_minor: Option<i64>,
    pub paid_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MatchRecurringExpenseInput {
    pub expected_version: u64,
    pub event_id: String,
    #[serde(default)]
    pub enable_future_auto_match: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReportFact {
    pub fact_id: String,
    pub metric: String,
    pub currency: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReportAggregate {
    pub month_start: NaiveDate,
    pub aggregate_sha256: String,
    pub report_status: ExpenseReportStatus,
    pub facts: Vec<ExpenseReportFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseReportObservation {
    pub text: String,
    pub fact_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReportResult {
    pub id: String,
    pub month_start: NaiveDate,
    pub aggregate_sha256: String,
    pub prompt_version: String,
    pub model: String,
    pub title: String,
    pub summary: String,
    pub facts: Vec<ExpenseReportFact>,
    pub observations: Vec<ExpenseReportObservation>,
    pub alerts: Vec<ExpenseReportObservation>,
    pub next_month_checks: Vec<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: u64,
    pub latency_ms: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveExpenseReportInput {
    pub month_start: NaiveDate,
    pub aggregate_sha256: String,
    pub prompt_version: String,
    pub model: String,
    pub title: String,
    pub summary: String,
    pub facts: Vec<ExpenseReportFact>,
    pub observations: Vec<ExpenseReportObservation>,
    pub alerts: Vec<ExpenseReportObservation>,
    pub next_month_checks: Vec<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cost_microusd: u64,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReportAttemptClaim {
    pub request_id: String,
    /// Seoul-calendar month used for the eight-attempt quota.
    pub month_start: NaiveDate,
    /// Month whose aggregate the model result describes.
    pub report_month_start: NaiveDate,
    pub attempt_number: u8,
    pub status: ExpenseReportAttemptStatus,
    pub failure_code: Option<String>,
    pub replayed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseReportAttemptStatus {
    Claimed,
    Succeeded,
    Failed,
}

string_enum!(ExpenseReportAttemptStatus {
    Claimed => "claimed",
    Succeeded => "succeeded",
    Failed => "failed",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseReportRequestBinding {
    pub request_id: String,
    pub report_month_start: NaiveDate,
    pub aggregate_sha256: String,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "operation", content = "payload", rename_all = "snake_case")]
pub enum ExpenseMutationCommand {
    InitializeCryptoProbe(ExpenseCryptoProbe),
    PreviewImport(ExpenseImportPreviewInput),
    Import {
        preview_session_id: String,
        input: NormalizedExpenseImport,
    },
    UpdateSource {
        source_id: String,
        input: UpdateExpenseSourceStatusInput,
    },
    ResolveReview {
        review_id: String,
        input: ResolveExpenseReviewInput,
    },
    OverrideTransaction {
        event_id: String,
        input: OverrideExpenseTransactionInput,
    },
    CreateRecurring(CreateRecurringExpenseInput),
    UpdateRecurring {
        recurring_expense_id: String,
        input: UpdateRecurringExpenseInput,
    },
    DeleteRecurring {
        recurring_expense_id: String,
        expected_version: u64,
    },
    ConfirmRecurringPaid {
        occurrence_key: String,
        input: ConfirmRecurringPaidInput,
    },
    MatchRecurring {
        occurrence_key: String,
        input: MatchRecurringExpenseInput,
    },
    SaveReport(SaveExpenseReportInput),
    RecordReportFeedback {
        report_id: String,
        helpful: bool,
    },
}

impl ExpenseMutationCommand {
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        match self {
            Self::InitializeCryptoProbe(_) => "expense.crypto_probe.initialize",
            Self::PreviewImport(_) => "expense.import.preview",
            Self::Import { .. } => "expense.import",
            Self::UpdateSource { .. } => "expense.source.update",
            Self::ResolveReview { .. } => "expense.review.resolve",
            Self::OverrideTransaction { .. } => "expense.transaction.override",
            Self::CreateRecurring(_) => "expense.recurring.create",
            Self::UpdateRecurring { .. } => "expense.recurring.update",
            Self::DeleteRecurring { .. } => "expense.recurring.delete",
            Self::ConfirmRecurringPaid { .. } => "expense.recurring.confirm_paid",
            Self::MatchRecurring { .. } => "expense.recurring.match",
            Self::SaveReport(_) => "expense.report.save",
            Self::RecordReportFeedback { .. } => "expense.report.feedback",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpenseMutationRequest {
    pub idempotency_key: String,
    pub actor: String,
    pub command: ExpenseMutationCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseMutationResult {
    pub operation: String,
    pub replayed: bool,
    pub value: Value,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseKeyInitializationStatus {
    pub ledger_empty: bool,
    pub key_initialized: bool,
    pub key_initialization_allowed: bool,
}

impl TmCore {
    pub fn expense_crypto_probe(&self) -> Result<Option<ExpenseCryptoProbe>> {
        self.database
            .connect()?
            .query_row(
                "SELECT key_version, nonce, ciphertext, aad
                 FROM expense_crypto_metadata
                 WHERE singleton_key = 'expense-data-key-probe'",
                [],
                |row| {
                    Ok(ExpenseCryptoProbe {
                        key_version: row.get(0)?,
                        nonce: row.get(1)?,
                        ciphertext: row.get(2)?,
                        aad: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn has_encrypted_expense_payloads(&self) -> Result<bool> {
        let connection = self.database.connect()?;
        has_encrypted_expense_payloads_in_connection(&connection)
    }

    /// Returns a read-only, fail-closed proof used before the first production
    /// expense key is created. A new key is safe only when there is neither a
    /// previously initialized key probe nor any durable expense state.
    pub fn expense_key_initialization_status(&self) -> Result<ExpenseKeyInitializationStatus> {
        let connection = self.database.connect()?;
        let (key_initialized, has_ledger_state) = connection.query_row(
            "SELECT
                EXISTS(
                    SELECT 1 FROM expense_crypto_metadata
                    WHERE singleton_key = 'expense-data-key-probe'
                ),
                EXISTS(
                    SELECT 1 FROM expense_sources
                    UNION ALL SELECT 1 FROM expense_import_batches
                    UNION ALL SELECT 1 FROM expense_import_preview_sessions
                    UNION ALL SELECT 1 FROM expense_raw_rows
                    UNION ALL SELECT 1 FROM expense_postings
                    UNION ALL SELECT 1 FROM expense_events
                    UNION ALL SELECT 1 FROM expense_event_postings
                    UNION ALL SELECT 1 FROM expense_allocations
                    UNION ALL SELECT 1 FROM expense_reviews
                    UNION ALL SELECT 1 FROM expense_rules
                    UNION ALL SELECT 1 FROM recurring_expense_items
                    UNION ALL SELECT 1 FROM recurring_expense_versions
                    UNION ALL SELECT 1 FROM recurring_expense_occurrences
                    UNION ALL SELECT 1 FROM expense_month_reports
                    UNION ALL SELECT 1 FROM expense_ai_reports
                    UNION ALL SELECT 1 FROM expense_ai_feedback
                    UNION ALL SELECT 1 FROM expense_ai_request_bindings
                    UNION ALL SELECT 1 FROM expense_ai_attempts
                    UNION ALL SELECT 1 FROM expense_mutation_receipts
                )",
            [],
            |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
        )?;
        let ledger_empty = !has_ledger_state;
        Ok(ExpenseKeyInitializationStatus {
            ledger_empty,
            key_initialized,
            key_initialization_allowed: ledger_empty && !key_initialized,
        })
    }

    pub fn initialize_expense_crypto_probe(
        &self,
        probe: ExpenseCryptoProbe,
    ) -> Result<ExpenseCryptoProbe> {
        validate_crypto_probe(&probe)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                initialize_crypto_probe_in_transaction(transaction, &probe)
            })
    }

    /// Atomically refuses first-time key initialization when encrypted payloads already exist.
    pub fn initialize_expense_crypto_probe_if_ledger_empty(
        &self,
        probe: ExpenseCryptoProbe,
    ) -> Result<ExpenseCryptoProbe> {
        self.initialize_expense_crypto_probe(probe)
    }

    pub fn execute_expense_mutation(
        &self,
        request: ExpenseMutationRequest,
    ) -> Result<ExpenseMutationResult> {
        validate_label("expense idempotency key", &request.idempotency_key, 200)?;
        validate_label("expense mutation actor", &request.actor, 128)?;
        validate_expense_mutation_command(&request.command)?;
        let operation = request.command.operation();
        let request_sha256 = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&request.command)?)
        );
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some((stored_operation, stored_sha256, response_json)) = transaction
                    .query_row(
                        "SELECT operation, request_sha256, response_json
                         FROM expense_mutation_receipts WHERE idempotency_key = ?1",
                        [&request.idempotency_key],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?
                {
                    if stored_operation != operation || stored_sha256 != request_sha256 {
                        return Err(Error::Conflict(
                            "expense idempotency key was already used for another mutation"
                                .to_owned(),
                        ));
                    }
                    let mut result: ExpenseMutationResult = serde_json::from_str(&response_json)?;
                    result.replayed = true;
                    return Ok(result);
                }

                let value = execute_expense_mutation_command(transaction, &request.command)?;
                refresh_after_expense_mutation(transaction, &request.command)?;
                let result = ExpenseMutationResult {
                    operation: operation.to_owned(),
                    replayed: false,
                    value,
                };
                transaction.execute(
                    "INSERT INTO expense_mutation_receipts(
                        idempotency_key, operation, request_sha256, actor,
                        response_json, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        request.idempotency_key,
                        operation,
                        request_sha256,
                        request.actor,
                        serde_json::to_string(&result)?,
                        now_utc(),
                    ],
                )?;
                Ok(result)
            })
    }

    pub fn import_expenses(
        &self,
        preview_session_id: &str,
        input: NormalizedExpenseImport,
    ) -> Result<ExpenseImportResult> {
        validate_label("expense import preview session ID", preview_session_id, 128)?;
        validate_normalized_import(&input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                consume_expense_preview_session(transaction, preview_session_id, &input)?;
                let result = import_expenses_in_transaction(transaction, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::Import {
                        preview_session_id: preview_session_id.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }

    pub fn list_expense_sources(&self) -> Result<Vec<ExpenseSourceStatus>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, adapter, source_kind, required_for_complete_report,
                    is_active, coverage_start, coverage_end, version
             FROM expense_sources ORDER BY created_at, id",
        )?;
        statement
            .query_map([], map_expense_source_status)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn update_expense_source_status(
        &self,
        source_id: &str,
        input: UpdateExpenseSourceStatusInput,
    ) -> Result<ExpenseSourceStatus> {
        validate_expense_source_status_update(source_id, &input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result =
                    update_expense_source_status_in_transaction(transaction, source_id, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::UpdateSource {
                        source_id: source_id.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }

    /// Compares normalized stable row identities and creates a ten-minute commit receipt.
    pub fn preview_expense_import(
        &self,
        input: &NormalizedExpenseImport,
    ) -> Result<ExpenseImportPreview> {
        validate_normalized_import(input)?;
        let redacted = ExpenseImportPreviewInput {
            adapter: input.adapter,
            source_kind: input.source_kind,
            source_fingerprint: input.source_fingerprint.clone(),
            file_sha256: input.file_sha256.clone(),
            normalized_sha256: input.normalized_sha256.clone(),
            coverage_start: input.coverage_start,
            coverage_end: input.coverage_end,
            rejected_count: input.rejected_count,
            rows: input
                .rows
                .iter()
                .map(|row| ExpenseImportPreviewRow {
                    stable_key: row.stable_key.clone(),
                    row_sha256: row.row_sha256.clone(),
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
                        .as_ref()
                        .map(|value| value.blind_index.clone()),
                    payment_method_fingerprint: row.payment_method_fingerprint.clone(),
                    external_reference_fingerprint: row.external_reference_fingerprint.clone(),
                })
                .collect(),
        };
        let mut preview = self.preview_expense_import_redacted(&redacted)?;
        preview.items = input.rows.iter().take(100).cloned().collect();
        Ok(preview)
    }

    /// Server-safe preview accepting only fingerprints and deterministic accounting fields.
    pub fn preview_expense_import_redacted(
        &self,
        input: &ExpenseImportPreviewInput,
    ) -> Result<ExpenseImportPreview> {
        validate_expense_preview_input(input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                Self::preview_expense_import_redacted_in_connection(transaction, input)
            })
    }

    fn preview_expense_import_redacted_in_connection(
        connection: &Connection,
        input: &ExpenseImportPreviewInput,
    ) -> Result<ExpenseImportPreview> {
        let source_id = connection
            .query_row(
                "SELECT id FROM expense_sources WHERE source_fingerprint = ?1",
                [&input.source_fingerprint],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some((saved_sha256,)) = source_id
            .as_deref()
            .map(|source_id| {
                connection
                    .query_row(
                        "SELECT normalized_sha256 FROM expense_import_batches
                         WHERE source_id = ?1 AND file_sha256 = ?2",
                        params![source_id, input.file_sha256],
                        |row| Ok((row.get::<_, String>(0)?,)),
                    )
                    .optional()
            })
            .transpose()?
            .flatten()
        {
            if saved_sha256 != input.normalized_sha256 {
                return Err(Error::Conflict(
                    "expense file fingerprint was replayed with different normalized content"
                        .to_owned(),
                ));
            }
        }

        let mut new_count = 0_u32;
        let mut duplicate_count = 0_u32;
        let mut settlement_candidate_count = 0_u32;
        let mut unconfirmed_count = 0_u32;
        for row in &input.rows {
            let stable_existing = source_id
                .as_deref()
                .map(|source_id| {
                    connection
                        .query_row(
                            "SELECT row_sha256 FROM expense_raw_rows
                             WHERE source_id = ?1 AND stable_key = ?2",
                            params![source_id, row.stable_key],
                            |record| record.get::<_, String>(0),
                        )
                        .optional()
                })
                .transpose()?
                .flatten();
            if let Some(existing_sha256) = stable_existing {
                if existing_sha256 != row.row_sha256 {
                    return Err(Error::Conflict(format!(
                        "expense stable row key was replayed with different content: {}",
                        row.stable_key
                    )));
                }
                duplicate_count = duplicate_count.saturating_add(1);
                continue;
            }
            let content_duplicate =
                if let Some(reference) = row.external_reference_fingerprint.as_deref() {
                    connection.query_row(
                        "SELECT EXISTS(
                        SELECT 1 FROM expense_postings
                        WHERE external_reference_fingerprint = ?1
                     )",
                        [reference],
                        |record| record.get::<_, bool>(0),
                    )?
                } else {
                    false
                };
            if content_duplicate {
                duplicate_count = duplicate_count.saturating_add(1);
                continue;
            }
            new_count = new_count.saturating_add(1);
            if matches!(
                row.kind,
                ExpenseEventKind::SettlementReceived
                    | ExpenseEventKind::SettlementSent
                    | ExpenseEventKind::UnknownP2p
                    | ExpenseEventKind::ExternalTransfer
            ) {
                settlement_candidate_count = settlement_candidate_count.saturating_add(1);
            }
            if matches!(
                row.kind,
                ExpenseEventKind::UnknownP2p | ExpenseEventKind::ExternalTransfer
            ) {
                unconfirmed_count = unconfirmed_count.saturating_add(1);
            }
        }
        let session_id = new_id();
        let created_at = now_utc();
        let expires_at =
            (Utc::now() + Duration::minutes(10)).to_rfc3339_opts(SecondsFormat::Millis, true);
        let expired_retention_cutoff =
            (Utc::now() - Duration::hours(24)).to_rfc3339_opts(SecondsFormat::Millis, true);
        connection.execute(
            "DELETE FROM expense_import_preview_sessions
             WHERE consumed_at IS NOT NULL OR expires_at < ?1",
            [expired_retention_cutoff],
        )?;
        let row_count = input
            .rows
            .len()
            .checked_add(input.rejected_count as usize)
            .and_then(|count| u32::try_from(count).ok())
            .ok_or_else(|| invalid("expense import preview row count overflow"))?;
        let content_sha256 = canonical_preview_content_sha256(input)?;
        connection.execute(
            "INSERT INTO expense_import_preview_sessions(
                id, adapter, source_kind, source_fingerprint, file_sha256,
                normalized_sha256, content_sha256, coverage_start, coverage_end,
                row_count, rejected_count, expires_at, created_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
             )",
            params![
                session_id,
                input.adapter.as_str(),
                input.source_kind.as_str(),
                input.source_fingerprint,
                input.file_sha256,
                input.normalized_sha256,
                content_sha256,
                input.coverage_start,
                input.coverage_end,
                row_count,
                input.rejected_count,
                expires_at,
                created_at,
            ],
        )?;
        Ok(ExpenseImportPreview {
            session_id,
            expires_at,
            items: Vec::new(),
            new_count,
            duplicate_count,
            settlement_candidate_count,
            unconfirmed_count,
            rejected_count: input.rejected_count,
        })
    }

    pub fn list_expense_transactions(
        &self,
        filter: ExpenseTransactionFilter,
    ) -> Result<ExpenseTransactionPage> {
        validate_month_start(filter.month_start)?;
        let limit = validate_page_limit(filter.limit)?;
        let month_end = next_month(filter.month_start)?;
        let (cursor_at, cursor_id) = parse_cursor(filter.cursor.as_deref())?;
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT e.id, e.event_kind, e.category, e.event_status,
                    e.amount_minor, e.currency, e.occurred_at, e.posted_date,
                    s.source_kind, s.source_fingerprint, raw.stable_key,
                    p.merchant_key_version, p.merchant_nonce, p.merchant_ciphertext,
                    p.merchant_aad, p.merchant_blind_index,
                    p.counterparty_key_version, p.counterparty_nonce,
                    p.counterparty_ciphertext, p.counterparty_aad,
                    p.counterparty_blind_index,
                    p.memo_key_version, p.memo_nonce, p.memo_ciphertext,
                    p.memo_aad, p.memo_blind_index,
                    p.payment_method_fingerprint, e.exclusion_reason,
                    e.duplicate_of_event_id, e.is_provisional,
                    (SELECT r.id FROM expense_reviews AS r
                     WHERE r.event_id = e.id AND r.review_status = 'pending'
                     ORDER BY CASE r.review_reason
                         WHEN 'unknown_p2p' THEN 0
                         WHEN 'ambiguous_mirror' THEN 1
                         WHEN 'import_rejected' THEN 2
                         WHEN 'category_confirmation' THEN 3
                         WHEN 'recurring_match_candidate' THEN 4
                         WHEN 'recurring_registration_candidate' THEN 5
                         ELSE 6 END,
                         r.created_at, r.id
                     LIMIT 1),
                    (SELECT a.amount_minor FROM expense_allocations AS a
                     WHERE a.event_id = e.id AND a.allocation_kind = 'personal'),
                    (SELECT a.related_event_id FROM expense_allocations AS a
                     WHERE a.event_id = e.id
                       AND a.allocation_kind IN (
                           'settlement_received', 'settlement_sent'
                       )
                     LIMIT 1),
                    e.version
             FROM expense_events AS e
             LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
             LEFT JOIN expense_raw_rows AS raw ON raw.id = p.raw_row_id
             LEFT JOIN expense_sources AS s ON s.id = p.source_id
             WHERE e.posted_date >= ?1 AND e.posted_date < ?2
               AND (?3 IS NULL OR e.occurred_at < ?3
                    OR (e.occurred_at = ?3 AND e.id < ?4))
             ORDER BY e.occurred_at DESC, e.id DESC
             LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                filter.month_start,
                month_end,
                cursor_at,
                cursor_id,
                i64::from(limit + 1),
            ],
            map_expense_transaction,
        )?;
        let mut items = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = items.len() > limit as usize;
        if has_more {
            items.pop();
        }
        let next_cursor = has_more
            .then(|| items.last().map(expense_transaction_cursor))
            .flatten();
        Ok(ExpenseTransactionPage { items, next_cursor })
    }

    pub fn list_expense_reviews(&self, filter: ExpenseReviewFilter) -> Result<ExpenseReviewPage> {
        if let Some(month) = filter.month_start {
            validate_month_start(month)?;
        }
        let limit = validate_page_limit(filter.limit)?;
        let month_end = filter.month_start.map(next_month).transpose()?;
        let (cursor_at, cursor_id) = parse_cursor(filter.cursor.as_deref())?;
        let status = filter.status.map(ExpenseReviewStatus::as_str);
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT r.id, r.event_id, r.review_reason, r.review_status,
                    r.recurring_expense_id, r.suggested_kind,
                    r.suggested_category, r.suggested_duplicate_of_event_id,
                    r.created_at, r.resolved_at, r.version
             FROM expense_reviews AS r
             JOIN expense_events AS e ON e.id = r.event_id
             WHERE (?1 IS NULL OR (e.posted_date >= ?1 AND e.posted_date < ?2))
               AND (?3 IS NULL OR r.review_status = ?3)
               AND (?4 IS NULL OR r.created_at < ?4
                    OR (r.created_at = ?4 AND r.id < ?5))
             ORDER BY r.created_at DESC, r.id DESC
             LIMIT ?6",
        )?;
        let rows = statement.query_map(
            params![
                filter.month_start,
                month_end,
                status,
                cursor_at,
                cursor_id,
                i64::from(limit + 1),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, u64>(10)?,
                ))
            },
        )?;
        let raw = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let has_more = raw.len() > limit as usize;
        let mut items = Vec::with_capacity(raw.len().min(limit as usize));
        for (
            id,
            event_id,
            reason,
            review_status,
            recurring_expense_id,
            suggested_kind,
            suggested_category,
            suggested_duplicate_of_event_id,
            created_at,
            resolved_at,
            version,
        ) in raw.into_iter().take(limit as usize)
        {
            items.push(ExpenseReview {
                id,
                reason: ExpenseReviewReason::from_str(&reason)?,
                status: ExpenseReviewStatus::from_str(&review_status)?,
                transaction: query_expense_transaction(&connection, &event_id)?,
                recurring_expense_id,
                suggested_kind: suggested_kind
                    .map(|value| ExpenseEventKind::from_str(&value))
                    .transpose()?,
                suggested_category: suggested_category
                    .map(|value| ExpenseCategory::from_str(&value))
                    .transpose()?,
                suggested_duplicate_of_event_id,
                created_at,
                resolved_at,
                version,
            });
        }
        let next_cursor = has_more
            .then(|| {
                items
                    .last()
                    .map(|review| encode_cursor(&review.created_at, &review.id))
            })
            .flatten();
        Ok(ExpenseReviewPage { items, next_cursor })
    }

    pub fn resolve_expense_review(
        &self,
        review_id: &str,
        input: ResolveExpenseReviewInput,
    ) -> Result<ExpenseReview> {
        if input.expected_version == 0 {
            return Err(invalid("expense review expected version must be positive"));
        }
        validate_review_resolution(&input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                resolve_expense_review_in_transaction(transaction, review_id, &input, false)?;
                let result = query_expense_review(transaction, review_id)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::ResolveReview {
                        review_id: review_id.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }

    pub fn override_expense_transaction(
        &self,
        event_id: &str,
        input: OverrideExpenseTransactionInput,
    ) -> Result<ExpenseTransaction> {
        validate_expense_transaction_override(event_id, &input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result =
                    override_expense_transaction_in_transaction(transaction, event_id, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::OverrideTransaction {
                        event_id: event_id.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }
}

fn map_expense_source_status(row: &Row<'_>) -> rusqlite::Result<ExpenseSourceStatus> {
    Ok(ExpenseSourceStatus {
        id: row.get(0)?,
        adapter: parse_db_enum(row.get::<_, String>(1)?, 1)?,
        source_kind: parse_db_enum(row.get::<_, String>(2)?, 2)?,
        required_for_complete_report: row.get(3)?,
        is_active: row.get(4)?,
        coverage_start: row.get(5)?,
        coverage_end: row.get(6)?,
        version: row.get(7)?,
    })
}

fn query_expense_source_status(
    connection: &Connection,
    source_id: &str,
) -> Result<ExpenseSourceStatus> {
    connection
        .query_row(
            "SELECT id, adapter, source_kind, required_for_complete_report,
                    is_active, coverage_start, coverage_end, version
             FROM expense_sources WHERE id = ?1",
            [source_id],
            map_expense_source_status,
        )
        .optional()?
        .ok_or_else(|| not_found("expense source", source_id))
}

fn update_expense_source_status_in_transaction(
    transaction: &Transaction<'_>,
    source_id: &str,
    input: &UpdateExpenseSourceStatusInput,
) -> Result<ExpenseSourceStatus> {
    let current = query_expense_source_status(transaction, source_id)?;
    if current.version != input.expected_version {
        return Err(Error::Conflict(format!(
            "expense source {source_id} changed from version {} to {}",
            input.expected_version, current.version
        )));
    }
    let changed = transaction.execute(
        "UPDATE expense_sources
         SET required_for_complete_report = ?2, is_active = ?3,
             updated_at = ?4, version = version + 1
         WHERE id = ?1 AND version = ?5",
        params![
            source_id,
            input.required_for_complete_report,
            input.is_active,
            now_utc(),
            input.expected_version,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "expense source {source_id} changed before the update completed"
        )));
    }
    query_expense_source_status(transaction, source_id)
}

fn initialize_crypto_probe_in_transaction(
    transaction: &Transaction<'_>,
    probe: &ExpenseCryptoProbe,
) -> Result<ExpenseCryptoProbe> {
    let existing = transaction
        .query_row(
            "SELECT key_version, nonce, ciphertext, aad
             FROM expense_crypto_metadata
             WHERE singleton_key = 'expense-data-key-probe'",
            [],
            |row| {
                Ok(ExpenseCryptoProbe {
                    key_version: row.get(0)?,
                    nonce: row.get(1)?,
                    ciphertext: row.get(2)?,
                    aad: row.get(3)?,
                })
            },
        )
        .optional()?;
    if let Some(existing) = existing {
        if existing == *probe {
            return Ok(existing);
        }
        return Err(Error::Conflict(
            "expense encryption probe is already initialized".to_owned(),
        ));
    }
    if has_any_expense_ledger_state_in_connection(transaction)? {
        return Err(Error::Conflict(
            "expense encryption probe cannot be initialized after expense ledger state exists"
                .to_owned(),
        ));
    }
    let now = now_utc();
    transaction.execute(
        "INSERT INTO expense_crypto_metadata(
            singleton_key, key_version, nonce, ciphertext, aad,
            created_at, updated_at
         ) VALUES ('expense-data-key-probe', ?1, ?2, ?3, ?4, ?5, ?5)",
        params![
            probe.key_version,
            probe.nonce,
            probe.ciphertext,
            probe.aad,
            now,
        ],
    )?;
    Ok(probe.clone())
}

fn has_encrypted_expense_payloads_in_connection(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT
                EXISTS(
                    SELECT 1 FROM expense_postings
                    WHERE merchant_key_version IS NOT NULL
                       OR counterparty_key_version IS NOT NULL
                       OR memo_key_version IS NOT NULL
                )
                OR EXISTS(SELECT 1 FROM recurring_expense_items)
                OR EXISTS(
                    SELECT 1 FROM expense_allocations
                    WHERE counterparty_key_version IS NOT NULL
                )",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn has_any_expense_ledger_state_in_connection(connection: &Connection) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM expense_sources
                UNION ALL SELECT 1 FROM expense_import_batches
                UNION ALL SELECT 1 FROM expense_import_preview_sessions
                UNION ALL SELECT 1 FROM expense_raw_rows
                UNION ALL SELECT 1 FROM expense_postings
                UNION ALL SELECT 1 FROM expense_events
                UNION ALL SELECT 1 FROM expense_event_postings
                UNION ALL SELECT 1 FROM expense_allocations
                UNION ALL SELECT 1 FROM expense_reviews
                UNION ALL SELECT 1 FROM expense_rules
                UNION ALL SELECT 1 FROM recurring_expense_items
                UNION ALL SELECT 1 FROM recurring_expense_versions
                UNION ALL SELECT 1 FROM recurring_expense_occurrences
                UNION ALL SELECT 1 FROM expense_month_reports
                UNION ALL SELECT 1 FROM expense_ai_reports
                UNION ALL SELECT 1 FROM expense_ai_feedback
                UNION ALL SELECT 1 FROM expense_ai_request_bindings
                UNION ALL SELECT 1 FROM expense_ai_attempts
                UNION ALL SELECT 1 FROM expense_mutation_receipts
             )",
            [],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn execute_expense_mutation_command(
    transaction: &Transaction<'_>,
    command: &ExpenseMutationCommand,
) -> Result<Value> {
    match command {
        ExpenseMutationCommand::InitializeCryptoProbe(probe) => {
            serde_json::to_value(initialize_crypto_probe_in_transaction(transaction, probe)?)
                .map_err(Into::into)
        }
        ExpenseMutationCommand::PreviewImport(input) => serde_json::to_value(
            TmCore::preview_expense_import_redacted_in_connection(transaction, input)?,
        )
        .map_err(Into::into),
        ExpenseMutationCommand::Import {
            preview_session_id,
            input,
        } => {
            consume_expense_preview_session(transaction, preview_session_id, input)?;
            serde_json::to_value(import_expenses_in_transaction(transaction, input)?)
                .map_err(Into::into)
        }
        ExpenseMutationCommand::UpdateSource { source_id, input } => serde_json::to_value(
            update_expense_source_status_in_transaction(transaction, source_id, input)?,
        )
        .map_err(Into::into),
        ExpenseMutationCommand::ResolveReview { review_id, input } => {
            resolve_expense_review_in_transaction(transaction, review_id, input, false)?;
            serde_json::to_value(query_expense_review(transaction, review_id)?).map_err(Into::into)
        }
        ExpenseMutationCommand::OverrideTransaction { event_id, input } => serde_json::to_value(
            override_expense_transaction_in_transaction(transaction, event_id, input)?,
        )
        .map_err(Into::into),
        ExpenseMutationCommand::CreateRecurring(input) => {
            serde_json::to_value(create_recurring_in_transaction(transaction, input)?)
                .map_err(Into::into)
        }
        ExpenseMutationCommand::UpdateRecurring {
            recurring_expense_id,
            input,
        } => serde_json::to_value(update_recurring_in_transaction(
            transaction,
            recurring_expense_id,
            input,
        )?)
        .map_err(Into::into),
        ExpenseMutationCommand::DeleteRecurring {
            recurring_expense_id,
            expected_version,
        } => {
            delete_recurring_in_transaction(transaction, recurring_expense_id, *expected_version)?;
            Ok(Value::Null)
        }
        ExpenseMutationCommand::ConfirmRecurringPaid {
            occurrence_key,
            input,
        } => serde_json::to_value(confirm_recurring_paid_in_transaction(
            transaction,
            occurrence_key,
            input,
        )?)
        .map_err(Into::into),
        ExpenseMutationCommand::MatchRecurring {
            occurrence_key,
            input,
        } => serde_json::to_value(match_recurring_in_transaction(
            transaction,
            occurrence_key,
            input,
        )?)
        .map_err(Into::into),
        ExpenseMutationCommand::SaveReport(input) => {
            serde_json::to_value(save_expense_report_in_transaction(transaction, input)?)
                .map_err(Into::into)
        }
        ExpenseMutationCommand::RecordReportFeedback { report_id, helpful } => {
            record_expense_feedback_in_transaction(transaction, report_id, *helpful)?;
            Ok(Value::Null)
        }
    }
}

fn canonical_preview_content_sha256(input: &ExpenseImportPreviewInput) -> Result<String> {
    let mut rows = input
        .rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "stableKey": row.stable_key,
                "rowSha256": row.row_sha256,
                "sourceRowNumber": row.source_row_number,
                "occurredAt": row.occurred_at,
                "postedDate": row.posted_date,
                "direction": row.direction,
                "amountMinor": row.amount_minor,
                "currency": row.currency,
                "kind": row.kind,
                "categoryHint": row.category_hint,
                "merchantBlindIndex": row.merchant_blind_index,
                "paymentMethodFingerprint": row.payment_method_fingerprint,
                "externalReferenceFingerprint": row.external_reference_fingerprint,
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left["stableKey"].as_str().cmp(&right["stableKey"].as_str()));
    let canonical = serde_json::json!({
        "adapter": input.adapter,
        "sourceKind": input.source_kind,
        "sourceFingerprint": input.source_fingerprint,
        "fileSha256": input.file_sha256,
        "coverageStart": input.coverage_start,
        "coverageEnd": input.coverage_end,
        "rejectedCount": input.rejected_count,
        "rows": rows,
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical)?)
    ))
}

fn canonical_import_content_sha256(input: &NormalizedExpenseImport) -> Result<String> {
    let preview = ExpenseImportPreviewInput {
        adapter: input.adapter,
        source_kind: input.source_kind,
        source_fingerprint: input.source_fingerprint.clone(),
        file_sha256: input.file_sha256.clone(),
        normalized_sha256: input.normalized_sha256.clone(),
        coverage_start: input.coverage_start,
        coverage_end: input.coverage_end,
        rejected_count: input.rejected_count,
        rows: input
            .rows
            .iter()
            .map(|row| ExpenseImportPreviewRow {
                stable_key: row.stable_key.clone(),
                row_sha256: row.row_sha256.clone(),
                source_row_number: row.source_row_number,
                occurred_at: row.occurred_at.clone(),
                posted_date: row.posted_date,
                direction: row.direction,
                amount_minor: row.amount_minor,
                currency: row.currency.clone(),
                kind: row.kind,
                category_hint: row.category_hint,
                merchant_blind_index: row.merchant.as_ref().map(|value| value.blind_index.clone()),
                payment_method_fingerprint: row.payment_method_fingerprint.clone(),
                external_reference_fingerprint: row.external_reference_fingerprint.clone(),
            })
            .collect(),
    };
    canonical_preview_content_sha256(&preview)
}

fn consume_expense_preview_session(
    transaction: &Transaction<'_>,
    preview_session_id: &str,
    input: &NormalizedExpenseImport,
) -> Result<()> {
    let receipt = transaction
        .query_row(
            "SELECT adapter, source_kind, source_fingerprint, file_sha256,
                    normalized_sha256, content_sha256, coverage_start, coverage_end,
                    row_count, rejected_count, expires_at, consumed_at
             FROM expense_import_preview_sessions WHERE id = ?1",
            [preview_session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, NaiveDate>(6)?,
                    row.get::<_, NaiveDate>(7)?,
                    row.get::<_, u32>(8)?,
                    row.get::<_, u32>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| not_found("expense import preview session", preview_session_id))?;
    if receipt.11.is_some() {
        return Err(Error::Conflict(
            "expense import preview session has already been consumed".to_owned(),
        ));
    }
    let expires_at = DateTime::parse_from_rfc3339(&receipt.10)
        .map_err(|_| Error::Invariant("expense preview expiry is invalid".to_owned()))?
        .with_timezone(&Utc);
    if expires_at <= Utc::now() {
        return Err(Error::Conflict(
            "expense import preview session has expired".to_owned(),
        ));
    }
    let row_count = input
        .rows
        .len()
        .checked_add(input.rejected_count as usize)
        .and_then(|count| u32::try_from(count).ok())
        .ok_or_else(|| invalid("expense import row count overflow"))?;
    let content_sha256 = canonical_import_content_sha256(input)?;
    if receipt.0 != input.adapter.as_str()
        || receipt.1 != input.source_kind.as_str()
        || receipt.2 != input.source_fingerprint
        || receipt.3 != input.file_sha256
        || receipt.4 != input.normalized_sha256
        || receipt.5 != content_sha256
        || receipt.6 != input.coverage_start
        || receipt.7 != input.coverage_end
        || receipt.8 != row_count
        || receipt.9 != input.rejected_count
    {
        return Err(Error::Conflict(
            "expense import does not match its preview receipt".to_owned(),
        ));
    }
    let changed = transaction.execute(
        "UPDATE expense_import_preview_sessions
         SET consumed_at = ?2
         WHERE id = ?1 AND consumed_at IS NULL",
        params![preview_session_id, now_utc()],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "expense import preview session was consumed concurrently".to_owned(),
        ));
    }
    Ok(())
}

fn import_expenses_in_transaction(
    transaction: &Transaction<'_>,
    input: &NormalizedExpenseImport,
) -> Result<ExpenseImportResult> {
    let expected_source_kind = match input.adapter {
        ExpenseImportAdapter::KbCardUsageV1 => ExpenseSourceKind::Card,
        ExpenseImportAdapter::KbAccountHistoryV1 => ExpenseSourceKind::Account,
        ExpenseImportAdapter::KakaopayMoneyV1 => ExpenseSourceKind::Wallet,
    };
    if input.source_kind != expected_source_kind {
        return Err(invalid(format!(
            "adapter {} requires source kind {}",
            input.adapter, expected_source_kind
        )));
    }

    let source = transaction
        .query_row(
            "SELECT id, adapter, source_kind FROM expense_sources
             WHERE source_fingerprint = ?1",
            [&input.source_fingerprint],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let source_id = if let Some((id, adapter, source_kind)) = source {
        if adapter != input.adapter.as_str() || source_kind != input.source_kind.as_str() {
            return Err(Error::Conflict(
                "expense source fingerprint was already used for another adapter".to_owned(),
            ));
        }
        id
    } else {
        let id = new_id();
        let now = now_utc();
        transaction.execute(
            "INSERT INTO expense_sources(
                id, adapter, source_kind, source_fingerprint,
                coverage_start, coverage_end, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                input.adapter.as_str(),
                input.source_kind.as_str(),
                input.source_fingerprint,
                input.coverage_start,
                input.coverage_end,
                now,
            ],
        )?;
        id
    };

    let existing_batch = transaction
        .query_row(
            "SELECT id, normalized_sha256, row_count, accepted_count,
                    duplicate_count, rejected_count
             FROM expense_import_batches
             WHERE source_id = ?1 AND file_sha256 = ?2",
            params![source_id, input.file_sha256],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, u32>(5)?,
                ))
            },
        )
        .optional()?;
    if let Some((batch_id, normalized_sha256, row_count, accepted, duplicates, rejected)) =
        existing_batch
    {
        if normalized_sha256 != input.normalized_sha256 {
            return Err(Error::Conflict(
                "expense file fingerprint was replayed with different normalized content"
                    .to_owned(),
            ));
        }
        let (excluded_count, review_count): (u32, u32) = transaction.query_row(
            "SELECT
                count(DISTINCT CASE WHEN e.event_status = 'excluded' THEN e.id END),
                count(DISTINCT CASE WHEN r.review_status = 'pending' THEN r.id END)
             FROM expense_raw_rows AS raw
             LEFT JOIN expense_postings AS p ON p.raw_row_id = raw.id
             LEFT JOIN expense_events AS e ON e.primary_posting_id = p.id
             LEFT JOIN expense_reviews AS r ON r.event_id = e.id
             WHERE raw.batch_id = ?1",
            [&batch_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        return Ok(ExpenseImportResult {
            batch_id,
            source_id,
            row_count,
            new_count: accepted,
            duplicate_count: duplicates,
            rejected_count: rejected,
            excluded_count,
            review_count,
            idempotent_replay: true,
        });
    }

    let mut new_rows = Vec::new();
    let mut duplicate_count = 0_u32;
    for row in &input.rows {
        let existing = transaction
            .query_row(
                "SELECT row_sha256 FROM expense_raw_rows
                 WHERE source_id = ?1 AND stable_key = ?2",
                params![source_id, row.stable_key],
                |record| record.get::<_, String>(0),
            )
            .optional()?;
        match existing {
            Some(row_sha256) if row_sha256 == row.row_sha256 => {
                duplicate_count = duplicate_count.saturating_add(1);
            }
            Some(_) => {
                return Err(Error::Conflict(format!(
                    "expense stable row key was replayed with different content: {}",
                    row.stable_key
                )));
            }
            None => new_rows.push(row),
        }
    }

    let batch_id = new_id();
    let row_count = u32::try_from(input.rows.len())
        .map_err(|_| invalid("expense import contains too many rows"))?
        .saturating_add(input.rejected_count);
    let accepted_count = u32::try_from(new_rows.len())
        .map_err(|_| invalid("expense import contains too many rows"))?;
    transaction.execute(
        "INSERT INTO expense_import_batches(
            id, source_id, file_sha256, normalized_sha256,
            coverage_start, coverage_end, row_count, accepted_count,
            duplicate_count, rejected_count, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            batch_id,
            source_id,
            input.file_sha256,
            input.normalized_sha256,
            input.coverage_start,
            input.coverage_end,
            row_count,
            accepted_count,
            duplicate_count,
            input.rejected_count,
            now_utc(),
        ],
    )?;

    let mut excluded_count = 0_u32;
    let mut review_count = 0_u32;
    for row in new_rows {
        let rule = if matches!(
            row.kind,
            ExpenseEventKind::Purchase
                | ExpenseEventKind::UnknownP2p
                | ExpenseEventKind::ExternalTransfer
        ) {
            classification_rule(transaction, row)?
        } else {
            None
        };
        let resolved_kind = rule.as_ref().map_or(row.kind, |item| item.0);
        let category_hint = rule.as_ref().map(|item| item.1).or(row.category_hint);
        let (event_status, exclusion_reason, resolved_category) =
            initial_event_classification(resolved_kind, category_hint);
        let raw_id = new_id();
        let posting_id = new_id();
        let event_id = new_id();
        let now = now_utc();
        transaction.execute(
            "INSERT INTO expense_raw_rows(
                id, batch_id, source_id, stable_key, row_sha256,
                source_row_number, occurred_at, posted_date, direction,
                amount_minor, currency, normalized_kind, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                raw_id,
                batch_id,
                source_id,
                row.stable_key,
                row.row_sha256,
                row.source_row_number,
                row.occurred_at,
                row.posted_date,
                row.direction.as_str(),
                row.amount_minor,
                row.currency,
                row.kind.as_str(),
                now,
            ],
        )?;
        insert_posting(transaction, &posting_id, &raw_id, &source_id, row, &now)?;
        transaction.execute(
            "INSERT INTO expense_events(
                id, event_kind, category, event_status, amount_minor, currency,
                occurred_at, posted_date, primary_posting_id, exclusion_reason,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![
                event_id,
                resolved_kind.as_str(),
                resolved_category.as_str(),
                event_status.as_str(),
                row.amount_minor,
                row.currency,
                row.occurred_at,
                row.posted_date,
                posting_id,
                exclusion_reason,
                now,
            ],
        )?;
        transaction.execute(
            "INSERT INTO expense_event_postings(event_id, posting_id, posting_role, created_at)
             VALUES (?1, ?2, 'primary', ?3)",
            params![event_id, posting_id, now],
        )?;

        let duplicate_or_mirror = mark_duplicate_or_mirror(transaction, &event_id, row)?;
        if duplicate_or_mirror {
            excluded_count = excluded_count.saturating_add(1);
        } else if event_status == ExpenseEventStatus::Excluded {
            excluded_count = excluded_count.saturating_add(1);
        }

        if event_status == ExpenseEventStatus::Unconfirmed {
            create_review(
                transaction,
                &event_id,
                ExpenseReviewReason::UnknownP2p,
                None,
                None,
                None,
                None,
            )?;
            review_count = review_count.saturating_add(1);
        }
        if !duplicate_or_mirror
            && event_status == ExpenseEventStatus::Confirmed
            && resolved_kind == ExpenseEventKind::Purchase
            && rule.is_none()
        {
            create_review(
                transaction,
                &event_id,
                ExpenseReviewReason::CategoryConfirmation,
                None,
                Some(ExpenseEventKind::Purchase),
                Some(resolved_category),
                None,
            )?;
            review_count = review_count.saturating_add(1);
        }
        if !duplicate_or_mirror
            && event_status == ExpenseEventStatus::Confirmed
            && resolved_kind == ExpenseEventKind::Purchase
        {
            if maybe_create_ambiguous_mirror_review(transaction, &event_id, row)? {
                review_count = review_count.saturating_add(1);
            }
            review_count = review_count.saturating_add(maybe_link_recurring_candidate(
                transaction,
                &event_id,
                row,
            )?);
            review_count = review_count.saturating_add(maybe_create_recurring_registration_review(
                transaction,
                &event_id,
                row,
            )?);
        }
    }

    transaction.execute(
        "UPDATE expense_sources
         SET coverage_start = min(coalesce(coverage_start, ?2), ?2),
             coverage_end = max(coalesce(coverage_end, ?3), ?3),
             updated_at = ?4,
             version = version + 1
         WHERE id = ?1",
        params![
            source_id,
            input.coverage_start,
            input.coverage_end,
            now_utc()
        ],
    )?;

    Ok(ExpenseImportResult {
        batch_id,
        source_id,
        row_count,
        new_count: accepted_count,
        duplicate_count,
        rejected_count: input.rejected_count,
        excluded_count,
        review_count,
        idempotent_replay: false,
    })
}

fn insert_posting(
    transaction: &Transaction<'_>,
    posting_id: &str,
    raw_id: &str,
    source_id: &str,
    row: &NormalizedExpenseRow,
    now: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO expense_postings(
            id, raw_row_id, source_id, direction, amount_minor, currency,
            occurred_at, posted_date,
            merchant_key_version, merchant_nonce, merchant_ciphertext,
            merchant_aad, merchant_blind_index,
            counterparty_key_version, counterparty_nonce, counterparty_ciphertext,
            counterparty_aad, counterparty_blind_index,
            memo_key_version, memo_nonce, memo_ciphertext, memo_aad,
            memo_blind_index, payment_method_fingerprint,
            external_reference_fingerprint, created_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
            ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26
         )",
        params![
            posting_id,
            raw_id,
            source_id,
            row.direction.as_str(),
            row.amount_minor,
            row.currency,
            row.occurred_at,
            row.posted_date,
            envelope_key_version(&row.merchant),
            envelope_value(&row.merchant, |value| &value.nonce),
            envelope_value(&row.merchant, |value| &value.ciphertext),
            envelope_value(&row.merchant, |value| &value.aad),
            envelope_value(&row.merchant, |value| &value.blind_index),
            envelope_key_version(&row.counterparty),
            envelope_value(&row.counterparty, |value| &value.nonce),
            envelope_value(&row.counterparty, |value| &value.ciphertext),
            envelope_value(&row.counterparty, |value| &value.aad),
            envelope_value(&row.counterparty, |value| &value.blind_index),
            envelope_key_version(&row.memo),
            envelope_value(&row.memo, |value| &value.nonce),
            envelope_value(&row.memo, |value| &value.ciphertext),
            envelope_value(&row.memo, |value| &value.aad),
            envelope_value(&row.memo, |value| &value.blind_index),
            row.payment_method_fingerprint,
            row.external_reference_fingerprint,
            now,
        ],
    )?;
    Ok(())
}

fn envelope_key_version(value: &Option<EncryptedExpenseText>) -> Option<u32> {
    value.as_ref().map(|item| item.key_version)
}

fn envelope_value<'a>(
    value: &'a Option<EncryptedExpenseText>,
    field: impl FnOnce(&'a EncryptedExpenseText) -> &'a str,
) -> Option<&'a str> {
    value.as_ref().map(field)
}

fn initial_event_classification(
    kind: ExpenseEventKind,
    category_hint: Option<ExpenseCategory>,
) -> (ExpenseEventStatus, Option<&'static str>, ExpenseCategory) {
    match kind {
        ExpenseEventKind::CardPayment => (
            ExpenseEventStatus::Excluded,
            Some("card_payment"),
            ExpenseCategory::TransferSettlement,
        ),
        ExpenseEventKind::WalletTopup => (
            ExpenseEventStatus::Excluded,
            Some("wallet_topup"),
            ExpenseCategory::TransferSettlement,
        ),
        ExpenseEventKind::InternalTransfer => (
            ExpenseEventStatus::Excluded,
            Some("internal_transfer"),
            ExpenseCategory::TransferSettlement,
        ),
        ExpenseEventKind::UnknownP2p | ExpenseEventKind::ExternalTransfer => (
            ExpenseEventStatus::Unconfirmed,
            None,
            ExpenseCategory::Unconfirmed,
        ),
        ExpenseEventKind::Refund => (
            ExpenseEventStatus::Confirmed,
            None,
            category_hint.unwrap_or(ExpenseCategory::RefundIncome),
        ),
        ExpenseEventKind::SettlementReceived | ExpenseEventKind::SettlementSent => (
            ExpenseEventStatus::Confirmed,
            None,
            ExpenseCategory::TransferSettlement,
        ),
        ExpenseEventKind::Fee => (
            ExpenseEventStatus::Confirmed,
            None,
            category_hint.unwrap_or(ExpenseCategory::InsuranceFinanceTax),
        ),
        ExpenseEventKind::Purchase | ExpenseEventKind::ManualRecurring => (
            ExpenseEventStatus::Confirmed,
            None,
            category_hint.unwrap_or(ExpenseCategory::Other),
        ),
    }
}

fn classification_rule(
    transaction: &Transaction<'_>,
    row: &NormalizedExpenseRow,
) -> Result<Option<(ExpenseEventKind, ExpenseCategory)>> {
    let Some(merchant) = row.merchant.as_ref() else {
        return Ok(None);
    };
    transaction
        .query_row(
            "SELECT event_kind, category
             FROM expense_rules
             WHERE rule_kind = 'classification'
               AND merchant_blind_index = ?1
               AND payment_method_fingerprint IS ?2
             ORDER BY created_at DESC, id DESC
             LIMIT 1",
            params![merchant.blind_index, row.payment_method_fingerprint],
            |record| Ok((record.get::<_, String>(0)?, record.get::<_, String>(1)?)),
        )
        .optional()?
        .map(|(kind, category)| {
            let kind = ExpenseEventKind::from_str(&kind)?;
            let category = ExpenseCategory::from_str(&category)?;
            if !expense_kind_accepts_direction(kind, row.direction) {
                return Ok(None);
            }
            Ok(Some((kind, category)))
        })
        .transpose()
        .map(Option::flatten)
}

fn expense_kind_accepts_direction(kind: ExpenseEventKind, direction: ExpenseDirection) -> bool {
    match kind {
        ExpenseEventKind::Purchase
        | ExpenseEventKind::Fee
        | ExpenseEventKind::CardPayment
        | ExpenseEventKind::WalletTopup
        | ExpenseEventKind::SettlementSent => direction == ExpenseDirection::Debit,
        ExpenseEventKind::Refund | ExpenseEventKind::SettlementReceived => {
            direction == ExpenseDirection::Credit
        }
        ExpenseEventKind::InternalTransfer
        | ExpenseEventKind::ExternalTransfer
        | ExpenseEventKind::UnknownP2p => true,
        ExpenseEventKind::ManualRecurring => false,
    }
}

fn mark_duplicate_or_mirror(
    transaction: &Transaction<'_>,
    event_id: &str,
    row: &NormalizedExpenseRow,
) -> Result<bool> {
    let current_source_id: String = transaction.query_row(
        "SELECT p.source_id
         FROM expense_events AS e
         JOIN expense_postings AS p ON p.id = e.primary_posting_id
         WHERE e.id = ?1",
        [event_id],
        |record| record.get(0),
    )?;

    let duplicate = if let Some(reference) = row.external_reference_fingerprint.as_deref() {
        transaction
            .query_row(
                "SELECT e.id, p.source_id
                 FROM expense_postings AS p
                 JOIN expense_events AS e ON e.primary_posting_id = p.id
                 WHERE p.external_reference_fingerprint = ?1 AND e.id <> ?2
                 ORDER BY e.created_at, e.id
                 LIMIT 1",
                params![reference, event_id],
                |record| Ok((record.get::<_, String>(0)?, record.get::<_, String>(1)?)),
            )
            .optional()?
    } else {
        None
    };
    let Some((duplicate_of, duplicate_source_id)) = duplicate else {
        return Ok(false);
    };
    let reason = if duplicate_source_id == current_source_id {
        "duplicate"
    } else {
        "cross_source_mirror"
    };
    transaction.execute(
        "UPDATE expense_events
         SET event_status = 'excluded', exclusion_reason = ?2,
             duplicate_of_event_id = ?3, updated_at = ?4, version = version + 1
         WHERE id = ?1",
        params![event_id, reason, duplicate_of, now_utc()],
    )?;
    Ok(true)
}

fn maybe_create_ambiguous_mirror_review(
    transaction: &Transaction<'_>,
    event_id: &str,
    row: &NormalizedExpenseRow,
) -> Result<bool> {
    let Some(merchant) = row.merchant.as_ref() else {
        return Ok(false);
    };
    let candidate = transaction
        .query_row(
            "SELECT candidate.id
             FROM expense_events AS current
             JOIN expense_postings AS current_posting
               ON current_posting.id = current.primary_posting_id
             JOIN expense_postings AS candidate_posting
               ON candidate_posting.source_id != current_posting.source_id
              AND candidate_posting.posted_date = current_posting.posted_date
              AND candidate_posting.amount_minor = current_posting.amount_minor
              AND candidate_posting.currency = current_posting.currency
              AND candidate_posting.direction = current_posting.direction
              AND candidate_posting.merchant_blind_index = ?2
             JOIN expense_events AS candidate
               ON candidate.primary_posting_id = candidate_posting.id
             WHERE current.id = ?1 AND candidate.event_status = 'confirmed'
             ORDER BY candidate.created_at, candidate.id
             LIMIT 1",
            params![event_id, merchant.blind_index],
            |record| record.get::<_, String>(0),
        )
        .optional()?;
    let Some(candidate) = candidate else {
        return Ok(false);
    };
    create_review(
        transaction,
        event_id,
        ExpenseReviewReason::AmbiguousMirror,
        None,
        Some(ExpenseEventKind::Purchase),
        row.category_hint,
        Some(&candidate),
    )?;
    Ok(true)
}

fn create_review(
    transaction: &Transaction<'_>,
    event_id: &str,
    reason: ExpenseReviewReason,
    recurring_expense_id: Option<&str>,
    suggested_kind: Option<ExpenseEventKind>,
    suggested_category: Option<ExpenseCategory>,
    suggested_duplicate_of_event_id: Option<&str>,
) -> Result<()> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM expense_reviews
            WHERE event_id = ?1 AND review_reason = ?2
              AND recurring_expense_id IS ?3
         )",
        params![event_id, reason.as_str(), recurring_expense_id],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    transaction.execute(
        "INSERT INTO expense_reviews(
            id, event_id, recurring_expense_id, review_reason,
            suggested_kind, suggested_category,
            suggested_duplicate_of_event_id, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            new_id(),
            event_id,
            recurring_expense_id,
            reason.as_str(),
            suggested_kind.map(ExpenseEventKind::as_str),
            suggested_category.map(ExpenseCategory::as_str),
            suggested_duplicate_of_event_id,
            now_utc(),
        ],
    )?;
    Ok(())
}

fn maybe_link_recurring_candidate(
    transaction: &Transaction<'_>,
    event_id: &str,
    row: &NormalizedExpenseRow,
) -> Result<u32> {
    let Some(merchant) = row.merchant.as_ref() else {
        return Ok(0);
    };
    let mut statement = transaction.prepare(
        "SELECT id FROM recurring_expense_items
         WHERE deleted_at IS NULL
         ORDER BY created_at, id",
    )?;
    let candidate_ids = statement
        .query_map([], |record| record.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut candidates = Vec::new();
    let transaction_month = month_start(row.posted_date)?;
    let mut occurrence_months = vec![transaction_month];
    if let Ok(month) = previous_month(transaction_month) {
        occurrence_months.push(month);
    }
    if let Ok(month) = next_month(transaction_month) {
        occurrence_months.push(month);
    }
    occurrence_months.sort_unstable();
    for recurring_id in candidate_ids {
        let has_auto_rule: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM expense_rules
                WHERE rule_kind = 'recurring_match'
                  AND recurring_expense_id = ?1
                  AND merchant_blind_index = ?2
                  AND payment_method_fingerprint IS ?3
            )",
            params![
                &recurring_id,
                merchant.blind_index,
                row.payment_method_fingerprint,
            ],
            |record| record.get(0),
        )?;
        for occurrence_month in &occurrence_months {
            let Some(item) =
                recurring_item_for_month(transaction, &recurring_id, *occurrence_month)?
            else {
                continue;
            };
            if item.status != RecurringExpenseStatus::Active {
                continue;
            }
            let vendor_matches =
                item.vendor.as_ref().is_some_and(|vendor| {
                    vendor.blind_index.as_str() == merchant.blind_index.as_str()
                }) || (item.vendor.is_none() && has_auto_rule);
            let payment_method_matches = item.payment_method_fingerprint.as_deref()
                == row.payment_method_fingerprint.as_deref()
                || (item.payment_method_fingerprint.is_none() && has_auto_rule);
            if !vendor_matches || !payment_method_matches {
                continue;
            }
            let Some(occurrence) =
                virtual_occurrence_for_item(transaction, &item, *occurrence_month)?
            else {
                continue;
            };
            if occurrence.status == RecurringOccurrenceStatus::Matched {
                continue;
            }
            let within_window = (occurrence.due_date - row.posted_date).num_days().abs() <= 5;
            if !within_window || !recurring_amount_matches(&item, row.amount_minor) {
                continue;
            }
            candidates.push((item, occurrence, has_auto_rule));
        }
    }
    if candidates.len() == 1 {
        let (item, occurrence, has_auto_rule) = candidates
            .pop()
            .ok_or_else(|| Error::Invariant("recurring match candidate disappeared".to_owned()))?;
        if item.auto_match_enabled && has_auto_rule {
            match_recurring_in_transaction(
                transaction,
                &occurrence.occurrence_key,
                &MatchRecurringExpenseInput {
                    expected_version: occurrence.version,
                    event_id: event_id.to_owned(),
                    enable_future_auto_match: false,
                },
            )?;
            return Ok(0);
        }
        create_review(
            transaction,
            event_id,
            ExpenseReviewReason::RecurringMatchCandidate,
            Some(&item.id),
            Some(ExpenseEventKind::Purchase),
            Some(item.category),
            None,
        )?;
        return Ok(1);
    }
    let mut reviews = 0_u32;
    for (item, _, _) in candidates {
        create_review(
            transaction,
            event_id,
            ExpenseReviewReason::RecurringMatchCandidate,
            Some(&item.id),
            Some(ExpenseEventKind::Purchase),
            Some(item.category),
            None,
        )?;
        reviews = reviews.saturating_add(1);
    }
    Ok(reviews)
}

fn maybe_create_recurring_registration_review(
    transaction: &Transaction<'_>,
    event_id: &str,
    row: &NormalizedExpenseRow,
) -> Result<u32> {
    let Some(merchant) = row.merchant.as_ref() else {
        return Ok(0);
    };
    let already_registered: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM recurring_expense_items
            WHERE deleted_at IS NULL AND vendor_blind_index = ?1
              AND payment_method_fingerprint IS ?2
         )",
        params![merchant.blind_index, row.payment_method_fingerprint],
        |record| record.get(0),
    )?;
    if already_registered {
        return Ok(0);
    }
    let pending_candidate: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1
            FROM expense_reviews AS r
            JOIN expense_events AS e ON e.id = r.event_id
            JOIN expense_postings AS p ON p.id = e.primary_posting_id
            WHERE r.review_status = 'pending'
              AND r.review_reason = 'recurring_registration_candidate'
              AND p.merchant_blind_index = ?1
              AND p.payment_method_fingerprint IS ?2
         )",
        params![merchant.blind_index, row.payment_method_fingerprint],
        |record| record.get(0),
    )?;
    if pending_candidate {
        return Ok(0);
    }

    let transaction_months = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT substr(e.posted_date, 1, 7)
             FROM expense_events AS e
             JOIN expense_postings AS p ON p.id = e.primary_posting_id
             WHERE e.event_kind = 'purchase' AND e.event_status = 'confirmed'
               AND p.merchant_blind_index = ?1
               AND p.payment_method_fingerprint IS ?2
             ORDER BY 1",
        )?;
        statement
            .query_map(
                params![merchant.blind_index, row.payment_method_fingerprint],
                |record| record.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    if transaction_months.len() < 3 {
        return Ok(0);
    }
    let transaction_months = transaction_months
        .into_iter()
        .map(|month| {
            NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
                .map_err(|_| invalid("recurring candidate contains an invalid transaction month"))
        })
        .collect::<Result<Vec<_>>>()?;
    let intervals = transaction_months
        .windows(2)
        .map(|months| month_difference(months[0], months[1]))
        .collect::<Result<Vec<_>>>()?;
    let expected_interval = intervals[0];
    if ![1, 2, 3, 6, 12].contains(&expected_interval)
        || intervals
            .iter()
            .any(|interval| *interval != expected_interval)
    {
        return Ok(0);
    }

    let amounts = {
        let mut statement = transaction.prepare(
            "SELECT e.amount_minor
         FROM expense_events AS e
         JOIN expense_postings AS p ON p.id = e.primary_posting_id
         WHERE e.event_kind = 'purchase' AND e.event_status = 'confirmed'
           AND p.merchant_blind_index = ?1
           AND p.payment_method_fingerprint IS ?2
         ORDER BY e.posted_date, e.id",
        )?;
        statement
            .query_map(
                params![merchant.blind_index, row.payment_method_fingerprint],
                |record| record.get::<_, i64>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let minimum = amounts.iter().copied().min().unwrap_or_default();
    let maximum = amounts.iter().copied().max().unwrap_or_default();
    let sum = amounts
        .iter()
        .fold(0_i128, |total, amount| total + i128::from(*amount));
    let count = i128::try_from(amounts.len())
        .map_err(|_| Error::Invariant("too many recurring candidate amounts".to_owned()))?;
    let average = if count == 0 { 0 } else { sum / count };
    if i128::from(maximum.saturating_sub(minimum)) > (average.abs() / 10).max(1) {
        return Ok(0);
    }
    create_review(
        transaction,
        event_id,
        ExpenseReviewReason::RecurringRegistrationCandidate,
        None,
        Some(ExpenseEventKind::Purchase),
        row.category_hint,
        None,
    )?;
    Ok(1)
}

fn linked_settlement_totals(
    transaction: &Transaction<'_>,
    related_event_id: &str,
    excluding_event_id: &str,
) -> Result<(i128, i128)> {
    let mut received = 0_i128;
    let mut sent = 0_i128;
    let mut statement = transaction.prepare(
        "SELECT allocation_kind, amount_minor
         FROM expense_allocations
         WHERE related_event_id = ?1 AND event_id != ?2
         ORDER BY id",
    )?;
    let rows = statement.query_map(params![related_event_id, excluding_event_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (kind, amount_minor) = row?;
        match kind.as_str() {
            "settlement_received" => add_financial_aggregate(
                &mut received,
                i128::from(amount_minor),
                "linked settlement received",
            )?,
            "settlement_sent" => add_financial_aggregate(
                &mut sent,
                i128::from(amount_minor),
                "linked settlement sent",
            )?,
            _ => {}
        }
    }
    Ok((received, sent))
}

fn personal_allocation(
    transaction: &Transaction<'_>,
    event_id: &str,
) -> Result<Option<(i64, String)>> {
    transaction
        .query_row(
            "SELECT amount_minor, allocation_source
             FROM expense_allocations
             WHERE event_id = ?1 AND allocation_kind = 'personal'",
            [event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(Into::into)
}

fn upsert_personal_allocation(
    transaction: &Transaction<'_>,
    event_id: &str,
    amount_minor: i64,
    currency: &str,
    allocation_source: &str,
    now: &str,
) -> Result<()> {
    let allocation_id = transaction
        .query_row(
            "SELECT id FROM expense_allocations
             WHERE event_id = ?1 AND allocation_kind = 'personal'",
            [event_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .unwrap_or_else(new_id);
    transaction.execute(
        "INSERT INTO expense_allocations(
            id, event_id, allocation_kind, allocation_source,
            amount_minor, currency, created_at
         ) VALUES (?1, ?2, 'personal', ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
            allocation_source = excluded.allocation_source,
            amount_minor = excluded.amount_minor,
            currency = excluded.currency",
        params![
            allocation_id,
            event_id,
            allocation_source,
            amount_minor,
            currency,
            now,
        ],
    )?;
    Ok(())
}

fn delete_settlement_personal_allocation(
    transaction: &Transaction<'_>,
    event_id: &str,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM expense_allocations
         WHERE event_id = ?1 AND allocation_kind = 'personal'
           AND allocation_source = 'settlement'",
        [event_id],
    )?;
    Ok(())
}

fn recompute_settlement_personal_allocation(
    transaction: &Transaction<'_>,
    event_id: &str,
    now: &str,
) -> Result<()> {
    let event = transaction
        .query_row(
            "SELECT event_kind, event_status, amount_minor, currency
             FROM expense_events WHERE id = ?1",
            [event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((kind, status, amount_minor, currency)) = event else {
        return Ok(());
    };
    if kind != ExpenseEventKind::Purchase.as_str()
        || status != ExpenseEventStatus::Confirmed.as_str()
    {
        return delete_settlement_personal_allocation(transaction, event_id);
    }
    let (received, sent) = linked_settlement_totals(transaction, event_id, "")?;
    if received == 0 && sent == 0 {
        return delete_settlement_personal_allocation(transaction, event_id);
    }
    let calculated = i128::from(amount_minor) + sent - received;
    if calculated < 0 || calculated > i128::from(amount_minor) {
        return Err(invalid(format!(
            "settlement allocations exceed the related purchase amount ({amount_minor})"
        )));
    }
    if let Some((personal_amount, source)) = personal_allocation(transaction, event_id)? {
        if source == "user" {
            if i128::from(personal_amount) > calculated {
                return Err(invalid(format!(
                    "personal amount and linked settlements exceed the purchase amount ({amount_minor})"
                )));
            }
            return Ok(());
        }
    }
    if calculated == 0 {
        // Allocation rows are strictly positive.  A linked, fully reimbursed
        // purchase represents its zero personal share by omitting this row.
        return delete_settlement_personal_allocation(transaction, event_id);
    }
    upsert_personal_allocation(
        transaction,
        event_id,
        safe_summary_amount("settlement personal allocation", calculated)?,
        &currency,
        "settlement",
        now,
    )
}

fn override_expense_transaction_in_transaction(
    transaction: &Transaction<'_>,
    event_id: &str,
    input: &OverrideExpenseTransactionInput,
) -> Result<ExpenseTransaction> {
    let (current_version, primary_posting_id) = transaction
        .query_row(
            "SELECT version, primary_posting_id FROM expense_events WHERE id = ?1",
            [event_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?
        .ok_or_else(|| not_found("expense event", event_id))?;
    if current_version != input.expected_version {
        return Err(Error::Conflict(format!(
            "expense event {event_id} changed from version {} to {current_version}",
            input.expected_version
        )));
    }
    if primary_posting_id.is_none() {
        return Err(invalid(
            "manual recurring events can only be changed through their recurring occurrence",
        ));
    }
    let allocation_kinds = {
        let mut statement = transaction.prepare(
            "SELECT allocation_kind FROM expense_allocations
             WHERE event_id = ?1 ORDER BY allocation_kind",
        )?;
        statement
            .query_map([event_id], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    let has_personal = allocation_kinds.iter().any(|kind| kind == "personal");
    let has_settlement = allocation_kinds
        .iter()
        .any(|kind| matches!(kind.as_str(), "settlement_received" | "settlement_sent"));
    if input.clear_personal_amount && has_settlement {
        return Err(invalid(
            "clearing a personal split cannot implicitly remove a settlement link",
        ));
    }
    if input.clear_related_event && has_personal {
        return Err(invalid(
            "clearing a settlement link cannot implicitly remove a personal split",
        ));
    }
    let preserve_current_allocations = input.personal_amount_minor.is_none()
        && input.related_event_id.is_none()
        && input.duplicate_of_event_id.is_none()
        && !input.clear_personal_amount
        && !input.clear_related_event;
    if preserve_current_allocations {
        if has_personal && input.kind != ExpenseEventKind::Purchase {
            return Err(invalid(
                "changing a split purchase requires clearPersonalAmount=true",
            ));
        }
        if allocation_kinds.iter().any(|kind| {
            (kind == "settlement_received" && input.kind != ExpenseEventKind::SettlementReceived)
                || (kind == "settlement_sent" && input.kind != ExpenseEventKind::SettlementSent)
        }) {
            return Err(invalid(
                "changing a linked settlement requires clearRelatedEvent=true",
            ));
        }
    }
    let has_incoming_settlements: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM expense_allocations WHERE related_event_id = ?1
         )",
        [event_id],
        |row| row.get(0),
    )?;
    if input.clear_personal_amount && has_incoming_settlements {
        return Err(invalid(
            "linked settlements must be cleared before clearing their purchase split",
        ));
    }
    if has_incoming_settlements
        && !matches!(
            input.kind,
            ExpenseEventKind::Purchase | ExpenseEventKind::Refund
        )
    {
        return Err(invalid(
            "linked settlements must be cleared before changing their purchase or refund target",
        ));
    }

    let review_id = new_id();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO expense_reviews(
            id, event_id, review_reason, review_status, created_at, version
         ) VALUES (?1, ?2, 'manual_override', 'pending', ?3, 1)",
        params![review_id, event_id, now],
    )?;
    resolve_expense_review_in_transaction(
        transaction,
        &review_id,
        &ResolveExpenseReviewInput {
            expected_version: 1,
            kind: input.kind,
            category: input.category,
            duplicate_of_event_id: input.duplicate_of_event_id.clone(),
            related_event_id: input.related_event_id.clone(),
            personal_amount_minor: input.personal_amount_minor,
            create_rule: input.create_rule,
        },
        preserve_current_allocations,
    )?;
    query_expense_transaction(transaction, event_id)
}

fn resolve_expense_review_in_transaction(
    transaction: &Transaction<'_>,
    review_id: &str,
    input: &ResolveExpenseReviewInput,
    preserve_current_allocations: bool,
) -> Result<()> {
    let current = transaction
        .query_row(
            "SELECT r.event_id, r.review_status, r.version,
                    e.amount_minor, e.currency, p.direction, p.source_id,
                    r.review_reason
             FROM expense_reviews AS r
             JOIN expense_events AS e ON e.id = r.event_id
             LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
             WHERE r.id = ?1",
            [review_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| not_found("expense review", review_id))?;
    if current.1 != ExpenseReviewStatus::Pending.as_str() {
        return Err(Error::Conflict(format!(
            "expense review {review_id} has already been resolved"
        )));
    }
    if current.2 != input.expected_version {
        return Err(Error::Conflict(format!(
            "expense review {review_id} changed from version {} to {}",
            input.expected_version, current.2
        )));
    }
    let expected_direction = match input.kind {
        ExpenseEventKind::Purchase
        | ExpenseEventKind::Fee
        | ExpenseEventKind::CardPayment
        | ExpenseEventKind::WalletTopup
        | ExpenseEventKind::SettlementSent => Some(ExpenseDirection::Debit),
        ExpenseEventKind::Refund | ExpenseEventKind::SettlementReceived => {
            Some(ExpenseDirection::Credit)
        }
        _ => None,
    };
    if let Some(expected_direction) = expected_direction {
        let actual_direction = current
            .5
            .as_deref()
            .map(ExpenseDirection::from_str)
            .transpose()?;
        if actual_direction != Some(expected_direction) {
            return Err(invalid(format!(
                "expense kind {} requires a {} posting",
                input.kind, expected_direction
            )));
        }
    }
    if input.related_event_id.is_some() && input.personal_amount_minor.is_some() {
        return Err(invalid(
            "an expense review cannot set a personal amount and a related event together",
        ));
    }
    if input.duplicate_of_event_id.is_some()
        && (input.related_event_id.is_some() || input.personal_amount_minor.is_some())
    {
        return Err(invalid(
            "a duplicate expense decision cannot also create an allocation",
        ));
    }
    if let Some(duplicate_id) = input.duplicate_of_event_id.as_deref() {
        if duplicate_id == current.0 {
            return Err(invalid("an expense event cannot duplicate itself"));
        }
        let target = transaction
            .query_row(
                "SELECT e.event_status, e.is_provisional, e.amount_minor,
                        e.currency, e.duplicate_of_event_id, p.direction, p.source_id
                 FROM expense_events AS e
                 LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
                 WHERE e.id = ?1",
                [duplicate_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, bool>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| not_found("expense event", duplicate_id))?;
        if target.0 != ExpenseEventStatus::Confirmed.as_str()
            || target.1
            || target.2 != current.3
            || target.3 != current.4
            || target.4.is_some()
        {
            return Err(invalid(
                "a duplicate target must be a confirmed non-provisional event with the same amount and currency",
            ));
        }
        if current.5 != target.5 {
            return Err(invalid(
                "a duplicate target must have the same posting direction",
            ));
        }
        if current.7 == ExpenseReviewReason::AmbiguousMirror.as_str()
            && (current.6.is_none() || current.6 == target.6)
        {
            return Err(invalid(
                "an ambiguous cross-source mirror must target another source",
            ));
        }
    }
    let mut pending_personal_allocation: Option<(String, i64, String, &'static str)> = None;
    let mut pending_personal_allocation_removal: Option<String> = None;
    let mut pending_settlement_allocation: Option<(&'static str, String)> = None;
    if !preserve_current_allocations {
        if let Some(amount_minor) = input.personal_amount_minor {
            if input.kind != ExpenseEventKind::Purchase {
                return Err(invalid(
                    "a personal amount allocation can only be applied to a purchase",
                ));
            }
            validate_amount_minor("personal amountMinor", amount_minor)?;
            if amount_minor > current.3 {
                return Err(invalid(format!(
                    "personal amount must be between 1 and the purchase amount ({})",
                    current.3
                )));
            }
            let (settlement_received, settlement_sent) =
                linked_settlement_totals(transaction, &current.0, "")?;
            let maximum_personal = i128::from(current.3) + settlement_sent - settlement_received;
            if i128::from(amount_minor) > maximum_personal {
                return Err(invalid(format!(
                    "personal amount and linked settlements exceed the purchase amount ({})",
                    current.3
                )));
            }
            pending_personal_allocation =
                Some((current.0.clone(), amount_minor, current.4.clone(), "user"));
        } else if let Some(related_event_id) = input.related_event_id.as_deref() {
            if !matches!(
                input.kind,
                ExpenseEventKind::SettlementReceived | ExpenseEventKind::SettlementSent
            ) {
                return Err(invalid(
                    "a related expense event can only be set for a settlement",
                ));
            }
            validate_label("related expense event ID", related_event_id, 128)?;
            if related_event_id == current.0 {
                return Err(invalid("an expense event cannot be related to itself"));
            }
            let target = transaction
                .query_row(
                    "SELECT event_kind, event_status, amount_minor, currency
                 FROM expense_events WHERE id = ?1",
                    [related_event_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| not_found("related expense event", related_event_id))?;
            let target_kind = ExpenseEventKind::from_str(&target.0)?;
            let target_status = ExpenseEventStatus::from_str(&target.1)?;
            if !matches!(
                target_kind,
                ExpenseEventKind::Purchase | ExpenseEventKind::Refund
            ) || target_status != ExpenseEventStatus::Confirmed
            {
                return Err(invalid(
                    "a settlement must be related to a confirmed purchase or refund",
                ));
            }
            if target.3 != current.4 {
                return Err(invalid(format!(
                    "settlement currency {} does not match related event currency {}",
                    current.4, target.3
                )));
            }
            let allocation_kind = match input.kind {
                ExpenseEventKind::SettlementReceived => "settlement_received",
                ExpenseEventKind::SettlementSent => "settlement_sent",
                _ => unreachable!("settlement kind was checked above"),
            };
            let (mut settlement_received, mut settlement_sent) =
                linked_settlement_totals(transaction, related_event_id, &current.0)?;
            match input.kind {
                ExpenseEventKind::SettlementReceived => {
                    settlement_received += i128::from(current.3);
                }
                ExpenseEventKind::SettlementSent => {
                    settlement_sent += i128::from(current.3);
                }
                _ => unreachable!("settlement kind was checked above"),
            }
            if target_kind == ExpenseEventKind::Purchase {
                let calculated_personal =
                    i128::from(target.2) + settlement_sent - settlement_received;
                if calculated_personal < 0 || calculated_personal > i128::from(target.2) {
                    return Err(invalid(format!(
                        "settlement allocations exceed the related purchase amount ({})",
                        target.2
                    )));
                }
                match personal_allocation(transaction, related_event_id)? {
                    Some((personal_amount, source)) if source == "user" => {
                        if i128::from(personal_amount) > calculated_personal {
                            return Err(invalid(format!(
                                "personal amount and linked settlements exceed the purchase amount ({})",
                                target.2
                            )));
                        }
                    }
                    Some(_) | None => {
                        if calculated_personal == 0 {
                            pending_personal_allocation_removal = Some(related_event_id.to_owned());
                        } else {
                            pending_personal_allocation = Some((
                                related_event_id.to_owned(),
                                safe_summary_amount(
                                    "settlement personal allocation",
                                    calculated_personal,
                                )?,
                                target.3.clone(),
                                "settlement",
                            ));
                        }
                    }
                }
            } else if settlement_received + settlement_sent > i128::from(target.2) {
                return Err(invalid(format!(
                    "settlement allocations exceed the related refund amount ({})",
                    target.2
                )));
            }
            pending_settlement_allocation = Some((allocation_kind, related_event_id.to_owned()));
        }
    }
    if !preserve_current_allocations
        && pending_personal_allocation.is_none()
        && input.related_event_id.is_none()
        && input.kind == ExpenseEventKind::Purchase
    {
        let (received, sent) = linked_settlement_totals(transaction, &current.0, "")?;
        if received != 0 || sent != 0 {
            let calculated = i128::from(current.3) + sent - received;
            if calculated < 0 || calculated > i128::from(current.3) {
                return Err(invalid(format!(
                    "settlement allocations exceed the purchase amount ({})",
                    current.3
                )));
            }
            if calculated == 0 {
                if personal_allocation(transaction, &current.0)?.is_some_and(
                    |(personal_amount, source)| {
                        source == "user" && i128::from(personal_amount) > calculated
                    },
                ) {
                    return Err(invalid(format!(
                        "personal amount and linked settlements exceed the purchase amount ({})",
                        current.3
                    )));
                }
                pending_personal_allocation_removal = Some(current.0.clone());
            } else {
                pending_personal_allocation = Some((
                    current.0.clone(),
                    safe_summary_amount("settlement personal allocation", calculated)?,
                    current.4.clone(),
                    "settlement",
                ));
            }
        }
    }
    let (event_status, exclusion_reason) = if input.duplicate_of_event_id.is_some() {
        (ExpenseEventStatus::Excluded, Some("duplicate"))
    } else {
        match input.kind {
            ExpenseEventKind::CardPayment => (ExpenseEventStatus::Excluded, Some("card_payment")),
            ExpenseEventKind::WalletTopup => (ExpenseEventStatus::Excluded, Some("wallet_topup")),
            ExpenseEventKind::InternalTransfer => {
                (ExpenseEventStatus::Excluded, Some("internal_transfer"))
            }
            ExpenseEventKind::UnknownP2p | ExpenseEventKind::ExternalTransfer => {
                (ExpenseEventStatus::Unconfirmed, None)
            }
            _ => (ExpenseEventStatus::Confirmed, None),
        }
    };
    let now = now_utc();
    let previous_related_events = {
        let mut statement = transaction.prepare(
            "SELECT DISTINCT related_event_id
             FROM expense_allocations
             WHERE event_id = ?1 AND related_event_id IS NOT NULL",
        )?;
        statement
            .query_map([&current.0], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    if !preserve_current_allocations {
        transaction.execute(
            "DELETE FROM expense_allocations WHERE event_id = ?1",
            [&current.0],
        )?;
        if !matches!(
            input.kind,
            ExpenseEventKind::Purchase | ExpenseEventKind::Refund
        ) {
            transaction.execute(
                "DELETE FROM expense_allocations WHERE related_event_id = ?1",
                [&current.0],
            )?;
        }
    }
    if let Some(event_id) = pending_personal_allocation_removal {
        delete_settlement_personal_allocation(transaction, &event_id)?;
    }
    transaction.execute(
        "UPDATE expense_events
         SET event_kind = ?2, category = ?3, event_status = ?4,
             duplicate_of_event_id = ?5, exclusion_reason = ?6,
             updated_at = ?7, version = version + 1
         WHERE id = ?1",
        params![
            current.0,
            input.kind.as_str(),
            input.category.as_str(),
            event_status.as_str(),
            input.duplicate_of_event_id,
            exclusion_reason,
            now,
        ],
    )?;
    if let Some((event_id, amount_minor, currency, allocation_source)) = pending_personal_allocation
    {
        upsert_personal_allocation(
            transaction,
            &event_id,
            amount_minor,
            &currency,
            allocation_source,
            &now,
        )?;
    }
    if let Some((allocation_kind, related_event_id)) = pending_settlement_allocation {
        transaction.execute(
            "INSERT INTO expense_allocations(
                id, event_id, related_event_id, allocation_kind, allocation_source,
                amount_minor, currency, created_at
             ) VALUES (?1, ?2, ?3, ?4, 'user', ?5, ?6, ?7)",
            params![
                new_id(),
                current.0,
                related_event_id,
                allocation_kind,
                current.3,
                current.4,
                now,
            ],
        )?;
    }
    if !preserve_current_allocations {
        for related_event_id in previous_related_events {
            if input.related_event_id.as_deref() != Some(related_event_id.as_str()) {
                recompute_settlement_personal_allocation(transaction, &related_event_id, &now)?;
            }
        }
    }
    if input.create_rule {
        let rule_source = transaction
            .query_row(
                "SELECT p.merchant_blind_index, p.payment_method_fingerprint
                 FROM expense_events AS e
                 JOIN expense_postings AS p ON p.id = e.primary_posting_id
                 WHERE e.id = ?1",
                [&current.0],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                    ))
                },
            )
            .optional()?;
        let Some((Some(merchant), payment_method)) = rule_source else {
            return Err(invalid(
                "a classification rule requires an encrypted merchant blind index",
            ));
        };
        let changed = transaction.execute(
            "UPDATE expense_rules
             SET event_kind = ?3, category = ?4, created_at = ?5
             WHERE rule_kind = 'classification'
               AND merchant_blind_index = ?1
               AND payment_method_fingerprint IS ?2",
            params![
                merchant,
                payment_method,
                input.kind.as_str(),
                input.category.as_str(),
                now,
            ],
        )?;
        if changed == 0 {
            transaction.execute(
                "INSERT INTO expense_rules(
                    id, rule_kind, merchant_blind_index, payment_method_fingerprint,
                    event_kind, category, created_at
                 ) VALUES (?1, 'classification', ?2, ?3, ?4, ?5, ?6)",
                params![
                    new_id(),
                    merchant,
                    payment_method,
                    input.kind.as_str(),
                    input.category.as_str(),
                    now,
                ],
            )?;
        }
    }
    let changed = transaction.execute(
        "UPDATE expense_reviews
         SET review_status = 'resolved', resolved_kind = ?2,
             resolved_category = ?3, duplicate_of_event_id = ?4,
             create_rule = ?5, resolved_at = ?6, version = version + 1
         WHERE id = ?1 AND review_status = 'pending' AND version = ?7",
        params![
            review_id,
            input.kind.as_str(),
            input.category.as_str(),
            input.duplicate_of_event_id,
            input.create_rule,
            now,
            input.expected_version,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "expense review {review_id} changed before resolution completed"
        )));
    }
    transaction.execute(
        "UPDATE expense_reviews
         SET review_status = 'resolved', resolved_kind = ?2,
             resolved_category = ?3, duplicate_of_event_id = ?4,
             resolved_at = ?5, version = version + 1
         WHERE event_id = ?1 AND id != ?6 AND review_status = 'pending'
           AND (
               review_reason IN (
                   'unknown_p2p', 'ambiguous_mirror',
                   'category_confirmation', 'import_rejected'
               )
               OR ?7 != 'confirmed' OR ?2 != 'purchase'
           )",
        params![
            current.0,
            input.kind.as_str(),
            input.category.as_str(),
            input.duplicate_of_event_id,
            now,
            review_id,
            event_status.as_str(),
        ],
    )?;
    Ok(())
}

fn query_expense_review(connection: &Connection, review_id: &str) -> Result<ExpenseReview> {
    let raw = connection
        .query_row(
            "SELECT id, event_id, review_reason, review_status,
                    recurring_expense_id, suggested_kind, suggested_category,
                    suggested_duplicate_of_event_id, created_at, resolved_at, version
             FROM expense_reviews WHERE id = ?1",
            [review_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, u64>(10)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| not_found("expense review", review_id))?;
    Ok(ExpenseReview {
        id: raw.0,
        reason: ExpenseReviewReason::from_str(&raw.2)?,
        status: ExpenseReviewStatus::from_str(&raw.3)?,
        transaction: query_expense_transaction(connection, &raw.1)?,
        recurring_expense_id: raw.4,
        suggested_kind: raw
            .5
            .map(|value| ExpenseEventKind::from_str(&value))
            .transpose()?,
        suggested_category: raw
            .6
            .map(|value| ExpenseCategory::from_str(&value))
            .transpose()?,
        suggested_duplicate_of_event_id: raw.7,
        created_at: raw.8,
        resolved_at: raw.9,
        version: raw.10,
    })
}

fn query_expense_transaction(
    connection: &Connection,
    event_id: &str,
) -> Result<ExpenseTransaction> {
    connection
        .query_row(
            "SELECT e.id, e.event_kind, e.category, e.event_status,
                    e.amount_minor, e.currency, e.occurred_at, e.posted_date,
                    s.source_kind, s.source_fingerprint, raw.stable_key,
                    p.merchant_key_version, p.merchant_nonce, p.merchant_ciphertext,
                    p.merchant_aad, p.merchant_blind_index,
                    p.counterparty_key_version, p.counterparty_nonce,
                    p.counterparty_ciphertext, p.counterparty_aad,
                    p.counterparty_blind_index,
                    p.memo_key_version, p.memo_nonce, p.memo_ciphertext,
                    p.memo_aad, p.memo_blind_index,
                    p.payment_method_fingerprint, e.exclusion_reason,
                    e.duplicate_of_event_id, e.is_provisional,
                    (SELECT r.id FROM expense_reviews AS r
                     WHERE r.event_id = e.id AND r.review_status = 'pending'
                     ORDER BY CASE r.review_reason
                         WHEN 'unknown_p2p' THEN 0
                         WHEN 'ambiguous_mirror' THEN 1
                         WHEN 'import_rejected' THEN 2
                         WHEN 'category_confirmation' THEN 3
                         WHEN 'recurring_match_candidate' THEN 4
                         WHEN 'recurring_registration_candidate' THEN 5
                         ELSE 6 END,
                         r.created_at, r.id
                     LIMIT 1),
                    (SELECT a.amount_minor FROM expense_allocations AS a
                     WHERE a.event_id = e.id AND a.allocation_kind = 'personal'),
                    (SELECT a.related_event_id FROM expense_allocations AS a
                     WHERE a.event_id = e.id
                       AND a.allocation_kind IN (
                           'settlement_received', 'settlement_sent'
                       )
                     LIMIT 1),
                    e.version
             FROM expense_events AS e
             LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
             LEFT JOIN expense_raw_rows AS raw ON raw.id = p.raw_row_id
             LEFT JOIN expense_sources AS s ON s.id = p.source_id
             WHERE e.id = ?1",
            [event_id],
            map_expense_transaction,
        )
        .optional()?
        .ok_or_else(|| not_found("expense event", event_id))
}

fn map_expense_transaction(row: &Row<'_>) -> rusqlite::Result<ExpenseTransaction> {
    let source_fingerprint = row.get::<_, Option<String>>(9)?;
    let stable_key = row.get::<_, Option<String>>(10)?;
    let crypto_context = match (source_fingerprint, stable_key) {
        (Some(source_fingerprint), Some(stable_key)) => {
            Some(format!("{source_fingerprint}:{stable_key}"))
        }
        (None, None) => None,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(Error::Invariant(
                    "expense transaction crypto context is incomplete".to_owned(),
                )),
            ));
        }
    };
    Ok(ExpenseTransaction {
        id: row.get(0)?,
        kind: parse_db_enum(row.get::<_, String>(1)?, 1)?,
        category: parse_db_enum(row.get::<_, String>(2)?, 2)?,
        status: parse_db_enum(row.get::<_, String>(3)?, 3)?,
        amount_minor: row.get(4)?,
        currency: row.get(5)?,
        occurred_at: row.get(6)?,
        posted_date: row.get(7)?,
        source_kind: row
            .get::<_, Option<String>>(8)?
            .map(|value| parse_db_enum(value, 8))
            .transpose()?,
        merchant: encrypted_from_row(row, 11)?,
        counterparty: encrypted_from_row(row, 16)?,
        memo: encrypted_from_row(row, 21)?,
        payment_method_fingerprint: row.get(26)?,
        exclusion_reason: row.get(27)?,
        duplicate_of_event_id: row.get(28)?,
        is_provisional: row.get(29)?,
        pending_review_id: row.get(30)?,
        personal_amount_minor: row.get(31)?,
        related_event_id: row.get(32)?,
        version: row.get(33)?,
        crypto_context,
    })
}

fn encrypted_from_row(
    row: &Row<'_>,
    start: usize,
) -> rusqlite::Result<Option<EncryptedExpenseText>> {
    let key_version = row.get::<_, Option<u32>>(start)?;
    key_version
        .map(|key_version| {
            Ok(EncryptedExpenseText {
                key_version,
                nonce: required_db_value(row, start + 1)?,
                ciphertext: required_db_value(row, start + 2)?,
                aad: required_db_value(row, start + 3)?,
                blind_index: required_db_value(row, start + 4)?,
            })
        })
        .transpose()
}

fn required_db_value(row: &Row<'_>, index: usize) -> rusqlite::Result<String> {
    row.get::<_, Option<String>>(index)?.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Null,
            Box::new(Error::Invariant(
                "encrypted expense envelope is incomplete".to_owned(),
            )),
        )
    })
}

fn parse_db_enum<T>(value: String, index: usize) -> rusqlite::Result<T>
where
    T: FromStr<Err = Error>,
{
    T::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

impl TmCore {
    pub fn list_recurring_expenses(&self) -> Result<Vec<RecurringExpenseItem>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(&format!(
            "{} WHERE deleted_at IS NULL ORDER BY created_at, id",
            recurring_item_select()
        ))?;
        let heads = statement
            .query_map([], map_recurring_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let current_month = current_month_start()?;
        heads
            .into_iter()
            .map(|head| {
                let mut visible = recurring_item_for_month(&connection, &head.id, current_month)?
                    .unwrap_or_else(|| head.clone());
                // The head version is the CAS revision while future snapshots are pending.
                visible.version = head.version;
                visible.updated_at = head.updated_at;
                validate_loaded_recurring_item(visible)
            })
            .collect()
    }

    pub fn create_recurring_expense(
        &self,
        input: CreateRecurringExpenseInput,
    ) -> Result<RecurringExpenseItem> {
        validate_recurring_create(&input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result = create_recurring_in_transaction(transaction, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::CreateRecurring(input.clone()),
                )?;
                Ok(result)
            })
    }

    pub fn update_recurring_expense(
        &self,
        recurring_expense_id: &str,
        input: UpdateRecurringExpenseInput,
    ) -> Result<RecurringExpenseItem> {
        validate_recurring_update(recurring_expense_id, &input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result =
                    update_recurring_in_transaction(transaction, recurring_expense_id, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::UpdateRecurring {
                        recurring_expense_id: recurring_expense_id.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }

    pub fn delete_recurring_expense(
        &self,
        recurring_expense_id: &str,
        expected_version: u64,
    ) -> Result<()> {
        if expected_version == 0 {
            return Err(invalid(
                "recurring expense expected version must be positive",
            ));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                delete_recurring_in_transaction(
                    transaction,
                    recurring_expense_id,
                    expected_version,
                )?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::DeleteRecurring {
                        recurring_expense_id: recurring_expense_id.to_owned(),
                        expected_version,
                    },
                )
            })
    }

    pub fn recurring_expense_occurrences(
        &self,
        month_start: NaiveDate,
    ) -> Result<Vec<RecurringExpenseOccurrence>> {
        validate_month_start(month_start)?;
        let connection = self.database.connect()?;
        recurring_occurrences_in_connection(&connection, month_start)
    }

    pub fn confirm_recurring_expense_paid(
        &self,
        occurrence_key: &str,
        input: ConfirmRecurringPaidInput,
    ) -> Result<RecurringExpenseOccurrence> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result =
                    confirm_recurring_paid_in_transaction(transaction, occurrence_key, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::ConfirmRecurringPaid {
                        occurrence_key: occurrence_key.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }

    pub fn match_recurring_expense(
        &self,
        occurrence_key: &str,
        input: MatchRecurringExpenseInput,
    ) -> Result<RecurringExpenseOccurrence> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let result = match_recurring_in_transaction(transaction, occurrence_key, &input)?;
                refresh_after_expense_mutation(
                    transaction,
                    &ExpenseMutationCommand::MatchRecurring {
                        occurrence_key: occurrence_key.to_owned(),
                        input: input.clone(),
                    },
                )?;
                Ok(result)
            })
    }
}

fn create_recurring_in_transaction(
    transaction: &Transaction<'_>,
    input: &CreateRecurringExpenseInput,
) -> Result<RecurringExpenseItem> {
    let id = &input.id;
    let now = now_utc();
    transaction.execute(
        "INSERT INTO recurring_expense_items(
            id, name_key_version, name_nonce, name_ciphertext, name_aad,
            name_blind_index, category,
            vendor_key_version, vendor_nonce, vendor_ciphertext, vendor_aad,
            vendor_blind_index, amount_minor, currency,
            payment_method_fingerprint, start_date, end_date,
            memo_key_version, memo_nonce, memo_ciphertext, memo_aad,
            memo_blind_index, reminder_days, amount_kind, interval_months,
            due_rule, due_day, item_status, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
            ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?29
         )",
        params![
            id,
            input.name.key_version,
            input.name.nonce,
            input.name.ciphertext,
            input.name.aad,
            input.name.blind_index,
            input.category.as_str(),
            envelope_key_version(&input.vendor),
            envelope_value(&input.vendor, |value| &value.nonce),
            envelope_value(&input.vendor, |value| &value.ciphertext),
            envelope_value(&input.vendor, |value| &value.aad),
            envelope_value(&input.vendor, |value| &value.blind_index),
            input.amount_minor,
            input.currency,
            input.payment_method_fingerprint,
            input.start_date,
            input.end_date,
            envelope_key_version(&input.memo),
            envelope_value(&input.memo, |value| &value.nonce),
            envelope_value(&input.memo, |value| &value.ciphertext),
            envelope_value(&input.memo, |value| &value.aad),
            envelope_value(&input.memo, |value| &value.blind_index),
            input.reminder_days,
            input.amount_kind.as_str(),
            input.interval_months,
            input.due_rule.as_str(),
            input.due_day,
            input.status.as_str(),
            now,
        ],
    )?;
    let item = query_recurring_item(transaction, id, false)?;
    insert_recurring_version(transaction, &item, month_start(input.start_date)?)?;
    Ok(item)
}

fn update_recurring_in_transaction(
    transaction: &Transaction<'_>,
    recurring_expense_id: &str,
    input: &UpdateRecurringExpenseInput,
) -> Result<RecurringExpenseItem> {
    let current = query_recurring_item(transaction, recurring_expense_id, true)?;
    if current.version != input.expected_version {
        return Err(Error::Conflict(format!(
            "recurring expense {recurring_expense_id} changed from version {} to {}",
            input.expected_version, current.version
        )));
    }
    if input.auto_match_enabled && !current.auto_match_enabled {
        return Err(invalid(
            "recurring auto-match can only be enabled by confirming a matched transaction",
        ));
    }
    let now = now_utc();
    let next_version = current
        .version
        .checked_add(1)
        .ok_or_else(|| Error::Invariant("recurring expense version overflowed".to_owned()))?;
    let effective_month = current_month_start()?;
    let effective_now = input.effective_from_month == effective_month;
    let changed = if effective_now {
        transaction.execute(
            "UPDATE recurring_expense_items
             SET name_key_version = ?2, name_nonce = ?3, name_ciphertext = ?4,
                 name_aad = ?5, name_blind_index = ?6, category = ?7,
                 vendor_key_version = ?8, vendor_nonce = ?9,
                 vendor_ciphertext = ?10, vendor_aad = ?11,
                 vendor_blind_index = ?12, amount_minor = ?13, currency = ?14,
                 payment_method_fingerprint = ?15, start_date = ?16,
                 end_date = ?17, memo_key_version = ?18, memo_nonce = ?19,
                 memo_ciphertext = ?20, memo_aad = ?21, memo_blind_index = ?22,
                 reminder_days = ?23, amount_kind = ?24, interval_months = ?25,
                 due_rule = ?26, due_day = ?27, item_status = ?28,
                 updated_at = ?29, version = version + 1
             WHERE id = ?1 AND version = ?30 AND deleted_at IS NULL",
            params![
                recurring_expense_id,
                input.name.key_version,
                input.name.nonce,
                input.name.ciphertext,
                input.name.aad,
                input.name.blind_index,
                input.category.as_str(),
                envelope_key_version(&input.vendor),
                envelope_value(&input.vendor, |value| &value.nonce),
                envelope_value(&input.vendor, |value| &value.ciphertext),
                envelope_value(&input.vendor, |value| &value.aad),
                envelope_value(&input.vendor, |value| &value.blind_index),
                input.amount_minor,
                input.currency,
                input.payment_method_fingerprint,
                input.start_date,
                input.end_date,
                envelope_key_version(&input.memo),
                envelope_value(&input.memo, |value| &value.nonce),
                envelope_value(&input.memo, |value| &value.ciphertext),
                envelope_value(&input.memo, |value| &value.aad),
                envelope_value(&input.memo, |value| &value.blind_index),
                input.reminder_days,
                input.amount_kind.as_str(),
                input.interval_months,
                input.due_rule.as_str(),
                input.due_day,
                input.status.as_str(),
                now,
                input.expected_version,
            ],
        )?
    } else {
        transaction.execute(
            "UPDATE recurring_expense_items
             SET updated_at = ?2, version = version + 1
             WHERE id = ?1 AND version = ?3 AND deleted_at IS NULL",
            params![recurring_expense_id, now, input.expected_version],
        )?
    };
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "recurring expense {recurring_expense_id} changed before the update completed"
        )));
    }
    if !input.auto_match_enabled {
        transaction.execute(
            "UPDATE recurring_expense_items SET auto_match_enabled = 0 WHERE id = ?1",
            [recurring_expense_id],
        )?;
        transaction.execute(
            "DELETE FROM expense_rules
             WHERE rule_kind = 'recurring_match' AND recurring_expense_id = ?1",
            [recurring_expense_id],
        )?;
    }
    let scheduled = RecurringExpenseItem {
        id: current.id,
        name: input.name.clone(),
        category: input.category,
        vendor: input.vendor.clone(),
        amount_minor: input.amount_minor,
        currency: input.currency.clone(),
        payment_method_fingerprint: input.payment_method_fingerprint.clone(),
        start_date: input.start_date,
        end_date: input.end_date,
        memo: input.memo.clone(),
        reminder_days: input.reminder_days,
        amount_kind: input.amount_kind,
        interval_months: input.interval_months,
        due_rule: input.due_rule,
        due_day: input.due_day,
        status: input.status,
        auto_match_enabled: input.auto_match_enabled,
        created_at: current.created_at,
        updated_at: now,
        version: next_version,
    };
    insert_recurring_version(transaction, &scheduled, input.effective_from_month)?;
    if effective_now {
        return Ok(scheduled);
    }
    let mut visible = recurring_item_for_month(transaction, recurring_expense_id, effective_month)?
        .unwrap_or_else(|| scheduled.clone());
    visible.version = scheduled.version;
    visible.updated_at = scheduled.updated_at;
    Ok(visible)
}

fn delete_recurring_in_transaction(
    transaction: &Transaction<'_>,
    recurring_expense_id: &str,
    expected_version: u64,
) -> Result<()> {
    let current = query_recurring_item(transaction, recurring_expense_id, true)?;
    if current.version != expected_version {
        return Err(Error::Conflict(format!(
            "recurring expense {recurring_expense_id} changed from version {expected_version} to {}",
            current.version
        )));
    }
    let now = now_utc();
    let mut tombstone =
        recurring_item_for_month(transaction, recurring_expense_id, current_month_start()?)?
            .unwrap_or_else(|| current.clone());
    let changed = transaction.execute(
        "UPDATE recurring_expense_items
         SET item_status = 'ended', deleted_at = ?2, updated_at = ?2,
             version = version + 1
         WHERE id = ?1 AND version = ?3 AND deleted_at IS NULL",
        params![recurring_expense_id, now, expected_version],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "recurring expense {recurring_expense_id} changed before deletion completed"
        )));
    }
    tombstone.status = RecurringExpenseStatus::Ended;
    tombstone.updated_at = now;
    tombstone.version = current
        .version
        .checked_add(1)
        .ok_or_else(|| Error::Invariant("recurring expense version overflowed".to_owned()))?;
    insert_recurring_version(transaction, &tombstone, current_month_start()?)?;
    Ok(())
}

fn insert_recurring_version(
    transaction: &Transaction<'_>,
    item: &RecurringExpenseItem,
    effective_from_month: NaiveDate,
) -> Result<()> {
    validate_month_start(effective_from_month)?;
    transaction.execute(
        "INSERT INTO recurring_expense_versions(
            id, recurring_expense_id, item_version, effective_from_month,
            snapshot_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new_id(),
            item.id,
            item.version,
            effective_from_month,
            serde_json::to_string(item)?,
            now_utc(),
        ],
    )?;
    Ok(())
}

fn recurring_item_select() -> &'static str {
    "SELECT id,
            name_key_version, name_nonce, name_ciphertext, name_aad,
            name_blind_index, category,
            vendor_key_version, vendor_nonce, vendor_ciphertext, vendor_aad,
            vendor_blind_index, amount_minor, currency,
            payment_method_fingerprint, start_date, end_date,
            memo_key_version, memo_nonce, memo_ciphertext, memo_aad,
            memo_blind_index, reminder_days, amount_kind, interval_months,
            due_rule, due_day, item_status, auto_match_enabled,
            created_at, updated_at, version
     FROM recurring_expense_items"
}

fn query_recurring_item(
    connection: &Connection,
    recurring_expense_id: &str,
    include_deleted: bool,
) -> Result<RecurringExpenseItem> {
    let query = format!(
        "{} WHERE id = ?1 AND (?2 = 1 OR deleted_at IS NULL)",
        recurring_item_select()
    );
    connection
        .query_row(
            &query,
            params![recurring_expense_id, include_deleted],
            map_recurring_item,
        )
        .optional()?
        .ok_or_else(|| not_found("recurring expense", recurring_expense_id))
        .and_then(validate_loaded_recurring_item)
}

fn map_recurring_item(row: &Row<'_>) -> rusqlite::Result<RecurringExpenseItem> {
    let name = encrypted_from_row(row, 1)?.ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Null,
            Box::new(Error::Invariant(
                "recurring expense name envelope is missing".to_owned(),
            )),
        )
    })?;
    Ok(RecurringExpenseItem {
        id: row.get(0)?,
        name,
        category: parse_db_enum(row.get::<_, String>(6)?, 6)?,
        vendor: encrypted_from_row(row, 7)?,
        amount_minor: row.get(12)?,
        currency: row.get(13)?,
        payment_method_fingerprint: row.get(14)?,
        start_date: row.get(15)?,
        end_date: row.get(16)?,
        memo: encrypted_from_row(row, 17)?,
        reminder_days: row.get(22)?,
        amount_kind: parse_db_enum(row.get::<_, String>(23)?, 23)?,
        interval_months: row.get(24)?,
        due_rule: parse_db_enum(row.get::<_, String>(25)?, 25)?,
        due_day: row.get(26)?,
        status: parse_db_enum(row.get::<_, String>(27)?, 27)?,
        auto_match_enabled: row.get(28)?,
        created_at: row.get(29)?,
        updated_at: row.get(30)?,
        version: row.get(31)?,
    })
}

fn validate_loaded_recurring_item(item: RecurringExpenseItem) -> Result<RecurringExpenseItem> {
    validate_encrypted_text("recurring expense name", &item.name)?;
    if let Some(value) = item.vendor.as_ref() {
        validate_encrypted_text("recurring expense vendor", value)?;
    }
    if let Some(value) = item.memo.as_ref() {
        validate_encrypted_text("recurring expense memo", value)?;
    }
    Ok(item)
}

fn recurring_occurrences_in_connection(
    connection: &Connection,
    requested_month: NaiveDate,
) -> Result<Vec<RecurringExpenseOccurrence>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT recurring_expense_id
         FROM recurring_expense_versions
         WHERE effective_from_month <= ?1
         ORDER BY recurring_expense_id",
    )?;
    let ids = statement
        .query_map([requested_month], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut occurrences = Vec::new();
    for id in ids {
        let Some(item) = recurring_item_for_month(connection, &id, requested_month)? else {
            continue;
        };
        if let Some(occurrence) = virtual_occurrence_for_item(connection, &item, requested_month)? {
            occurrences.push(occurrence);
        }
    }
    occurrences.sort_by(|left, right| {
        left.due_date
            .cmp(&right.due_date)
            .then_with(|| left.occurrence_key.cmp(&right.occurrence_key))
    });
    Ok(occurrences)
}

fn recurring_item_for_month(
    connection: &Connection,
    recurring_expense_id: &str,
    requested_month: NaiveDate,
) -> Result<Option<RecurringExpenseItem>> {
    let snapshot = connection
        .query_row(
            "SELECT snapshot_json
             FROM recurring_expense_versions
             WHERE recurring_expense_id = ?1 AND effective_from_month <= ?2
             ORDER BY item_version DESC, effective_from_month DESC
             LIMIT 1",
            params![recurring_expense_id, requested_month],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|json| serde_json::from_str::<RecurringExpenseItem>(&json))
        .transpose()?;
    let Some(mut snapshot) = snapshot else {
        return Ok(None);
    };
    // Auto-match is revocable consent, not historical schedule data.
    snapshot.auto_match_enabled = connection.query_row(
        "SELECT auto_match_enabled FROM recurring_expense_items WHERE id = ?1",
        [recurring_expense_id],
        |row| row.get(0),
    )?;
    Ok(Some(snapshot))
}

fn virtual_occurrence_for_item(
    connection: &Connection,
    item: &RecurringExpenseItem,
    requested_month: NaiveDate,
) -> Result<Option<RecurringExpenseOccurrence>> {
    let occurrence_key = recurring_occurrence_key(&item.id, requested_month);
    let stored = connection
        .query_row(
            "SELECT item_version, due_date, expected_amount_minor, currency,
                    occurrence_status, actual_amount_minor, actual_event_id,
                    manual_event_id, amount_changed, version
             FROM recurring_expense_occurrences WHERE occurrence_key = ?1",
            [&occurrence_key],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, NaiveDate>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, u64>(9)?,
                ))
            },
        )
        .optional()?;
    if let Some((
        item_version,
        due_date,
        expected_amount_minor,
        currency,
        status,
        actual_amount_minor,
        actual_event_id,
        manual_event_id,
        amount_changed,
        version,
    )) = stored
    {
        let versioned_item = recurring_item_for_version(connection, &item.id, item_version)?;
        let status = match status.as_str() {
            "paid" => RecurringOccurrenceStatus::Paid,
            "matched" => RecurringOccurrenceStatus::Matched,
            other => {
                return Err(Error::Invariant(format!(
                    "unknown stored recurring occurrence status: {other}"
                )));
            }
        };
        return Ok(Some(RecurringExpenseOccurrence {
            occurrence_key,
            recurring_expense_id: item.id.clone(),
            item_version,
            name: versioned_item.name,
            category: versioned_item.category,
            vendor: versioned_item.vendor,
            due_date,
            expected_amount_minor,
            actual_amount_minor: Some(actual_amount_minor),
            currency,
            status,
            reminder_days: versioned_item.reminder_days,
            amount_changed,
            actual_event_id: actual_event_id.or(manual_event_id),
            version,
        }));
    }
    if item.status != RecurringExpenseStatus::Active {
        return Ok(None);
    }
    let start_month = month_start(item.start_date)?;
    if requested_month < start_month {
        return Ok(None);
    }
    let difference = month_difference(start_month, requested_month)?;
    if difference % i32::from(item.interval_months) != 0 {
        return Ok(None);
    }
    let due_date = recurring_due_date(item, requested_month)?;
    if due_date < item.start_date || item.end_date.is_some_and(|end| due_date > end) {
        return Ok(None);
    }
    Ok(Some(RecurringExpenseOccurrence {
        occurrence_key,
        recurring_expense_id: item.id.clone(),
        item_version: item.version,
        name: item.name.clone(),
        category: item.category,
        vendor: item.vendor.clone(),
        due_date,
        expected_amount_minor: item.amount_minor,
        actual_amount_minor: None,
        currency: item.currency.clone(),
        status: calculated_occurrence_status(due_date, item.reminder_days),
        reminder_days: item.reminder_days,
        amount_changed: false,
        actual_event_id: None,
        version: 1,
    }))
}

fn recurring_item_for_version(
    connection: &Connection,
    recurring_expense_id: &str,
    item_version: u64,
) -> Result<RecurringExpenseItem> {
    connection
        .query_row(
            "SELECT snapshot_json
             FROM recurring_expense_versions
             WHERE recurring_expense_id = ?1 AND item_version = ?2",
            params![recurring_expense_id, item_version],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| {
            Error::Invariant(format!(
                "recurring expense {recurring_expense_id} is missing version {item_version}"
            ))
        })
        .and_then(|json| serde_json::from_str(&json).map_err(Into::into))
}

fn confirm_recurring_paid_in_transaction(
    transaction: &Transaction<'_>,
    occurrence_key: &str,
    input: &ConfirmRecurringPaidInput,
) -> Result<RecurringExpenseOccurrence> {
    let (recurring_expense_id, requested_month) = parse_occurrence_key(occurrence_key)?;
    let item = recurring_item_for_month(transaction, recurring_expense_id, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))?;
    let occurrence = virtual_occurrence_for_item(transaction, &item, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))?;
    if occurrence.version != input.expected_version {
        return Err(Error::Conflict(format!(
            "recurring occurrence {occurrence_key} changed from version {} to {}",
            input.expected_version, occurrence.version
        )));
    }
    if matches!(
        occurrence.status,
        RecurringOccurrenceStatus::Paid | RecurringOccurrenceStatus::Matched
    ) {
        return Err(Error::Conflict(format!(
            "recurring occurrence {occurrence_key} is already paid"
        )));
    }
    let amount = input.amount_minor.unwrap_or(item.amount_minor);
    validate_amount_minor("recurring paid amountMinor", amount)?;
    let paid_date = input.paid_date.unwrap_or_else(today_seoul);
    let event_id = new_id();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO expense_events(
            id, event_kind, category, event_status, amount_minor, currency,
            occurred_at, posted_date, is_provisional, created_at, updated_at
         ) VALUES (?1, 'manual_recurring', ?2, 'confirmed', ?3, ?4,
                   ?5, ?6, 1, ?7, ?7)",
        params![
            event_id,
            item.category.as_str(),
            amount,
            item.currency,
            format!("{paid_date}T00:00:00+09:00"),
            paid_date,
            now,
        ],
    )?;
    transaction.execute(
        "INSERT INTO recurring_expense_occurrences(
            occurrence_key, recurring_expense_id, item_version, due_date,
            expected_amount_minor, currency, occurrence_status,
            actual_amount_minor, manual_event_id, amount_changed,
            confirmed_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'paid', ?7, ?8, ?9, ?10, ?10)",
        params![
            occurrence_key,
            item.id,
            item.version,
            occurrence.due_date,
            item.amount_minor,
            item.currency,
            amount,
            event_id,
            amount != item.amount_minor,
            now,
        ],
    )?;
    virtual_occurrence_for_item(transaction, &item, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))
}

fn match_recurring_in_transaction(
    transaction: &Transaction<'_>,
    occurrence_key: &str,
    input: &MatchRecurringExpenseInput,
) -> Result<RecurringExpenseOccurrence> {
    let (recurring_expense_id, requested_month) = parse_occurrence_key(occurrence_key)?;
    let item = recurring_item_for_month(transaction, recurring_expense_id, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))?;
    let occurrence = virtual_occurrence_for_item(transaction, &item, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))?;
    if occurrence.version != input.expected_version {
        return Err(Error::Conflict(format!(
            "recurring occurrence {occurrence_key} changed from version {} to {}",
            input.expected_version, occurrence.version
        )));
    }
    if occurrence.status == RecurringOccurrenceStatus::Matched {
        if occurrence.actual_event_id.as_deref() == Some(input.event_id.as_str()) {
            return Ok(occurrence);
        }
        return Err(Error::Conflict(format!(
            "recurring occurrence {occurrence_key} is already matched"
        )));
    }
    if let Some(other_occurrence) = transaction
        .query_row(
            "SELECT occurrence_key
             FROM recurring_expense_occurrences
             WHERE actual_event_id = ?1 AND occurrence_key != ?2
             LIMIT 1",
            params![input.event_id, occurrence_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Err(Error::Conflict(format!(
            "expense event {} is already matched to recurring occurrence {other_occurrence}",
            input.event_id
        )));
    }
    let event = query_expense_transaction(transaction, &input.event_id)?;
    let posting_direction = transaction
        .query_row(
            "SELECT p.direction
             FROM expense_events AS e
             LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
             WHERE e.id = ?1",
            [&input.event_id],
            |row| row.get::<_, Option<String>>(0),
        )?
        .and_then(|direction| ExpenseDirection::from_str(&direction).ok());
    if event.is_provisional
        || posting_direction != Some(ExpenseDirection::Debit)
        || !matches!(
            event.kind,
            ExpenseEventKind::Purchase
                | ExpenseEventKind::ExternalTransfer
                | ExpenseEventKind::UnknownP2p
        )
    {
        return Err(invalid(
            "a recurring occurrence requires a non-provisional debit purchase candidate",
        ));
    }
    if event.currency != item.currency {
        return Err(invalid(
            "recurring occurrence and matched transaction currencies must agree",
        ));
    }
    if event.status == ExpenseEventStatus::Excluded {
        return Err(invalid("an excluded expense transaction cannot be matched"));
    }
    let now = now_utc();
    let stored = transaction
        .query_row(
            "SELECT manual_event_id FROM recurring_expense_occurrences
             WHERE occurrence_key = ?1",
            [occurrence_key],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?;
    if let Some(Some(manual_event_id)) = stored {
        transaction.execute(
            "UPDATE expense_events
             SET event_status = 'excluded', exclusion_reason = 'manual_replaced',
                 updated_at = ?2, version = version + 1
             WHERE id = ?1",
            params![manual_event_id, now],
        )?;
        transaction.execute(
            "UPDATE recurring_expense_occurrences
             SET occurrence_status = 'matched', actual_amount_minor = ?2,
                 actual_event_id = ?3, manual_event_id = NULL,
                 amount_changed = ?4, updated_at = ?5, version = version + 1
             WHERE occurrence_key = ?1",
            params![
                occurrence_key,
                event.amount_minor,
                event.id,
                event.amount_minor != item.amount_minor,
                now,
            ],
        )?;
    } else {
        transaction.execute(
            "INSERT INTO recurring_expense_occurrences(
                occurrence_key, recurring_expense_id, item_version, due_date,
                expected_amount_minor, currency, occurrence_status,
                actual_amount_minor, actual_event_id, amount_changed,
                confirmed_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'matched', ?7, ?8, ?9, ?10, ?10)",
            params![
                occurrence_key,
                item.id,
                item.version,
                occurrence.due_date,
                item.amount_minor,
                item.currency,
                event.amount_minor,
                event.id,
                event.amount_minor != item.amount_minor,
                now,
            ],
        )?;
    }
    transaction.execute(
        "UPDATE expense_events
         SET event_kind = 'purchase', category = ?2, event_status = 'confirmed',
             exclusion_reason = NULL, duplicate_of_event_id = NULL,
             updated_at = ?3, version = version + 1
         WHERE id = ?1",
        params![event.id, item.category.as_str(), now],
    )?;
    transaction.execute(
        "UPDATE expense_reviews
         SET review_status = 'resolved', resolved_kind = 'purchase',
             resolved_category = ?2, resolved_at = ?3, version = version + 1
         WHERE event_id = ?1 AND review_status = 'pending'",
        params![event.id, item.category.as_str(), now],
    )?;
    if input.enable_future_auto_match {
        let head = query_recurring_item(transaction, &item.id, false)?;
        let effective_month = current_month_start()?;
        let mut current = recurring_item_for_month(transaction, &item.id, effective_month)?
            .unwrap_or_else(|| head.clone());
        if head.version != current.version {
            return Err(Error::Conflict(
                "future auto-match can only be enabled from the current recurring version"
                    .to_owned(),
            ));
        }
        let event_vendor = event.merchant.as_ref().ok_or_else(|| {
            invalid("future auto-match requires an encrypted transaction merchant")
        })?;
        if let Some(vendor) = current.vendor.as_ref() {
            if vendor.blind_index != event_vendor.blind_index {
                return Err(invalid(
                    "matched transaction merchant does not match the recurring vendor",
                ));
            }
        }
        let event_payment_method = event.payment_method_fingerprint.as_ref().ok_or_else(|| {
            invalid("future auto-match requires a transaction payment method fingerprint")
        })?;
        let payment_method_fingerprint = match current.payment_method_fingerprint.as_ref() {
            Some(value) if value != event_payment_method => {
                return Err(invalid(
                    "matched transaction payment method does not match the recurring item",
                ));
            }
            Some(value) => value.clone(),
            None => event_payment_method.clone(),
        };
        transaction.execute(
            "INSERT OR IGNORE INTO expense_rules(
                id, rule_kind, merchant_blind_index, payment_method_fingerprint,
                event_kind, category, recurring_expense_id, created_at
             ) VALUES (?1, 'recurring_match', ?2, ?3, 'purchase', ?4, ?5, ?6)",
            params![
                new_id(),
                event_vendor.blind_index,
                payment_method_fingerprint,
                current.category.as_str(),
                head.id,
                now,
            ],
        )?;
        let changed = transaction.execute(
            "UPDATE recurring_expense_items
             SET payment_method_fingerprint = ?2,
                 auto_match_enabled = 1, updated_at = ?3, version = version + 1
             WHERE id = ?1 AND version = ?4",
            params![head.id, payment_method_fingerprint, now, head.version,],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "recurring expense {} changed before auto-match was enabled",
                head.id
            )));
        }
        current.payment_method_fingerprint = Some(payment_method_fingerprint);
        current.auto_match_enabled = true;
        current.updated_at = now;
        current.version = head
            .version
            .checked_add(1)
            .ok_or_else(|| Error::Invariant("recurring expense version overflowed".to_owned()))?;
        insert_recurring_version(transaction, &current, effective_month)?;
    }
    virtual_occurrence_for_item(transaction, &item, requested_month)?
        .ok_or_else(|| not_found("recurring expense occurrence", occurrence_key))
}

fn recurring_amount_matches(item: &RecurringExpenseItem, actual_amount_minor: i64) -> bool {
    match item.amount_kind {
        RecurringAmountKind::Fixed => actual_amount_minor == item.amount_minor,
        RecurringAmountKind::Estimate => {
            let tolerance = (item.amount_minor / 20).max(1);
            actual_amount_minor.abs_diff(item.amount_minor) <= tolerance as u64
        }
        RecurringAmountKind::Limit => actual_amount_minor <= item.amount_minor,
    }
}

fn recurring_due_date(
    item: &RecurringExpenseItem,
    requested_month: NaiveDate,
) -> Result<NaiveDate> {
    let last = last_day_of_month(requested_month.year(), requested_month.month())?;
    let day = match item.due_rule {
        RecurringDueRule::FirstDay => 1,
        RecurringDueRule::LastDay => last.day(),
        RecurringDueRule::SpecificDay => u32::from(
            item.due_day
                .ok_or_else(|| invalid("specific recurring due day is missing"))?,
        )
        .min(last.day()),
    };
    NaiveDate::from_ymd_opt(requested_month.year(), requested_month.month(), day)
        .ok_or_else(|| invalid("recurring expense due date is outside the supported range"))
}

fn recurring_occurrence_key(recurring_expense_id: &str, requested_month: NaiveDate) -> String {
    format!(
        "{recurring_expense_id}:{:04}-{:02}",
        requested_month.year(),
        requested_month.month()
    )
}

fn parse_occurrence_key(value: &str) -> Result<(&str, NaiveDate)> {
    let (id, month) = value
        .rsplit_once(':')
        .ok_or_else(|| invalid("recurring occurrence key is invalid"))?;
    if id.is_empty() || month.len() != 7 {
        return Err(invalid("recurring occurrence key is invalid"));
    }
    let date = NaiveDate::parse_from_str(&format!("{month}-01"), "%Y-%m-%d")
        .map_err(|_| invalid("recurring occurrence month is invalid"))?;
    Ok((id, date))
}

fn calculated_occurrence_status(
    due_date: NaiveDate,
    reminder_days: u8,
) -> RecurringOccurrenceStatus {
    calculated_occurrence_status_on(due_date, today_seoul(), reminder_days)
}

fn calculated_occurrence_status_on(
    due_date: NaiveDate,
    today: NaiveDate,
    reminder_days: u8,
) -> RecurringOccurrenceStatus {
    if due_date < today {
        RecurringOccurrenceStatus::Overdue
    } else if due_date == today {
        RecurringOccurrenceStatus::DueToday
    } else if due_date <= today + Duration::days(i64::from(reminder_days)) {
        RecurringOccurrenceStatus::DueSoon
    } else {
        RecurringOccurrenceStatus::Scheduled
    }
}

#[cfg(test)]
mod expense_unit_tests {
    use chrono::NaiveDate;

    use super::{RecurringOccurrenceStatus, calculated_occurrence_status_on};

    #[test]
    fn recurring_due_soon_uses_the_items_reminder_window() {
        let today = NaiveDate::from_ymd_opt(2026, 8, 1).expect("valid date");
        let due = NaiveDate::from_ymd_opt(2026, 8, 5).expect("valid date");
        assert_eq!(
            calculated_occurrence_status_on(due, today, 2),
            RecurringOccurrenceStatus::Scheduled
        );
        assert_eq!(
            calculated_occurrence_status_on(due, today, 7),
            RecurringOccurrenceStatus::DueSoon
        );
        assert_eq!(
            calculated_occurrence_status_on(today, today, 0),
            RecurringOccurrenceStatus::DueToday
        );
    }
}

#[derive(Default)]
struct CurrencyAccumulator {
    net: i128,
    purchases: i128,
    refunds: i128,
    settlement_received: i128,
    settlement_sent: i128,
    fees: i128,
    unconfirmed_outflow: i128,
    recurring_expected: i128,
    recurring_paid: i128,
    recurring_remaining: i128,
}

fn add_financial_aggregate(target: &mut i128, value: i128, name: &str) -> Result<()> {
    *target = target
        .checked_add(value)
        .ok_or_else(|| Error::Invariant(format!("expense {name} aggregate overflowed")))?;
    Ok(())
}

fn safe_summary_amount(name: &str, value: i128) -> Result<i64> {
    if value < -i128::from(MAX_SAFE_AMOUNT_MINOR) || value > i128::from(MAX_SAFE_AMOUNT_MINOR) {
        return Err(Error::Invariant(format!(
            "expense {name} aggregate exceeds the safe amountMinor range"
        )));
    }
    i64::try_from(value)
        .map_err(|_| Error::Invariant(format!("expense {name} aggregate does not fit in i64")))
}

fn expense_source_coverage_counts(
    connection: &Connection,
    requested_month: NaiveDate,
    month_end: NaiveDate,
) -> Result<(u32, u32, u32)> {
    let mut source_statement = connection.prepare(
        "SELECT id FROM expense_sources
         WHERE is_active = 1 AND required_for_complete_report = 1
           AND coverage_start <= ?1
         ORDER BY id",
    )?;
    let source_ids = source_statement
        .query_map([month_end], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let active_source_count = u32::try_from(source_ids.len())
        .map_err(|_| Error::Invariant("too many active expense sources".to_owned()))?;
    let mut present_source_count = 0_u32;
    let mut covered_source_count = 0_u32;
    let mut coverage_statement = connection.prepare(
        "SELECT coverage_start, coverage_end
         FROM expense_import_batches
         WHERE source_id = ?1
           AND coverage_end >= ?2 AND coverage_start <= ?3
         ORDER BY coverage_start, coverage_end",
    )?;
    for source_id in source_ids {
        let ranges = coverage_statement
            .query_map(params![source_id, requested_month, month_end], |row| {
                Ok((row.get::<_, NaiveDate>(0)?, row.get::<_, NaiveDate>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ranges.is_empty() {
            continue;
        }
        present_source_count = present_source_count.saturating_add(1);
        let mut next_uncovered = requested_month;
        let mut fully_covered = false;
        for (start, end) in ranges {
            let start = start.max(requested_month);
            let end = end.min(month_end);
            if end < next_uncovered {
                continue;
            }
            if start > next_uncovered {
                break;
            }
            if end >= month_end {
                fully_covered = true;
                break;
            }
            next_uncovered = end
                .succ_opt()
                .ok_or_else(|| Error::Invariant("expense coverage date overflowed".to_owned()))?;
        }
        if fully_covered {
            covered_source_count = covered_source_count.saturating_add(1);
        }
    }
    Ok((
        active_source_count,
        present_source_count,
        covered_source_count,
    ))
}

impl TmCore {
    pub fn get_expense_month_summary(
        &self,
        requested_month: NaiveDate,
    ) -> Result<ExpenseMonthSummary> {
        validate_month_start(requested_month)?;
        let connection = self.database.connect()?;
        expense_month_summary_in_connection(&connection, requested_month)
    }

    pub fn expense_report_aggregate(
        &self,
        requested_month: NaiveDate,
    ) -> Result<ExpenseReportAggregate> {
        let summary = self.get_expense_month_summary(requested_month)?;
        let previous_summary = self.get_expense_month_summary(previous_month(requested_month)?)?;
        let mut facts = Vec::new();
        for currency in &summary.currencies {
            for (metric, amount_minor) in [
                ("net_personal_spend", currency.net_personal_spend_minor),
                ("gross_purchase", currency.gross_purchase_minor),
                ("refunds", currency.refunds_minor),
                ("settlement_received", currency.settlement_received_minor),
                ("settlement_sent", currency.settlement_sent_minor),
                ("fees", currency.fees_minor),
                ("unconfirmed_outflow", currency.unconfirmed_outflow_minor),
                ("recurring_expected", currency.recurring_expected_minor),
                ("recurring_paid", currency.recurring_paid_minor),
                ("recurring_remaining", currency.recurring_remaining_minor),
            ] {
                facts.push(ExpenseReportFact {
                    fact_id: format!("currency:{}:{metric}", currency.currency),
                    metric: metric.to_owned(),
                    currency: currency.currency.clone(),
                    amount_minor,
                });
            }
        }
        for category in &summary.categories {
            facts.push(ExpenseReportFact {
                fact_id: format!(
                    "category:{}:{}",
                    category.currency,
                    category.category.as_str()
                ),
                metric: format!("category.{}", category.category.as_str()),
                currency: category.currency.clone(),
                amount_minor: category.amount_minor,
            });
        }
        if summary.status == ExpenseReportStatus::Confirmed
            && previous_summary.status == ExpenseReportStatus::Confirmed
        {
            let previous_currencies = previous_summary
                .currencies
                .iter()
                .map(|currency| (currency.currency.as_str(), currency))
                .collect::<BTreeMap<_, _>>();
            for currency in &summary.currencies {
                let Some(previous) = previous_currencies.get(currency.currency.as_str()) else {
                    continue;
                };
                for (metric, current_amount, previous_amount) in [
                    (
                        "net_personal_spend",
                        currency.net_personal_spend_minor,
                        previous.net_personal_spend_minor,
                    ),
                    (
                        "gross_purchase",
                        currency.gross_purchase_minor,
                        previous.gross_purchase_minor,
                    ),
                    ("refunds", currency.refunds_minor, previous.refunds_minor),
                    (
                        "settlement_received",
                        currency.settlement_received_minor,
                        previous.settlement_received_minor,
                    ),
                    (
                        "settlement_sent",
                        currency.settlement_sent_minor,
                        previous.settlement_sent_minor,
                    ),
                    ("fees", currency.fees_minor, previous.fees_minor),
                    (
                        "unconfirmed_outflow",
                        currency.unconfirmed_outflow_minor,
                        previous.unconfirmed_outflow_minor,
                    ),
                ] {
                    facts.push(ExpenseReportFact {
                        fact_id: format!("delta:currency:{}:{metric}", currency.currency),
                        metric: format!("delta.{metric}"),
                        currency: currency.currency.clone(),
                        amount_minor: safe_summary_amount(
                            "currency delta",
                            i128::from(current_amount) - i128::from(previous_amount),
                        )?,
                    });
                }
            }
            let mut category_deltas: BTreeMap<(String, ExpenseCategory), (i64, i64)> =
                BTreeMap::new();
            for current in &summary.categories {
                category_deltas
                    .entry((current.currency.clone(), current.category))
                    .or_default()
                    .0 = current.amount_minor;
            }
            for previous in &previous_summary.categories {
                category_deltas
                    .entry((previous.currency.clone(), previous.category))
                    .or_default()
                    .1 = previous.amount_minor;
            }
            for ((currency, category), (current_amount, previous_amount)) in category_deltas {
                facts.push(ExpenseReportFact {
                    fact_id: format!("delta:category:{currency}:{}", category.as_str()),
                    metric: format!("delta.category.{}", category.as_str()),
                    currency,
                    amount_minor: safe_summary_amount(
                        "category delta",
                        i128::from(current_amount) - i128::from(previous_amount),
                    )?,
                });
            }
        }
        facts.sort_by(|left, right| left.fact_id.cmp(&right.fact_id));
        let canonical = serde_json::to_vec(&(
            requested_month,
            summary.status,
            &facts,
            &summary.completeness,
            previous_summary.status,
            &previous_summary.completeness,
        ))?;
        let aggregate_sha256 = format!("{:x}", Sha256::digest(canonical));
        Ok(ExpenseReportAggregate {
            month_start: requested_month,
            aggregate_sha256,
            report_status: summary.status,
            facts,
        })
    }

    pub fn get_cached_expense_report(
        &self,
        aggregate_sha256: &str,
        prompt_version: &str,
    ) -> Result<Option<ExpenseReportResult>> {
        validate_sha256("expense aggregate SHA-256", aggregate_sha256)?;
        validate_label("expense report prompt version", prompt_version, 128)?;
        self.database
            .connect()?
            .query_row(
                "SELECT id, month_start, aggregate_sha256, prompt_version, model,
                        result_json, input_tokens, cached_input_tokens,
                        output_tokens, total_tokens, cost_microusd,
                        latency_ms, created_at
                 FROM expense_ai_reports
                 WHERE aggregate_sha256 = ?1 AND prompt_version = ?2",
                params![aggregate_sha256, prompt_version],
                map_expense_report_result,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn get_expense_report(&self, report_id: &str) -> Result<Option<ExpenseReportResult>> {
        validate_label("expense report ID", report_id, 128)?;
        self.database
            .connect()?
            .query_row(
                "SELECT id, month_start, aggregate_sha256, prompt_version, model,
                        result_json, input_tokens, cached_input_tokens,
                        output_tokens, total_tokens, cost_microusd,
                        latency_ms, created_at
                 FROM expense_ai_reports
                 WHERE id = ?1",
                [report_id],
                map_expense_report_result,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn save_expense_report(
        &self,
        input: SaveExpenseReportInput,
    ) -> Result<ExpenseReportResult> {
        validate_save_expense_report(&input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                save_expense_report_in_transaction(transaction, &input)
            })
    }

    pub fn record_expense_report_feedback(&self, report_id: &str, helpful: bool) -> Result<()> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                record_expense_feedback_in_transaction(transaction, report_id, helpful)
            })
    }

    pub fn expense_report_feedback(&self, report_id: &str) -> Result<Option<bool>> {
        validate_label("expense report ID", report_id, 128)?;
        self.database
            .connect()?
            .query_row(
                "SELECT helpful FROM expense_ai_feedback WHERE report_id = ?1",
                [report_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Binds an idempotency-derived request identity without consuming the monthly AI quota.
    /// Cached and uncached report paths must both call this before returning or reserving cost.
    pub fn bind_expense_report_request(
        &self,
        request_id: &str,
        report_month_start: NaiveDate,
        aggregate_sha256: &str,
    ) -> Result<ExpenseReportRequestBinding> {
        validate_label("expense report request ID", request_id, 128)?;
        validate_month_start(report_month_start)?;
        validate_sha256("expense aggregate SHA-256", aggregate_sha256)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some((saved_month, saved_aggregate)) = transaction
                    .query_row(
                        "SELECT report_month_start, aggregate_sha256
                         FROM expense_ai_request_bindings WHERE request_id = ?1",
                        [request_id],
                        |row| Ok((row.get::<_, NaiveDate>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?
                {
                    if saved_month != report_month_start || saved_aggregate != aggregate_sha256 {
                        return Err(Error::Conflict(
                            "expense report request ID was reused for another month or aggregate"
                                .to_owned(),
                        ));
                    }
                    return Ok(ExpenseReportRequestBinding {
                        request_id: request_id.to_owned(),
                        report_month_start,
                        aggregate_sha256: aggregate_sha256.to_owned(),
                        replayed: true,
                    });
                }
                transaction.execute(
                    "INSERT INTO expense_ai_request_bindings(
                        request_id, report_month_start, aggregate_sha256, created_at
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![request_id, report_month_start, aggregate_sha256, now_utc()],
                )?;
                Ok(ExpenseReportRequestBinding {
                    request_id: request_id.to_owned(),
                    report_month_start,
                    aggregate_sha256: aggregate_sha256.to_owned(),
                    replayed: false,
                })
            })
    }

    pub fn claim_expense_report_attempt(
        &self,
        requested_month: NaiveDate,
        request_id: &str,
    ) -> Result<ExpenseReportAttemptClaim> {
        self.claim_expense_report_attempt_for_report(requested_month, requested_month, request_id)
    }

    pub fn claim_expense_report_attempt_for_report(
        &self,
        quota_month: NaiveDate,
        report_month: NaiveDate,
        request_id: &str,
    ) -> Result<ExpenseReportAttemptClaim> {
        validate_month_start(quota_month)?;
        validate_month_start(report_month)?;
        validate_label("expense report request ID", request_id, 128)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some((
                    saved_quota_month,
                    saved_report_month,
                    attempt_number,
                    saved_status,
                    failure_code,
                )) = transaction
                    .query_row(
                        "SELECT month_start, report_month_start, attempt_number,
                                attempt_status, failure_code
                         FROM expense_ai_attempts WHERE request_id = ?1",
                        [request_id],
                        |row| {
                            Ok((
                                row.get::<_, NaiveDate>(0)?,
                                row.get::<_, NaiveDate>(1)?,
                                row.get::<_, u8>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<String>>(4)?,
                            ))
                        },
                    )
                    .optional()?
                {
                    if saved_quota_month != quota_month || saved_report_month != report_month {
                        return Err(Error::Conflict(
                            "expense report request ID was reused for another quota or report month"
                                .to_owned(),
                        ));
                    }
                    return Ok(ExpenseReportAttemptClaim {
                        request_id: request_id.to_owned(),
                        month_start: quota_month,
                        report_month_start: report_month,
                        attempt_number,
                        status: ExpenseReportAttemptStatus::from_str(&saved_status)?,
                        failure_code,
                        replayed: true,
                    });
                }
                let count: u8 = transaction.query_row(
                    "SELECT count(*) FROM expense_ai_attempts WHERE month_start = ?1",
                    [quota_month],
                    |row| row.get(0),
                )?;
                if count >= 8 {
                    return Err(Error::Conflict(
                        "expense AI monthly attempt limit of 8 has been reached".to_owned(),
                    ));
                }
                let attempt_number = count.saturating_add(1);
                transaction.execute(
                    "INSERT INTO expense_ai_attempts(
                        request_id, month_start, report_month_start,
                        attempt_number, created_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        request_id,
                        quota_month,
                        report_month,
                        attempt_number,
                        now_utc()
                    ],
                )?;
                Ok(ExpenseReportAttemptClaim {
                    request_id: request_id.to_owned(),
                    month_start: quota_month,
                    report_month_start: report_month,
                    attempt_number,
                    status: ExpenseReportAttemptStatus::Claimed,
                    failure_code: None,
                    replayed: false,
                })
            })
    }

    pub fn stage_expense_report_attempt_result(
        &self,
        request_id: &str,
        input: &SaveExpenseReportInput,
    ) -> Result<SaveExpenseReportInput> {
        validate_label("expense report request ID", request_id, 128)?;
        validate_save_expense_report(input)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let (report_month_start, attempt_status, stored_json) = transaction
                    .query_row(
                        "SELECT report_month_start, attempt_status, result_json
                         FROM expense_ai_attempts WHERE request_id = ?1",
                        [request_id],
                        |row| {
                            Ok((
                                row.get::<_, NaiveDate>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )
                    .optional()?
                    .ok_or_else(|| not_found("expense report attempt", request_id))?;
                if report_month_start != input.month_start {
                    return Err(Error::Conflict(
                        "expense report attempt month does not match the staged result".to_owned(),
                    ));
                }
                let attempt_status = ExpenseReportAttemptStatus::from_str(&attempt_status)?;
                if let Some(stored_json) = stored_json {
                    let stored: SaveExpenseReportInput = serde_json::from_str(&stored_json)?;
                    if stored == *input {
                        return Ok(stored);
                    }
                    return Err(Error::Conflict(
                        "expense report attempt already has a different staged result".to_owned(),
                    ));
                }
                if attempt_status != ExpenseReportAttemptStatus::Claimed {
                    return Err(Error::Conflict(
                        "a terminal expense report attempt cannot accept a staged result"
                            .to_owned(),
                    ));
                }
                let changed = transaction.execute(
                    "UPDATE expense_ai_attempts
                     SET result_json = ?2, attempt_status = 'succeeded', completed_at = ?3
                     WHERE request_id = ?1 AND attempt_status = 'claimed'
                       AND result_json IS NULL",
                    params![request_id, serde_json::to_string(input)?, now_utc()],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "expense report attempt result changed before it was staged".to_owned(),
                    ));
                }
                Ok(input.clone())
            })
    }

    pub fn get_staged_expense_report_attempt_result(
        &self,
        request_id: &str,
    ) -> Result<Option<SaveExpenseReportInput>> {
        validate_label("expense report request ID", request_id, 128)?;
        self.database
            .connect()?
            .query_row(
                "SELECT result_json FROM expense_ai_attempts WHERE request_id = ?1",
                [request_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
            .map(|json| serde_json::from_str(&json).map_err(Into::into))
            .transpose()
    }

    /// Marks an upstream attempt terminal without permitting a potentially double-billed retry.
    pub fn fail_expense_report_attempt(
        &self,
        request_id: &str,
        failure_code: &str,
    ) -> Result<ExpenseReportAttemptClaim> {
        validate_label("expense report request ID", request_id, 128)?;
        validate_label("expense report failure code", failure_code, 128)?;
        if !EXPENSE_REPORT_FAILURE_CODES.contains(&failure_code) {
            return Err(invalid("expense report failure code is not supported"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let (quota_month, report_month, attempt_number, status, saved_failure) =
                    transaction
                        .query_row(
                            "SELECT month_start, report_month_start, attempt_number,
                                    attempt_status, failure_code
                             FROM expense_ai_attempts WHERE request_id = ?1",
                            [request_id],
                            |row| {
                                Ok((
                                    row.get::<_, NaiveDate>(0)?,
                                    row.get::<_, NaiveDate>(1)?,
                                    row.get::<_, u8>(2)?,
                                    row.get::<_, String>(3)?,
                                    row.get::<_, Option<String>>(4)?,
                                ))
                            },
                        )
                        .optional()?
                        .ok_or_else(|| not_found("expense report attempt", request_id))?;
                let status = ExpenseReportAttemptStatus::from_str(&status)?;
                if status == ExpenseReportAttemptStatus::Succeeded {
                    return Err(Error::Conflict(
                        "a succeeded expense report attempt cannot be marked failed".to_owned(),
                    ));
                }
                if status == ExpenseReportAttemptStatus::Failed {
                    if saved_failure.as_deref() != Some(failure_code) {
                        return Err(Error::Conflict(
                            "expense report attempt already has a different terminal failure"
                                .to_owned(),
                        ));
                    }
                    return Ok(ExpenseReportAttemptClaim {
                        request_id: request_id.to_owned(),
                        month_start: quota_month,
                        report_month_start: report_month,
                        attempt_number,
                        status,
                        failure_code: saved_failure,
                        replayed: true,
                    });
                }
                let changed = transaction.execute(
                    "UPDATE expense_ai_attempts
                     SET attempt_status = 'failed', failure_code = ?2, completed_at = ?3
                     WHERE request_id = ?1 AND attempt_status = 'claimed'",
                    params![request_id, failure_code, now_utc()],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "expense report attempt changed before failure was recorded".to_owned(),
                    ));
                }
                Ok(ExpenseReportAttemptClaim {
                    request_id: request_id.to_owned(),
                    month_start: quota_month,
                    report_month_start: report_month,
                    attempt_number,
                    status: ExpenseReportAttemptStatus::Failed,
                    failure_code: Some(failure_code.to_owned()),
                    replayed: false,
                })
            })
    }
}

pub(crate) fn expense_month_summary_in_connection(
    connection: &Connection,
    requested_month: NaiveDate,
) -> Result<ExpenseMonthSummary> {
    let month_after = next_month(requested_month)?;
    let month_end = month_after
        .pred_opt()
        .ok_or_else(|| invalid("expense month is outside the supported range"))?;
    let mut currencies: BTreeMap<String, CurrencyAccumulator> = BTreeMap::new();
    let mut categories: BTreeMap<(ExpenseCategory, String), i128> = BTreeMap::new();
    let mut daily: BTreeMap<(NaiveDate, String), i128> = BTreeMap::new();
    let mut statement = connection.prepare(
        "SELECT e.event_kind, e.category, e.event_status, e.amount_minor,
                e.currency, e.posted_date, p.direction,
                (SELECT a.amount_minor
                 FROM expense_allocations AS a
                 WHERE a.event_id = e.id AND a.allocation_kind = 'personal'),
                EXISTS(
                    SELECT 1
                    FROM expense_allocations AS link
                    WHERE link.related_event_id = e.id
                      AND link.allocation_kind IN (
                          'settlement_received', 'settlement_sent'
                      )
                ),
                EXISTS(
                    SELECT 1
                    FROM expense_allocations AS link
                    JOIN expense_events AS target
                      ON target.id = link.related_event_id
                     AND target.event_kind = 'purchase'
                     AND target.event_status = 'confirmed'
                    WHERE link.event_id = e.id
                      AND link.allocation_kind IN (
                          'settlement_received', 'settlement_sent'
                      )
                )
         FROM expense_events AS e
         LEFT JOIN expense_postings AS p ON p.id = e.primary_posting_id
         WHERE e.posted_date >= ?1 AND e.posted_date < ?2
         ORDER BY e.posted_date, e.id",
    )?;
    let rows = statement.query_map(params![requested_month, month_after], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, NaiveDate>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, bool>(8)?,
            row.get::<_, bool>(9)?,
        ))
    })?;
    for row in rows {
        let (
            kind,
            category,
            status,
            amount,
            currency,
            date,
            direction,
            personal_amount,
            has_linked_settlements,
            linked_to_confirmed_purchase,
        ) = row?;
        let kind = ExpenseEventKind::from_str(&kind)?;
        let category = ExpenseCategory::from_str(&category)?;
        let status = ExpenseEventStatus::from_str(&status)?;
        let totals = currencies.entry(currency.clone()).or_default();
        let amount = i128::from(amount);
        if status == ExpenseEventStatus::Unconfirmed
            && direction.as_deref() == Some(ExpenseDirection::Debit.as_str())
        {
            add_financial_aggregate(
                &mut totals.unconfirmed_outflow,
                amount,
                "unconfirmed outflow",
            )?;
            continue;
        }
        if status != ExpenseEventStatus::Confirmed {
            continue;
        }
        let net_effect = match kind {
            ExpenseEventKind::Purchase => {
                add_financial_aggregate(&mut totals.purchases, amount, "gross purchases")?;
                // Exact reimbursement intentionally has incoming links but no
                // zero-valued personal allocation row.
                personal_amount
                    .map(i128::from)
                    .unwrap_or(if has_linked_settlements { 0 } else { amount })
            }
            ExpenseEventKind::ManualRecurring => {
                add_financial_aggregate(&mut totals.purchases, amount, "gross purchases")?;
                amount
            }
            ExpenseEventKind::Refund => {
                add_financial_aggregate(&mut totals.refunds, amount, "refunds")?;
                -amount
            }
            ExpenseEventKind::SettlementReceived => {
                add_financial_aggregate(
                    &mut totals.settlement_received,
                    amount,
                    "settlement received",
                )?;
                if linked_to_confirmed_purchase {
                    0
                } else {
                    -amount
                }
            }
            ExpenseEventKind::SettlementSent => {
                add_financial_aggregate(&mut totals.settlement_sent, amount, "settlement sent")?;
                if linked_to_confirmed_purchase {
                    0
                } else {
                    amount
                }
            }
            ExpenseEventKind::Fee => {
                add_financial_aggregate(&mut totals.fees, amount, "fees")?;
                amount
            }
            ExpenseEventKind::CardPayment
            | ExpenseEventKind::WalletTopup
            | ExpenseEventKind::InternalTransfer
            | ExpenseEventKind::ExternalTransfer
            | ExpenseEventKind::UnknownP2p => 0,
        };
        add_financial_aggregate(&mut totals.net, net_effect, "net personal spend")?;
        if net_effect != 0 {
            let category_total = categories.entry((category, currency.clone())).or_default();
            add_financial_aggregate(category_total, net_effect, "category total")?;
            let daily_total = daily.entry((date, currency.clone())).or_default();
            add_financial_aggregate(daily_total, net_effect, "daily total")?;
        }
    }

    for occurrence in recurring_occurrences_in_connection(connection, requested_month)? {
        let totals = currencies.entry(occurrence.currency.clone()).or_default();
        add_financial_aggregate(
            &mut totals.recurring_expected,
            i128::from(occurrence.expected_amount_minor),
            "recurring expected",
        )?;
        match occurrence.status {
            RecurringOccurrenceStatus::Paid | RecurringOccurrenceStatus::Matched => {
                add_financial_aggregate(
                    &mut totals.recurring_paid,
                    i128::from(occurrence.actual_amount_minor.unwrap_or_default()),
                    "recurring paid",
                )?;
            }
            _ => {
                add_financial_aggregate(
                    &mut totals.recurring_remaining,
                    i128::from(occurrence.expected_amount_minor),
                    "recurring remaining",
                )?;
            }
        }
    }

    let (active_source_count, present_source_count, covered_source_count) =
        expense_source_coverage_counts(connection, requested_month, month_end)?;
    let rejected_row_count: u32 = connection.query_row(
        "SELECT coalesce(sum(b.rejected_count), 0)
         FROM expense_import_batches AS b
         WHERE b.coverage_end >= ?1 AND b.coverage_start <= ?2
           AND NOT EXISTS(
               SELECT 1 FROM expense_import_batches AS newer
               WHERE newer.source_id = b.source_id
                 AND newer.coverage_start <= b.coverage_start
                 AND newer.coverage_end >= b.coverage_end
                 AND (newer.created_at > b.created_at
                      OR (newer.created_at = b.created_at AND newer.id > b.id))
           )",
        params![requested_month, month_end],
        |row| row.get(0),
    )?;
    let pending_review_count: u32 = connection.query_row(
        "SELECT count(*)
         FROM expense_reviews AS r
         JOIN expense_events AS e ON e.id = r.event_id
         WHERE r.review_status = 'pending'
           AND r.review_reason IN (
               'unknown_p2p', 'ambiguous_mirror', 'import_rejected'
           )
           AND e.posted_date >= ?1 AND e.posted_date < ?2",
        params![requested_month, month_after],
        |row| row.get(0),
    )?;
    let recurring_candidates: u32 = connection.query_row(
        "SELECT count(*)
         FROM expense_reviews AS r
         JOIN expense_events AS e ON e.id = r.event_id
         WHERE r.review_status = 'pending'
           AND r.review_reason IN (
               'recurring_registration_candidate', 'recurring_match_candidate'
           )
           AND e.posted_date >= ?1 AND e.posted_date < ?2",
        params![requested_month, month_after],
        |row| row.get(0),
    )?;
    let unconfirmed_event_count: u32 = connection.query_row(
        "SELECT count(*) FROM expense_events
         WHERE event_status = 'unconfirmed'
           AND posted_date >= ?1 AND posted_date < ?2",
        params![requested_month, month_after],
        |row| row.get(0),
    )?;
    let report_status = if active_source_count == 0
        || present_source_count < active_source_count
        || rejected_row_count > 0
    {
        ExpenseReportStatus::Incomplete
    } else if covered_source_count < active_source_count
        || pending_review_count > 0
        || unconfirmed_event_count > 0
    {
        ExpenseReportStatus::Provisional
    } else {
        ExpenseReportStatus::Confirmed
    };

    Ok(ExpenseMonthSummary {
        month: format!(
            "{:04}-{:02}",
            requested_month.year(),
            requested_month.month()
        ),
        month_start: requested_month,
        month_end,
        status: report_status,
        currencies: currencies
            .into_iter()
            .map(|(currency, value)| {
                Ok(ExpenseCurrencySummary {
                    currency,
                    net_personal_spend_minor: safe_summary_amount("net personal spend", value.net)?,
                    gross_purchase_minor: safe_summary_amount("gross purchases", value.purchases)?,
                    refunds_minor: safe_summary_amount("refunds", value.refunds)?,
                    settlement_received_minor: safe_summary_amount(
                        "settlement received",
                        value.settlement_received,
                    )?,
                    settlement_sent_minor: safe_summary_amount(
                        "settlement sent",
                        value.settlement_sent,
                    )?,
                    fees_minor: safe_summary_amount("fees", value.fees)?,
                    unconfirmed_outflow_minor: safe_summary_amount(
                        "unconfirmed outflow",
                        value.unconfirmed_outflow,
                    )?,
                    recurring_expected_minor: safe_summary_amount(
                        "recurring expected",
                        value.recurring_expected,
                    )?,
                    recurring_paid_minor: safe_summary_amount(
                        "recurring paid",
                        value.recurring_paid,
                    )?,
                    recurring_remaining_minor: safe_summary_amount(
                        "recurring remaining",
                        value.recurring_remaining,
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        categories: categories
            .into_iter()
            .map(|((category, currency), amount_minor)| {
                Ok(ExpenseCategoryTotal {
                    category,
                    currency,
                    amount_minor: safe_summary_amount("category total", amount_minor)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        daily: daily
            .into_iter()
            .map(|((date, currency), amount_minor)| {
                Ok(ExpenseDailyTotal {
                    date,
                    currency,
                    amount_minor: safe_summary_amount("daily total", amount_minor)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        completeness: ExpenseDataCompleteness {
            active_source_count,
            covered_source_count,
            pending_review_count,
            rejected_row_count,
        },
        recurring_candidates,
    })
}

fn persist_expense_month_reports(
    transaction: &Transaction<'_>,
    summary: &ExpenseMonthSummary,
) -> Result<()> {
    let generated_at = now_utc();
    let retained_currencies = summary
        .currencies
        .iter()
        .map(|currency| currency.currency.as_str())
        .collect::<HashSet<_>>();
    for currency in &summary.currencies {
        let snapshot = ExpenseMonthCurrencySnapshot {
            month_start: summary.month_start,
            month_end: summary.month_end,
            status: summary.status,
            currency,
            categories: summary
                .categories
                .iter()
                .filter(|category| category.currency == currency.currency)
                .collect(),
            daily: summary
                .daily
                .iter()
                .filter(|daily| daily.currency == currency.currency)
                .collect(),
            completeness: &summary.completeness,
            recurring_candidates: summary.recurring_candidates,
        };
        let aggregate_json = serde_json::to_string(&snapshot)?;
        let aggregate_sha256 = format!("{:x}", Sha256::digest(aggregate_json.as_bytes()));
        transaction.execute(
            "INSERT INTO expense_month_reports(
                month_start, currency, report_status, aggregate_sha256,
                aggregate_json, generated_at, version
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
             ON CONFLICT(month_start, currency) DO UPDATE SET
                report_status = excluded.report_status,
                aggregate_sha256 = excluded.aggregate_sha256,
                aggregate_json = excluded.aggregate_json,
                generated_at = excluded.generated_at,
                version = expense_month_reports.version + 1
             WHERE expense_month_reports.report_status != excluded.report_status
                OR expense_month_reports.aggregate_sha256 != excluded.aggregate_sha256",
            params![
                summary.month_start,
                currency.currency,
                summary.status.as_str(),
                aggregate_sha256,
                aggregate_json,
                generated_at,
            ],
        )?;
    }

    let stale_currencies = {
        let mut statement = transaction.prepare(
            "SELECT currency FROM expense_month_reports
             WHERE month_start = ?1 ORDER BY currency",
        )?;
        statement
            .query_map([summary.month_start], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for currency in stale_currencies {
        if !retained_currencies.contains(currency.as_str()) {
            transaction.execute(
                "DELETE FROM expense_month_reports
                 WHERE month_start = ?1 AND currency = ?2",
                params![summary.month_start, currency],
            )?;
        }
    }
    Ok(())
}

fn refresh_expense_months_in_transaction(
    transaction: &Transaction<'_>,
    months: BTreeSet<NaiveDate>,
) -> Result<()> {
    for requested_month in months {
        let summary = expense_month_summary_in_connection(transaction, requested_month)?;
        persist_expense_month_reports(transaction, &summary)?;
    }
    Ok(())
}

fn known_expense_months(transaction: &Transaction<'_>) -> Result<BTreeSet<NaiveDate>> {
    let mut statement = transaction.prepare(
        "SELECT month_start FROM expense_month_reports
         UNION
         SELECT substr(posted_date, 1, 7) || '-01' FROM expense_events
         ORDER BY 1",
    )?;
    statement
        .query_map([], |row| row.get::<_, NaiveDate>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(Into::into)
}

fn refresh_after_expense_mutation(
    transaction: &Transaction<'_>,
    command: &ExpenseMutationCommand,
) -> Result<()> {
    let mut months = BTreeSet::new();
    match command {
        ExpenseMutationCommand::Import { input, .. } => {
            for row in &input.rows {
                months.insert(month_start(row.posted_date)?);
            }
            let coverage_start = month_start(input.coverage_start)?;
            let coverage_end = month_start(input.coverage_end)?;
            months.extend(
                known_expense_months(transaction)?
                    .into_iter()
                    .filter(|month| *month >= coverage_start && *month <= coverage_end),
            );
        }
        ExpenseMutationCommand::UpdateSource { .. }
        | ExpenseMutationCommand::ResolveReview { .. }
        | ExpenseMutationCommand::OverrideTransaction { .. } => {
            months = known_expense_months(transaction)?;
        }
        ExpenseMutationCommand::CreateRecurring(input) => {
            let effective = month_start(input.start_date)?;
            months.extend(
                known_expense_months(transaction)?
                    .into_iter()
                    .filter(|month| *month >= effective),
            );
        }
        ExpenseMutationCommand::UpdateRecurring { input, .. } => {
            months.extend(
                known_expense_months(transaction)?
                    .into_iter()
                    .filter(|month| *month >= input.effective_from_month),
            );
        }
        ExpenseMutationCommand::DeleteRecurring { .. } => {
            let effective = current_month_start()?;
            months.extend(
                known_expense_months(transaction)?
                    .into_iter()
                    .filter(|month| *month >= effective),
            );
        }
        ExpenseMutationCommand::ConfirmRecurringPaid {
            occurrence_key,
            input,
        } => {
            let (_, occurrence_month) = parse_occurrence_key(occurrence_key)?;
            months.insert(occurrence_month);
            if let Some(paid_date) = input.paid_date {
                months.insert(month_start(paid_date)?);
            }
        }
        ExpenseMutationCommand::MatchRecurring { occurrence_key, .. } => {
            let (_, occurrence_month) = parse_occurrence_key(occurrence_key)?;
            months.insert(occurrence_month);
            months.extend(known_expense_months(transaction)?);
        }
        ExpenseMutationCommand::InitializeCryptoProbe(_)
        | ExpenseMutationCommand::PreviewImport(_)
        | ExpenseMutationCommand::SaveReport(_)
        | ExpenseMutationCommand::RecordReportFeedback { .. } => {}
    }
    refresh_expense_months_in_transaction(transaction, months)
}

fn save_expense_report_in_transaction(
    transaction: &Transaction<'_>,
    input: &SaveExpenseReportInput,
) -> Result<ExpenseReportResult> {
    if let Some(existing) = transaction
        .query_row(
            "SELECT id, month_start, aggregate_sha256, prompt_version, model,
                    result_json, input_tokens, cached_input_tokens,
                    output_tokens, total_tokens, cost_microusd,
                    latency_ms, created_at
             FROM expense_ai_reports
             WHERE aggregate_sha256 = ?1 AND prompt_version = ?2",
            params![input.aggregate_sha256, input.prompt_version],
            map_expense_report_result,
        )
        .optional()?
    {
        return Ok(existing);
    }
    let id = new_id();
    let created_at = now_utc();
    let content = serde_json::json!({
        "title": input.title,
        "summary": input.summary,
        "facts": input.facts,
        "observations": input.observations,
        "alerts": input.alerts,
        "nextMonthChecks": input.next_month_checks,
    });
    transaction.execute(
        "INSERT INTO expense_ai_reports(
            id, month_start, aggregate_sha256, prompt_version, model,
            result_json, input_tokens, cached_input_tokens, output_tokens,
            total_tokens, cost_microusd, latency_ms, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id,
            input.month_start,
            input.aggregate_sha256,
            input.prompt_version,
            input.model,
            serde_json::to_string(&content)?,
            as_i64(input.input_tokens, "expense report input tokens")?,
            as_i64(
                input.cached_input_tokens,
                "expense report cached input tokens",
            )?,
            as_i64(input.output_tokens, "expense report output tokens")?,
            as_i64(input.total_tokens, "expense report total tokens")?,
            as_i64(input.cost_microusd, "expense report cost")?,
            as_i64(input.latency_ms, "expense report latency")?,
            created_at,
        ],
    )?;
    transaction
        .query_row(
            "SELECT id, month_start, aggregate_sha256, prompt_version, model,
                    result_json, input_tokens, cached_input_tokens,
                    output_tokens, total_tokens, cost_microusd,
                    latency_ms, created_at
             FROM expense_ai_reports WHERE id = ?1",
            [&id],
            map_expense_report_result,
        )
        .map_err(Into::into)
}

fn map_expense_report_result(row: &Row<'_>) -> rusqlite::Result<ExpenseReportResult> {
    let json = row.get::<_, String>(5)?;
    let content: Value = serde_json::from_str(&json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let field = |name: &str| {
        content
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(
                    5,
                    rusqlite::types::Type::Text,
                    Box::new(Error::Invariant(format!(
                        "expense report result is missing {name}"
                    ))),
                )
            })
    };
    let observations = serde_json::from_value(
        content
            .get("observations")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let facts = serde_json::from_value(
        content
            .get("facts")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let alerts = serde_json::from_value(
        content
            .get("alerts")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let next_month_checks = serde_json::from_value(
        content
            .get("nextMonthChecks")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    )
    .map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ExpenseReportResult {
        id: row.get(0)?,
        month_start: row.get(1)?,
        aggregate_sha256: row.get(2)?,
        prompt_version: row.get(3)?,
        model: row.get(4)?,
        title: field("title")?,
        summary: field("summary")?,
        facts,
        observations,
        alerts,
        next_month_checks,
        input_tokens: read_nonnegative_u64(row, 6)?,
        cached_input_tokens: read_nonnegative_u64(row, 7)?,
        output_tokens: read_nonnegative_u64(row, 8)?,
        total_tokens: read_nonnegative_u64(row, 9)?,
        cost_microusd: read_nonnegative_u64(row, 10)?,
        latency_ms: read_nonnegative_u64(row, 11)?,
        created_at: row.get(12)?,
    })
}

fn record_expense_feedback_in_transaction(
    transaction: &Transaction<'_>,
    report_id: &str,
    helpful: bool,
) -> Result<()> {
    let exists: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM expense_ai_reports WHERE id = ?1)",
        [report_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(not_found("expense AI report", report_id));
    }
    transaction.execute(
        "INSERT INTO expense_ai_feedback(id, report_id, helpful, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(report_id) DO UPDATE SET helpful = excluded.helpful,
                                              created_at = excluded.created_at",
        params![new_id(), report_id, helpful, now_utc()],
    )?;
    Ok(())
}

fn validate_expense_mutation_command(command: &ExpenseMutationCommand) -> Result<()> {
    match command {
        ExpenseMutationCommand::InitializeCryptoProbe(probe) => validate_crypto_probe(probe),
        ExpenseMutationCommand::PreviewImport(input) => validate_expense_preview_input(input),
        ExpenseMutationCommand::Import {
            preview_session_id,
            input,
        } => {
            validate_label("expense import preview session ID", preview_session_id, 128)?;
            validate_normalized_import(input)
        }
        ExpenseMutationCommand::UpdateSource { source_id, input } => {
            validate_expense_source_status_update(source_id, input)
        }
        ExpenseMutationCommand::ResolveReview { review_id, input } => {
            validate_label("expense review ID", review_id, 128)?;
            if input.expected_version == 0 {
                return Err(invalid("expense review expected version must be positive"));
            }
            validate_review_resolution(input)
        }
        ExpenseMutationCommand::OverrideTransaction { event_id, input } => {
            validate_expense_transaction_override(event_id, input)
        }
        ExpenseMutationCommand::CreateRecurring(input) => validate_recurring_create(input),
        ExpenseMutationCommand::UpdateRecurring {
            recurring_expense_id,
            input,
        } => {
            validate_label("recurring expense ID", recurring_expense_id, 128)?;
            validate_recurring_update(recurring_expense_id, input)
        }
        ExpenseMutationCommand::DeleteRecurring {
            recurring_expense_id,
            expected_version,
        } => {
            validate_label("recurring expense ID", recurring_expense_id, 128)?;
            if *expected_version == 0 {
                return Err(invalid(
                    "recurring expense expected version must be positive",
                ));
            }
            Ok(())
        }
        ExpenseMutationCommand::ConfirmRecurringPaid {
            occurrence_key,
            input,
        } => {
            parse_occurrence_key(occurrence_key)?;
            if input.expected_version == 0 {
                return Err(invalid(
                    "recurring occurrence expected item version must be positive",
                ));
            }
            if let Some(amount_minor) = input.amount_minor {
                validate_amount_minor("recurring paid amountMinor", amount_minor)?;
            }
            Ok(())
        }
        ExpenseMutationCommand::MatchRecurring {
            occurrence_key,
            input,
        } => {
            parse_occurrence_key(occurrence_key)?;
            if input.expected_version == 0 {
                return Err(invalid(
                    "recurring occurrence expected item version must be positive",
                ));
            }
            validate_label("expense event ID", &input.event_id, 128)
        }
        ExpenseMutationCommand::SaveReport(input) => validate_save_expense_report(input),
        ExpenseMutationCommand::RecordReportFeedback { report_id, .. } => {
            validate_label("expense report ID", report_id, 128)
        }
    }
}

fn validate_crypto_probe(probe: &ExpenseCryptoProbe) -> Result<()> {
    if probe.key_version == 0 {
        return Err(invalid("expense crypto probe key version must be positive"));
    }
    validate_bounded_text("expense crypto probe nonce", &probe.nonce, 16, 128)?;
    validate_bounded_text(
        "expense crypto probe ciphertext",
        &probe.ciphertext,
        16,
        4096,
    )?;
    if probe.aad != EXPENSE_CRYPTO_PROBE_AAD {
        return Err(invalid(format!(
            "expense crypto probe AAD must be {EXPENSE_CRYPTO_PROBE_AAD}"
        )));
    }
    Ok(())
}

fn validate_normalized_import(input: &NormalizedExpenseImport) -> Result<()> {
    validate_sha256("expense source fingerprint", &input.source_fingerprint)?;
    validate_sha256("expense file SHA-256", &input.file_sha256)?;
    validate_sha256("normalized expense SHA-256", &input.normalized_sha256)?;
    if input.coverage_start > input.coverage_end {
        return Err(invalid(
            "expense import coverage end cannot be before its start",
        ));
    }
    let total_rows = input
        .rows
        .len()
        .checked_add(input.rejected_count as usize)
        .ok_or_else(|| invalid("expense import row count overflow"))?;
    if total_rows > MAX_IMPORT_ROWS {
        return Err(invalid(format!(
            "expense import cannot exceed {MAX_IMPORT_ROWS} rows"
        )));
    }
    let mut stable_keys = HashSet::with_capacity(input.rows.len());
    for row in &input.rows {
        validate_label("expense stable row key", &row.stable_key, 256)?;
        if !stable_keys.insert(row.stable_key.as_str()) {
            return Err(invalid(format!(
                "expense import repeats stable row key {}",
                row.stable_key
            )));
        }
        validate_sha256("expense row SHA-256", &row.row_sha256)?;
        if row.source_row_number == 0 || row.source_row_number > MAX_IMPORT_ROWS as u32 {
            return Err(invalid(format!(
                "expense source row number must be between 1 and {MAX_IMPORT_ROWS}"
            )));
        }
        DateTime::parse_from_rfc3339(&row.occurred_at)
            .map_err(|_| invalid("expense occurredAt must be RFC 3339 with an offset"))?;
        if row.posted_date < input.coverage_start || row.posted_date > input.coverage_end {
            return Err(invalid(
                "expense row posted date must be inside the declared coverage window",
            ));
        }
        validate_amount_minor("expense amountMinor", row.amount_minor)?;
        validate_currency(&row.currency)?;
        if row.kind == ExpenseEventKind::ManualRecurring {
            return Err(invalid(
                "manual recurring events cannot be supplied by a normalized import",
            ));
        }
        for (name, value, field) in [
            ("expense merchant", row.merchant.as_ref(), "merchant"),
            (
                "expense counterparty",
                row.counterparty.as_ref(),
                "counterparty",
            ),
            ("expense memo", row.memo.as_ref(), "memo"),
        ] {
            if let Some(value) = value {
                validate_encrypted_text(name, value)?;
                let context = format!("{}:{}", input.source_fingerprint, row.stable_key);
                let expected = expense_text_aad(&context, field);
                if value.aad != expected {
                    return Err(invalid(format!(
                        "{name} AAD must bind source fingerprint, stable row key, and field"
                    )));
                }
            }
        }
        if let Some(value) = row.payment_method_fingerprint.as_deref() {
            validate_sha256("expense payment method fingerprint", value)?;
        }
        if let Some(value) = row.external_reference_fingerprint.as_deref() {
            validate_sha256("expense external reference fingerprint", value)?;
        }
    }
    Ok(())
}

fn validate_expense_preview_input(input: &ExpenseImportPreviewInput) -> Result<()> {
    let expected_source_kind = match input.adapter {
        ExpenseImportAdapter::KbCardUsageV1 => ExpenseSourceKind::Card,
        ExpenseImportAdapter::KbAccountHistoryV1 => ExpenseSourceKind::Account,
        ExpenseImportAdapter::KakaopayMoneyV1 => ExpenseSourceKind::Wallet,
    };
    if input.source_kind != expected_source_kind {
        return Err(invalid(format!(
            "adapter {} requires source kind {}",
            input.adapter, expected_source_kind
        )));
    }
    validate_sha256("expense source fingerprint", &input.source_fingerprint)?;
    validate_sha256("expense file SHA-256", &input.file_sha256)?;
    validate_sha256("normalized expense SHA-256", &input.normalized_sha256)?;
    if input.coverage_start > input.coverage_end {
        return Err(invalid(
            "expense import preview coverageStart must not follow coverageEnd",
        ));
    }
    let total_rows = input
        .rows
        .len()
        .checked_add(input.rejected_count as usize)
        .ok_or_else(|| invalid("expense import preview row count overflow"))?;
    if total_rows > MAX_IMPORT_ROWS {
        return Err(invalid(format!(
            "expense import preview cannot exceed {MAX_IMPORT_ROWS} rows"
        )));
    }
    let mut stable_keys = HashSet::with_capacity(input.rows.len());
    for row in &input.rows {
        validate_label("expense stable row key", &row.stable_key, 256)?;
        if !stable_keys.insert(row.stable_key.as_str()) {
            return Err(invalid(format!(
                "expense import preview repeats stable row key {}",
                row.stable_key
            )));
        }
        validate_sha256("expense row SHA-256", &row.row_sha256)?;
        if row.source_row_number == 0 || row.source_row_number > MAX_IMPORT_ROWS as u32 {
            return Err(invalid(
                "expense preview source row number must be between 1 and 5000",
            ));
        }
        DateTime::parse_from_rfc3339(&row.occurred_at)
            .map_err(|_| invalid("expense occurredAt must be RFC 3339 with an offset"))?;
        if row.posted_date < input.coverage_start || row.posted_date > input.coverage_end {
            return Err(invalid(
                "expense preview posted date must be inside the declared coverage window",
            ));
        }
        validate_amount_minor("expense amountMinor", row.amount_minor)?;
        validate_currency(&row.currency)?;
        if row.kind == ExpenseEventKind::ManualRecurring {
            return Err(invalid(
                "manual recurring events cannot be supplied by an import preview",
            ));
        }
        for (name, fingerprint) in [
            (
                "expense merchant blind index",
                row.merchant_blind_index.as_deref(),
            ),
            (
                "expense payment method fingerprint",
                row.payment_method_fingerprint.as_deref(),
            ),
            (
                "expense external reference fingerprint",
                row.external_reference_fingerprint.as_deref(),
            ),
        ] {
            if let Some(fingerprint) = fingerprint {
                validate_sha256(name, fingerprint)?;
            }
        }
    }
    Ok(())
}

fn validate_review_resolution(input: &ResolveExpenseReviewInput) -> Result<()> {
    if matches!(
        input.kind,
        ExpenseEventKind::UnknownP2p
            | ExpenseEventKind::ExternalTransfer
            | ExpenseEventKind::ManualRecurring
    ) {
        return Err(invalid(
            "a resolved expense decision cannot remain an unknown transfer or create a manual recurring event",
        ));
    }
    if input.category == ExpenseCategory::Unconfirmed
        && !matches!(
            input.kind,
            ExpenseEventKind::UnknownP2p | ExpenseEventKind::ExternalTransfer
        )
    {
        return Err(invalid(
            "the unconfirmed category is reserved for unresolved transfers",
        ));
    }
    if let Some(value) = input.duplicate_of_event_id.as_deref() {
        validate_label("duplicate expense event ID", value, 128)?;
    }
    if let Some(value) = input.related_event_id.as_deref() {
        validate_label("related expense event ID", value, 128)?;
    }
    Ok(())
}

fn validate_expense_transaction_override(
    event_id: &str,
    input: &OverrideExpenseTransactionInput,
) -> Result<()> {
    validate_label("expense event ID", event_id, 128)?;
    if input.expected_version == 0 {
        return Err(invalid("expense event expected version must be positive"));
    }
    if input.clear_personal_amount && input.personal_amount_minor.is_some() {
        return Err(invalid(
            "personalAmountMinor and clearPersonalAmount cannot be used together",
        ));
    }
    if input.clear_related_event && input.related_event_id.is_some() {
        return Err(invalid(
            "relatedEventId and clearRelatedEvent cannot be used together",
        ));
    }
    validate_review_resolution(&ResolveExpenseReviewInput {
        expected_version: 1,
        kind: input.kind,
        category: input.category,
        duplicate_of_event_id: input.duplicate_of_event_id.clone(),
        related_event_id: input.related_event_id.clone(),
        personal_amount_minor: input.personal_amount_minor,
        create_rule: input.create_rule,
    })
}

fn validate_expense_source_status_update(
    source_id: &str,
    input: &UpdateExpenseSourceStatusInput,
) -> Result<()> {
    validate_label("expense source ID", source_id, 128)?;
    if input.expected_version == 0 {
        return Err(invalid("expense source expected version must be positive"));
    }
    Ok(())
}

fn validate_recurring_create(input: &CreateRecurringExpenseInput) -> Result<()> {
    validate_label("recurring expense ID", &input.id, 128)?;
    validate_recurring_fields(
        &input.id,
        &input.name,
        input.category,
        input.vendor.as_ref(),
        input.amount_minor,
        &input.currency,
        input.payment_method_fingerprint.as_deref(),
        input.start_date,
        input.end_date,
        input.memo.as_ref(),
        input.reminder_days,
        input.interval_months,
        input.due_rule,
        input.due_day,
    )
}

fn validate_recurring_update(
    recurring_expense_id: &str,
    input: &UpdateRecurringExpenseInput,
) -> Result<()> {
    validate_label("recurring expense ID", recurring_expense_id, 128)?;
    if input.expected_version == 0 {
        return Err(invalid(
            "recurring expense expected version must be positive",
        ));
    }
    validate_month_start(input.effective_from_month)?;
    if input.effective_from_month < current_month_start()? {
        return Err(invalid(
            "recurring expense changes can only take effect in the current or a future month",
        ));
    }
    validate_recurring_fields(
        recurring_expense_id,
        &input.name,
        input.category,
        input.vendor.as_ref(),
        input.amount_minor,
        &input.currency,
        input.payment_method_fingerprint.as_deref(),
        input.start_date,
        input.end_date,
        input.memo.as_ref(),
        input.reminder_days,
        input.interval_months,
        input.due_rule,
        input.due_day,
    )
}

#[allow(clippy::too_many_arguments)]
fn validate_recurring_fields(
    recurring_expense_id: &str,
    name: &EncryptedExpenseText,
    category: ExpenseCategory,
    vendor: Option<&EncryptedExpenseText>,
    amount_minor: i64,
    currency: &str,
    payment_method_fingerprint: Option<&str>,
    start_date: NaiveDate,
    end_date: Option<NaiveDate>,
    memo: Option<&EncryptedExpenseText>,
    reminder_days: u8,
    interval_months: u8,
    due_rule: RecurringDueRule,
    due_day: Option<u8>,
) -> Result<()> {
    let context = format!("recurring:{recurring_expense_id}");
    validate_encrypted_text("recurring expense name", name)?;
    if name.aad != expense_text_aad(&context, "name") {
        return Err(invalid(
            "recurring expense name AAD must bind the recurring expense ID",
        ));
    }
    if let Some(value) = vendor {
        validate_encrypted_text("recurring expense vendor", value)?;
        if value.aad != expense_text_aad(&context, "vendor") {
            return Err(invalid(
                "recurring expense vendor AAD must bind the recurring expense ID",
            ));
        }
    }
    if let Some(value) = memo {
        validate_encrypted_text("recurring expense memo", value)?;
        if value.aad != expense_text_aad(&context, "memo") {
            return Err(invalid(
                "recurring expense memo AAD must bind the recurring expense ID",
            ));
        }
    }
    if category == ExpenseCategory::Unconfirmed {
        return Err(invalid(
            "recurring expenses cannot use the unconfirmed category",
        ));
    }
    validate_amount_minor("recurring expense amountMinor", amount_minor)?;
    validate_currency(currency)?;
    if let Some(value) = payment_method_fingerprint {
        validate_sha256("recurring payment method fingerprint", value)?;
    }
    if end_date.is_some_and(|date| date < start_date) {
        return Err(invalid(
            "recurring expense end date cannot be before its start date",
        ));
    }
    if reminder_days > 90 {
        return Err(invalid("recurring expense reminder days cannot exceed 90"));
    }
    if !matches!(interval_months, 1 | 2 | 3 | 6 | 12) {
        return Err(invalid(
            "recurring interval months must be one of 1, 2, 3, 6, or 12",
        ));
    }
    match due_rule {
        RecurringDueRule::SpecificDay if !matches!(due_day, Some(1..=31)) => {
            return Err(invalid(
                "a specific recurring due rule requires a day from 1 through 31",
            ));
        }
        RecurringDueRule::FirstDay | RecurringDueRule::LastDay if due_day.is_some() => {
            return Err(invalid(
                "first-day and last-day recurring rules cannot include dueDay",
            ));
        }
        _ => {}
    }
    Ok(())
}

fn validate_save_expense_report(input: &SaveExpenseReportInput) -> Result<()> {
    validate_month_start(input.month_start)?;
    validate_sha256("expense aggregate SHA-256", &input.aggregate_sha256)?;
    validate_label("expense report prompt version", &input.prompt_version, 128)?;
    validate_label("expense report model", &input.model, 128)?;
    validate_bounded_text("expense report title", &input.title, 1, 500)?;
    validate_bounded_text("expense report summary", &input.summary, 1, 4_000)?;
    if input.facts.len() > 500 {
        return Err(invalid("expense report cannot contain more than 500 facts"));
    }
    let mut fact_ids = HashSet::with_capacity(input.facts.len());
    for fact in &input.facts {
        validate_label("expense report fact ID", &fact.fact_id, 256)?;
        validate_label("expense report fact metric", &fact.metric, 256)?;
        validate_currency(&fact.currency)?;
        if fact.amount_minor.unsigned_abs() > MAX_SAFE_AMOUNT_MINOR as u64 {
            return Err(invalid(format!(
                "expense report fact amount must be within +/-{MAX_SAFE_AMOUNT_MINOR}"
            )));
        }
        if !fact_ids.insert(fact.fact_id.as_str()) {
            return Err(invalid(format!(
                "expense report repeats fact ID {}",
                fact.fact_id
            )));
        }
    }
    if input.observations.len() > 20
        || input.alerts.len() > 20
        || input.next_month_checks.len() > 20
    {
        return Err(invalid(
            "expense report sections cannot contain more than 20 entries",
        ));
    }
    for observation in input.observations.iter().chain(&input.alerts) {
        validate_bounded_text("expense report observation", &observation.text, 1, 2_000)?;
        if observation.fact_ids.is_empty() || observation.fact_ids.len() > 20 {
            return Err(invalid(
                "expense report observations must cite between 1 and 20 fact IDs",
            ));
        }
        for fact_id in &observation.fact_ids {
            validate_label("expense report fact ID", fact_id, 256)?;
            if !fact_ids.contains(fact_id.as_str()) {
                return Err(invalid(format!(
                    "expense report observation cites unknown fact ID {fact_id}"
                )));
            }
        }
    }
    for item in &input.next_month_checks {
        validate_bounded_text("expense report next-month check", item, 1, 2_000)?;
    }
    if input.cost_microusd > 50_000 {
        return Err(invalid(
            "expense AI report cost cannot exceed the $0.05 per-request reservation",
        ));
    }
    if input.cached_input_tokens > input.input_tokens {
        return Err(invalid(
            "expense report cached input tokens cannot exceed input tokens",
        ));
    }
    let expected_total_tokens = input
        .input_tokens
        .checked_add(input.output_tokens)
        .ok_or_else(|| invalid("expense report token totals overflowed"))?;
    if input.total_tokens != expected_total_tokens {
        return Err(invalid(
            "expense report total tokens must equal input plus output tokens",
        ));
    }
    for (name, value) in [
        ("expense report input tokens", input.input_tokens),
        (
            "expense report cached input tokens",
            input.cached_input_tokens,
        ),
        ("expense report output tokens", input.output_tokens),
        ("expense report total tokens", input.total_tokens),
        ("expense report cost", input.cost_microusd),
        ("expense report latency", input.latency_ms),
    ] {
        as_i64(value, name)?;
    }
    Ok(())
}

fn validate_encrypted_text(name: &str, value: &EncryptedExpenseText) -> Result<()> {
    if value.key_version == 0 {
        return Err(invalid(format!("{name} key version must be positive")));
    }
    validate_bounded_text(&format!("{name} nonce"), &value.nonce, 16, 128)?;
    validate_bounded_text(
        &format!("{name} ciphertext"),
        &value.ciphertext,
        16,
        100_000,
    )?;
    validate_bounded_text(&format!("{name} AAD"), &value.aad, 1, 512)?;
    if !value.aad.starts_with(EXPENSE_AAD_PREFIX) {
        return Err(invalid(format!(
            "{name} AAD must use the {EXPENSE_AAD_PREFIX} context prefix"
        )));
    }
    validate_sha256(&format!("{name} blind index"), &value.blind_index)
}

fn validate_sha256(name: &str, value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(invalid(format!(
            "{name} must be exactly 64 lowercase hexadecimal characters"
        )));
    }
    Ok(())
}

fn validate_amount_minor(name: &str, value: i64) -> Result<()> {
    if value <= 0 || value > MAX_SAFE_AMOUNT_MINOR {
        return Err(invalid(format!(
            "{name} must be between 1 and {MAX_SAFE_AMOUNT_MINOR}"
        )));
    }
    Ok(())
}

fn validate_currency(value: &str) -> Result<()> {
    if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(invalid(
            "expense currency must be a three-letter uppercase ISO code",
        ));
    }
    Ok(())
}

fn validate_label(name: &str, value: &str, maximum: usize) -> Result<()> {
    validate_bounded_text(name, value, 1, maximum)?;
    if value.chars().any(char::is_control) {
        return Err(invalid(format!("{name} cannot contain control characters")));
    }
    Ok(())
}

fn validate_bounded_text(name: &str, value: &str, minimum: usize, maximum: usize) -> Result<()> {
    let length = value.chars().count();
    if !(minimum..=maximum).contains(&length) {
        return Err(invalid(format!(
            "{name} must contain between {minimum} and {maximum} characters"
        )));
    }
    Ok(())
}

fn validate_month_start(value: NaiveDate) -> Result<()> {
    if value.day() != 1 {
        return Err(invalid("expense month must be the first day of a month"));
    }
    Ok(())
}

fn month_start(value: NaiveDate) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(value.year(), value.month(), 1)
        .ok_or_else(|| invalid("expense date is outside the supported range"))
}

fn current_month_start() -> Result<NaiveDate> {
    month_start(today_seoul())
}

fn next_month(value: NaiveDate) -> Result<NaiveDate> {
    validate_month_start(value)?;
    if value.month() == 12 {
        NaiveDate::from_ymd_opt(value.year().saturating_add(1), 1, 1)
    } else {
        NaiveDate::from_ymd_opt(value.year(), value.month() + 1, 1)
    }
    .ok_or_else(|| invalid("expense month is outside the supported range"))
}

fn previous_month(value: NaiveDate) -> Result<NaiveDate> {
    validate_month_start(value)?;
    value
        .pred_opt()
        .ok_or_else(|| invalid("expense month is outside the supported range"))
        .and_then(month_start)
}

fn last_day_of_month(year: i32, month: u32) -> Result<NaiveDate> {
    let first = NaiveDate::from_ymd_opt(year, month, 1)
        .ok_or_else(|| invalid("expense month is invalid"))?;
    next_month(first)?
        .pred_opt()
        .ok_or_else(|| invalid("expense month is outside the supported range"))
}

fn month_difference(start: NaiveDate, end: NaiveDate) -> Result<i32> {
    let start_month =
        i32::try_from(start.month()).map_err(|_| invalid("recurring start month is invalid"))?;
    let end_month =
        i32::try_from(end.month()).map_err(|_| invalid("recurring end month is invalid"))?;
    Ok(end
        .year()
        .saturating_sub(start.year())
        .saturating_mul(12)
        .saturating_add(end_month.saturating_sub(start_month)))
}

fn validate_page_limit(value: u32) -> Result<u32> {
    if value == 0 || value > MAX_PAGE_SIZE {
        return Err(invalid(format!(
            "expense page limit must be between 1 and {MAX_PAGE_SIZE}"
        )));
    }
    Ok(value)
}

fn encode_cursor(timestamp: &str, id: &str) -> String {
    format!("{timestamp}|{id}")
}

fn expense_transaction_cursor(value: &ExpenseTransaction) -> String {
    encode_cursor(&value.occurred_at, &value.id)
}

fn parse_cursor(value: Option<&str>) -> Result<(Option<String>, Option<String>)> {
    let Some(value) = value else {
        return Ok((None, None));
    };
    if value.len() > 512 {
        return Err(invalid("expense cursor is too long"));
    }
    let (timestamp, id) = value
        .split_once('|')
        .ok_or_else(|| invalid("expense cursor is invalid"))?;
    validate_label("expense cursor timestamp", timestamp, 128)?;
    validate_label("expense cursor ID", id, 128)?;
    Ok((Some(timestamp.to_owned()), Some(id.to_owned())))
}

fn as_i64(value: u64, name: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid(format!("{name} is too large")))
}

fn read_nonnegative_u64(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
