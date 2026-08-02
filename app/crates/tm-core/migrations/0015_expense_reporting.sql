CREATE TABLE expense_crypto_metadata (
    singleton_key TEXT PRIMARY KEY NOT NULL CHECK(singleton_key = 'expense-data-key-probe'),
    key_version INTEGER NOT NULL CHECK(key_version > 0),
    nonce TEXT NOT NULL CHECK(length(nonce) BETWEEN 16 AND 128),
    ciphertext TEXT NOT NULL CHECK(length(ciphertext) BETWEEN 16 AND 4096),
    aad TEXT NOT NULL CHECK(length(aad) BETWEEN 1 AND 512),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE expense_sources (
    id TEXT PRIMARY KEY NOT NULL,
    adapter TEXT NOT NULL CHECK(adapter IN (
        'kb_card_usage_v1', 'kb_account_history_v1', 'kakaopay_money_v1'
    )),
    source_kind TEXT NOT NULL CHECK(source_kind IN ('card', 'account', 'wallet')),
    source_fingerprint TEXT NOT NULL UNIQUE CHECK(length(source_fingerprint) = 64),
    required_for_complete_report INTEGER NOT NULL DEFAULT 1
        CHECK(required_for_complete_report IN (0, 1)),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK(is_active IN (0, 1)),
    coverage_start TEXT,
    coverage_end TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    CHECK(coverage_start IS NULL OR coverage_end IS NULL OR coverage_start <= coverage_end)
);

CREATE TABLE expense_import_batches (
    id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES expense_sources(id) ON DELETE RESTRICT,
    file_sha256 TEXT NOT NULL CHECK(length(file_sha256) = 64),
    normalized_sha256 TEXT NOT NULL CHECK(length(normalized_sha256) = 64),
    coverage_start TEXT NOT NULL,
    coverage_end TEXT NOT NULL,
    row_count INTEGER NOT NULL CHECK(row_count BETWEEN 0 AND 5000),
    accepted_count INTEGER NOT NULL CHECK(accepted_count BETWEEN 0 AND row_count),
    duplicate_count INTEGER NOT NULL CHECK(duplicate_count BETWEEN 0 AND row_count),
    rejected_count INTEGER NOT NULL CHECK(rejected_count BETWEEN 0 AND row_count),
    created_at TEXT NOT NULL,
    UNIQUE(source_id, file_sha256),
    CHECK(coverage_start <= coverage_end),
    CHECK(accepted_count + duplicate_count + rejected_count = row_count)
);

CREATE TABLE expense_import_preview_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    adapter TEXT NOT NULL CHECK(adapter IN (
        'kb_card_usage_v1', 'kb_account_history_v1', 'kakaopay_money_v1'
    )),
    source_kind TEXT NOT NULL CHECK(source_kind IN ('card', 'account', 'wallet')),
    source_fingerprint TEXT NOT NULL CHECK(length(source_fingerprint) = 64),
    file_sha256 TEXT NOT NULL CHECK(length(file_sha256) = 64),
    normalized_sha256 TEXT NOT NULL CHECK(length(normalized_sha256) = 64),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    coverage_start TEXT NOT NULL,
    coverage_end TEXT NOT NULL,
    row_count INTEGER NOT NULL CHECK(row_count BETWEEN 0 AND 5000),
    rejected_count INTEGER NOT NULL CHECK(rejected_count BETWEEN 0 AND row_count),
    expires_at TEXT NOT NULL,
    consumed_at TEXT,
    created_at TEXT NOT NULL,
    CHECK(coverage_start <= coverage_end),
    CHECK(expires_at > created_at)
);

CREATE TABLE expense_raw_rows (
    id TEXT PRIMARY KEY NOT NULL,
    batch_id TEXT NOT NULL REFERENCES expense_import_batches(id) ON DELETE RESTRICT,
    source_id TEXT NOT NULL REFERENCES expense_sources(id) ON DELETE RESTRICT,
    stable_key TEXT NOT NULL CHECK(length(stable_key) BETWEEN 1 AND 256),
    row_sha256 TEXT NOT NULL CHECK(length(row_sha256) = 64),
    source_row_number INTEGER NOT NULL CHECK(source_row_number BETWEEN 1 AND 5000),
    occurred_at TEXT NOT NULL,
    posted_date TEXT NOT NULL,
    direction TEXT NOT NULL CHECK(direction IN ('debit', 'credit')),
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    normalized_kind TEXT NOT NULL CHECK(normalized_kind IN (
        'purchase', 'refund', 'card_payment', 'wallet_topup', 'internal_transfer',
        'settlement_received', 'settlement_sent', 'fee', 'external_transfer',
        'unknown_p2p'
    )),
    created_at TEXT NOT NULL,
    UNIQUE(source_id, stable_key)
);

CREATE TABLE expense_postings (
    id TEXT PRIMARY KEY NOT NULL,
    raw_row_id TEXT NOT NULL UNIQUE REFERENCES expense_raw_rows(id) ON DELETE RESTRICT,
    source_id TEXT NOT NULL REFERENCES expense_sources(id) ON DELETE RESTRICT,
    direction TEXT NOT NULL CHECK(direction IN ('debit', 'credit')),
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    occurred_at TEXT NOT NULL,
    posted_date TEXT NOT NULL,
    merchant_key_version INTEGER,
    merchant_nonce TEXT,
    merchant_ciphertext TEXT,
    merchant_aad TEXT,
    merchant_blind_index TEXT,
    counterparty_key_version INTEGER,
    counterparty_nonce TEXT,
    counterparty_ciphertext TEXT,
    counterparty_aad TEXT,
    counterparty_blind_index TEXT,
    memo_key_version INTEGER,
    memo_nonce TEXT,
    memo_ciphertext TEXT,
    memo_aad TEXT,
    memo_blind_index TEXT,
    payment_method_fingerprint TEXT,
    external_reference_fingerprint TEXT,
    created_at TEXT NOT NULL,
    CHECK((merchant_key_version IS NULL) = (merchant_nonce IS NULL)),
    CHECK((merchant_key_version IS NULL) = (merchant_ciphertext IS NULL)),
    CHECK((merchant_key_version IS NULL) = (merchant_aad IS NULL)),
    CHECK((merchant_key_version IS NULL) = (merchant_blind_index IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_nonce IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_ciphertext IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_aad IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_blind_index IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_nonce IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_ciphertext IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_aad IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_blind_index IS NULL))
);

CREATE TABLE expense_events (
    id TEXT PRIMARY KEY NOT NULL,
    event_kind TEXT NOT NULL CHECK(event_kind IN (
        'purchase', 'refund', 'card_payment', 'wallet_topup', 'internal_transfer',
        'settlement_received', 'settlement_sent', 'fee', 'external_transfer',
        'unknown_p2p', 'manual_recurring'
    )),
    category TEXT NOT NULL CHECK(category IN (
        'food', 'delivery', 'cafe', 'groceries', 'housing_utilities',
        'transportation', 'ott_subscriptions', 'shopping', 'health', 'leisure',
        'education', 'travel', 'insurance_finance_tax', 'gifts_dues',
        'refund_income', 'transfer_settlement', 'other', 'unconfirmed'
    )),
    event_status TEXT NOT NULL CHECK(event_status IN ('confirmed', 'unconfirmed', 'excluded')),
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    occurred_at TEXT NOT NULL,
    posted_date TEXT NOT NULL,
    primary_posting_id TEXT UNIQUE REFERENCES expense_postings(id) ON DELETE RESTRICT,
    duplicate_of_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    exclusion_reason TEXT CHECK(exclusion_reason IN (
        'duplicate', 'card_payment', 'wallet_topup', 'internal_transfer',
        'cross_source_mirror', 'manual_replaced'
    )),
    is_provisional INTEGER NOT NULL DEFAULT 0 CHECK(is_provisional IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    CHECK(event_status = 'excluded' OR exclusion_reason IS NULL)
);

CREATE TABLE expense_event_postings (
    event_id TEXT NOT NULL REFERENCES expense_events(id) ON DELETE RESTRICT,
    posting_id TEXT NOT NULL UNIQUE REFERENCES expense_postings(id) ON DELETE RESTRICT,
    posting_role TEXT NOT NULL CHECK(posting_role IN ('primary', 'mirror', 'supporting')),
    created_at TEXT NOT NULL,
    PRIMARY KEY(event_id, posting_id)
);

CREATE TABLE expense_allocations (
    id TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL REFERENCES expense_events(id) ON DELETE RESTRICT,
    related_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    allocation_kind TEXT NOT NULL CHECK(allocation_kind IN (
        'personal', 'shared_personal', 'settlement_received', 'settlement_sent', 'fee'
    )),
    allocation_source TEXT NOT NULL DEFAULT 'user'
        CHECK(allocation_source IN ('user', 'settlement')),
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    counterparty_key_version INTEGER,
    counterparty_nonce TEXT,
    counterparty_ciphertext TEXT,
    counterparty_aad TEXT,
    counterparty_blind_index TEXT,
    created_at TEXT NOT NULL,
    CHECK((counterparty_key_version IS NULL) = (counterparty_nonce IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_ciphertext IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_aad IS NULL)),
    CHECK((counterparty_key_version IS NULL) = (counterparty_blind_index IS NULL))
);

CREATE TABLE expense_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    event_id TEXT NOT NULL REFERENCES expense_events(id) ON DELETE RESTRICT,
    recurring_expense_id TEXT REFERENCES recurring_expense_items(id) ON DELETE RESTRICT,
    review_reason TEXT NOT NULL CHECK(review_reason IN (
        'unknown_p2p', 'ambiguous_mirror', 'recurring_match_candidate',
        'recurring_registration_candidate', 'category_confirmation', 'import_rejected',
        'manual_override'
    )),
    review_status TEXT NOT NULL DEFAULT 'pending' CHECK(review_status IN ('pending', 'resolved')),
    suggested_kind TEXT,
    suggested_category TEXT,
    suggested_duplicate_of_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    resolved_kind TEXT,
    resolved_category TEXT,
    duplicate_of_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    create_rule INTEGER NOT NULL DEFAULT 0 CHECK(create_rule IN (0, 1)),
    created_at TEXT NOT NULL,
    resolved_at TEXT,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    UNIQUE(event_id, review_reason, recurring_expense_id)
);

CREATE TABLE expense_rules (
    id TEXT PRIMARY KEY NOT NULL,
    rule_kind TEXT NOT NULL CHECK(rule_kind IN ('classification', 'recurring_match')),
    merchant_blind_index TEXT NOT NULL,
    payment_method_fingerprint TEXT,
    event_kind TEXT NOT NULL,
    category TEXT NOT NULL,
    recurring_expense_id TEXT REFERENCES recurring_expense_items(id) ON DELETE RESTRICT,
    created_at TEXT NOT NULL,
    UNIQUE(rule_kind, merchant_blind_index, payment_method_fingerprint, recurring_expense_id)
);

CREATE TABLE recurring_expense_items (
    id TEXT PRIMARY KEY NOT NULL,
    name_key_version INTEGER NOT NULL,
    name_nonce TEXT NOT NULL,
    name_ciphertext TEXT NOT NULL,
    name_aad TEXT NOT NULL,
    name_blind_index TEXT NOT NULL,
    category TEXT NOT NULL,
    vendor_key_version INTEGER,
    vendor_nonce TEXT,
    vendor_ciphertext TEXT,
    vendor_aad TEXT,
    vendor_blind_index TEXT,
    amount_minor INTEGER NOT NULL CHECK(amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    payment_method_fingerprint TEXT,
    start_date TEXT NOT NULL,
    end_date TEXT,
    memo_key_version INTEGER,
    memo_nonce TEXT,
    memo_ciphertext TEXT,
    memo_aad TEXT,
    memo_blind_index TEXT,
    reminder_days INTEGER NOT NULL DEFAULT 7 CHECK(reminder_days BETWEEN 0 AND 90),
    amount_kind TEXT NOT NULL CHECK(amount_kind IN ('fixed', 'estimate', 'limit')),
    interval_months INTEGER NOT NULL CHECK(interval_months IN (1, 2, 3, 6, 12)),
    due_rule TEXT NOT NULL CHECK(due_rule IN ('specific_day', 'first_day', 'last_day')),
    due_day INTEGER CHECK(due_day BETWEEN 1 AND 31),
    item_status TEXT NOT NULL CHECK(item_status IN ('active', 'paused', 'ended')),
    auto_match_enabled INTEGER NOT NULL DEFAULT 0 CHECK(auto_match_enabled IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    CHECK(end_date IS NULL OR start_date <= end_date),
    CHECK((due_rule = 'specific_day') = (due_day IS NOT NULL)),
    CHECK((vendor_key_version IS NULL) = (vendor_nonce IS NULL)),
    CHECK((vendor_key_version IS NULL) = (vendor_ciphertext IS NULL)),
    CHECK((vendor_key_version IS NULL) = (vendor_aad IS NULL)),
    CHECK((vendor_key_version IS NULL) = (vendor_blind_index IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_nonce IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_ciphertext IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_aad IS NULL)),
    CHECK((memo_key_version IS NULL) = (memo_blind_index IS NULL))
);

CREATE TABLE recurring_expense_versions (
    id TEXT PRIMARY KEY NOT NULL,
    recurring_expense_id TEXT NOT NULL REFERENCES recurring_expense_items(id) ON DELETE RESTRICT,
    item_version INTEGER NOT NULL CHECK(item_version > 0),
    effective_from_month TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json)),
    created_at TEXT NOT NULL,
    UNIQUE(recurring_expense_id, item_version)
);

CREATE TABLE recurring_expense_occurrences (
    occurrence_key TEXT PRIMARY KEY NOT NULL,
    recurring_expense_id TEXT NOT NULL REFERENCES recurring_expense_items(id) ON DELETE RESTRICT,
    item_version INTEGER NOT NULL CHECK(item_version > 0),
    due_date TEXT NOT NULL,
    expected_amount_minor INTEGER NOT NULL CHECK(expected_amount_minor > 0),
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    occurrence_status TEXT NOT NULL CHECK(occurrence_status IN ('paid', 'matched')),
    actual_amount_minor INTEGER NOT NULL CHECK(actual_amount_minor > 0),
    actual_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    manual_event_id TEXT REFERENCES expense_events(id) ON DELETE RESTRICT,
    amount_changed INTEGER NOT NULL DEFAULT 0 CHECK(amount_changed IN (0, 1)),
    confirmed_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    UNIQUE(recurring_expense_id, due_date),
    CHECK((occurrence_status = 'matched') = (actual_event_id IS NOT NULL)),
    CHECK((occurrence_status = 'paid') = (manual_event_id IS NOT NULL))
);

CREATE TABLE expense_month_reports (
    month_start TEXT NOT NULL,
    currency TEXT NOT NULL CHECK(length(currency) = 3 AND currency = upper(currency)),
    report_status TEXT NOT NULL CHECK(report_status IN ('confirmed', 'provisional', 'incomplete')),
    aggregate_sha256 TEXT NOT NULL CHECK(length(aggregate_sha256) = 64),
    aggregate_json TEXT NOT NULL CHECK(json_valid(aggregate_json)),
    generated_at TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    PRIMARY KEY(month_start, currency)
);

CREATE TABLE expense_ai_reports (
    id TEXT PRIMARY KEY NOT NULL,
    month_start TEXT NOT NULL,
    aggregate_sha256 TEXT NOT NULL CHECK(length(aggregate_sha256) = 64),
    prompt_version TEXT NOT NULL,
    model TEXT NOT NULL,
    result_json TEXT NOT NULL CHECK(json_valid(result_json)),
    input_tokens INTEGER NOT NULL DEFAULT 0 CHECK(input_tokens >= 0),
    cached_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK(cached_input_tokens >= 0),
    output_tokens INTEGER NOT NULL DEFAULT 0 CHECK(output_tokens >= 0),
    total_tokens INTEGER NOT NULL DEFAULT 0 CHECK(total_tokens >= 0),
    cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK(cost_microusd >= 0),
    latency_ms INTEGER NOT NULL DEFAULT 0 CHECK(latency_ms >= 0),
    created_at TEXT NOT NULL,
    UNIQUE(aggregate_sha256, prompt_version)
);

CREATE TABLE expense_ai_feedback (
    id TEXT PRIMARY KEY NOT NULL,
    report_id TEXT NOT NULL REFERENCES expense_ai_reports(id) ON DELETE RESTRICT,
    helpful INTEGER NOT NULL CHECK(helpful IN (0, 1)),
    created_at TEXT NOT NULL,
    UNIQUE(report_id)
);

CREATE TABLE expense_ai_request_bindings (
    request_id TEXT PRIMARY KEY NOT NULL,
    report_month_start TEXT NOT NULL,
    aggregate_sha256 TEXT NOT NULL CHECK(length(aggregate_sha256) = 64),
    created_at TEXT NOT NULL
);

CREATE TABLE expense_ai_attempts (
    request_id TEXT PRIMARY KEY NOT NULL,
    month_start TEXT NOT NULL,
    report_month_start TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number BETWEEN 1 AND 8),
    attempt_status TEXT NOT NULL DEFAULT 'claimed'
        CHECK(attempt_status IN ('claimed', 'succeeded', 'failed')),
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    failure_code TEXT,
    created_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE(month_start, attempt_number),
    CHECK((attempt_status = 'succeeded') = (result_json IS NOT NULL)),
    CHECK((attempt_status = 'failed') = (failure_code IS NOT NULL)),
    CHECK(attempt_status = 'claimed' OR completed_at IS NOT NULL)
);

CREATE TABLE expense_mutation_receipts (
    idempotency_key TEXT PRIMARY KEY NOT NULL,
    operation TEXT NOT NULL,
    request_sha256 TEXT NOT NULL CHECK(length(request_sha256) = 64),
    actor TEXT NOT NULL,
    response_json TEXT NOT NULL CHECK(json_valid(response_json)),
    created_at TEXT NOT NULL
);

CREATE INDEX idx_expense_import_batches_source_created
    ON expense_import_batches(source_id, created_at DESC);
CREATE INDEX idx_expense_import_preview_expiry
    ON expense_import_preview_sessions(expires_at, consumed_at);
CREATE INDEX idx_expense_raw_rows_batch ON expense_raw_rows(batch_id, source_row_number);
CREATE INDEX idx_expense_raw_rows_source_digest
    ON expense_raw_rows(source_id, row_sha256);
CREATE INDEX idx_expense_postings_date ON expense_postings(posted_date DESC, id DESC);
CREATE INDEX idx_expense_postings_merchant
    ON expense_postings(merchant_blind_index, payment_method_fingerprint, posted_date);
CREATE INDEX idx_expense_events_month
    ON expense_events(posted_date DESC, id DESC, event_status, currency);
CREATE INDEX idx_expense_events_posting ON expense_events(primary_posting_id);
CREATE UNIQUE INDEX idx_expense_allocations_personal_unique
    ON expense_allocations(event_id)
    WHERE allocation_kind = 'personal';
CREATE UNIQUE INDEX idx_expense_allocations_settlement_event_unique
    ON expense_allocations(event_id)
    WHERE allocation_kind IN ('settlement_received', 'settlement_sent');
CREATE INDEX idx_expense_reviews_queue
    ON expense_reviews(review_status, created_at DESC, id DESC);
CREATE UNIQUE INDEX idx_expense_rules_classification_unique
    ON expense_rules(merchant_blind_index, coalesce(payment_method_fingerprint, ''))
    WHERE rule_kind = 'classification';
CREATE UNIQUE INDEX idx_expense_rules_recurring_match_unique
    ON expense_rules(
        recurring_expense_id,
        merchant_blind_index,
        coalesce(payment_method_fingerprint, '')
    )
    WHERE rule_kind = 'recurring_match';
CREATE INDEX idx_recurring_expense_versions_effective
    ON recurring_expense_versions(recurring_expense_id, effective_from_month DESC);
CREATE INDEX idx_recurring_occurrences_due
    ON recurring_expense_occurrences(due_date, occurrence_status);
CREATE UNIQUE INDEX idx_recurring_occurrences_actual_event_unique
    ON recurring_expense_occurrences(actual_event_id)
    WHERE actual_event_id IS NOT NULL;
CREATE INDEX idx_expense_ai_reports_month
    ON expense_ai_reports(month_start, created_at DESC);
CREATE INDEX idx_expense_ai_attempts_month
    ON expense_ai_attempts(month_start, created_at);

CREATE TRIGGER expense_raw_rows_immutable_update
BEFORE UPDATE ON expense_raw_rows
BEGIN
    SELECT RAISE(ABORT, 'expense raw rows are immutable');
END;

CREATE TRIGGER expense_raw_rows_immutable_delete
BEFORE DELETE ON expense_raw_rows
BEGIN
    SELECT RAISE(ABORT, 'expense raw rows are immutable');
END;

CREATE TRIGGER expense_postings_immutable_update
BEFORE UPDATE ON expense_postings
BEGIN
    SELECT RAISE(ABORT, 'expense postings are immutable');
END;

CREATE TRIGGER expense_postings_immutable_delete
BEFORE DELETE ON expense_postings
BEGIN
    SELECT RAISE(ABORT, 'expense postings are immutable');
END;

CREATE TRIGGER expense_event_postings_immutable_update
BEFORE UPDATE ON expense_event_postings
BEGIN
    SELECT RAISE(ABORT, 'expense event posting links are immutable');
END;

CREATE TRIGGER expense_event_postings_immutable_delete
BEFORE DELETE ON expense_event_postings
BEGIN
    SELECT RAISE(ABORT, 'expense event posting links are immutable');
END;

CREATE TRIGGER recurring_expense_versions_immutable_update
BEFORE UPDATE ON recurring_expense_versions
BEGIN
    SELECT RAISE(ABORT, 'recurring expense versions are immutable');
END;

CREATE TRIGGER recurring_expense_versions_immutable_delete
BEFORE DELETE ON recurring_expense_versions
BEGIN
    SELECT RAISE(ABORT, 'recurring expense versions are immutable');
END;
