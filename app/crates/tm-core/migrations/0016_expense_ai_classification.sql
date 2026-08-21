ALTER TABLE expense_events
ADD COLUMN classification_source TEXT NOT NULL DEFAULT 'deterministic'
    CHECK(classification_source IN ('deterministic', 'user_rule', 'manual', 'ai'));

ALTER TABLE expense_events
ADD COLUMN classification_confidence INTEGER
    CHECK(classification_confidence IS NULL
          OR classification_confidence BETWEEN 0 AND 100);

-- Schema 15 did not persist classification provenance. A resolved user-facing
-- category decision is the strongest evidence available during migration and
-- must take precedence over the deterministic default.
UPDATE expense_events
SET classification_source = 'manual',
    classification_confidence = NULL
WHERE EXISTS (
    SELECT 1
    FROM expense_reviews
    WHERE expense_reviews.event_id = expense_events.id
      AND expense_reviews.review_status = 'resolved'
      AND expense_reviews.review_reason IN ('category_confirmation', 'manual_override')
);

CREATE TABLE expense_ai_classification_batches (
    request_id TEXT PRIMARY KEY NOT NULL,
    quota_month_start TEXT NOT NULL,
    target_month_start TEXT NOT NULL,
    attempt_number INTEGER NOT NULL CHECK(attempt_number BETWEEN 1 AND 12),
    input_sha256 TEXT NOT NULL CHECK(length(input_sha256) = 64),
    prompt_version TEXT NOT NULL,
    model TEXT NOT NULL,
    batch_status TEXT NOT NULL DEFAULT 'claimed'
        CHECK(batch_status IN ('claimed', 'staged', 'applied', 'failed', 'stale')),
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)),
    item_group_count INTEGER NOT NULL CHECK(item_group_count > 0),
    review_count INTEGER NOT NULL CHECK(review_count > 0),
    result_count INTEGER NOT NULL DEFAULT 0 CHECK(result_count >= 0),
    confirmed_count INTEGER NOT NULL DEFAULT 0 CHECK(confirmed_count >= 0),
    provisional_count INTEGER NOT NULL DEFAULT 0 CHECK(provisional_count >= 0),
    review_required_count INTEGER NOT NULL DEFAULT 0 CHECK(review_required_count >= 0),
    privacy_skipped_count INTEGER NOT NULL DEFAULT 0 CHECK(privacy_skipped_count >= 0),
    version_conflict_count INTEGER NOT NULL DEFAULT 0 CHECK(version_conflict_count >= 0),
    input_tokens INTEGER NOT NULL DEFAULT 0 CHECK(input_tokens >= 0),
    cached_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK(cached_input_tokens >= 0),
    output_tokens INTEGER NOT NULL DEFAULT 0 CHECK(output_tokens >= 0),
    total_tokens INTEGER NOT NULL DEFAULT 0 CHECK(total_tokens >= 0),
    cost_microusd INTEGER NOT NULL DEFAULT 0 CHECK(cost_microusd >= 0),
    latency_ms INTEGER NOT NULL DEFAULT 0 CHECK(latency_ms >= 0),
    failure_code TEXT CHECK(failure_code IS NULL OR failure_code IN (
        'transport', 'authentication', 'rate_limited', 'request_rejected',
        'upstream_unavailable', 'invalid_response', 'model_mismatch', 'timeout',
        'budget_exhausted', 'disabled', 'lease_expired', 'persistence_failed',
        'version_conflict'
    )),
    created_at TEXT NOT NULL,
    completed_at TEXT,
    UNIQUE(quota_month_start, attempt_number),
    CHECK(cached_input_tokens <= input_tokens),
    CHECK(total_tokens = input_tokens + output_tokens),
    CHECK(result_count <= item_group_count),
    CHECK(confirmed_count + provisional_count + review_required_count
          + privacy_skipped_count <= review_count),
    CHECK((batch_status IN ('staged', 'applied', 'stale')) = (result_json IS NOT NULL)),
    CHECK((batch_status IN ('failed', 'stale')) = (failure_code IS NOT NULL)),
    CHECK((batch_status IN ('applied', 'failed', 'stale')) = (completed_at IS NOT NULL))
);

CREATE TABLE expense_ai_classification_items (
    batch_request_id TEXT NOT NULL
        REFERENCES expense_ai_classification_batches(request_id) ON DELETE RESTRICT,
    item_id TEXT NOT NULL,
    event_id TEXT NOT NULL REFERENCES expense_events(id) ON DELETE RESTRICT,
    review_id TEXT NOT NULL REFERENCES expense_reviews(id) ON DELETE RESTRICT,
    expected_event_version INTEGER NOT NULL CHECK(expected_event_version > 0),
    expected_review_version INTEGER NOT NULL CHECK(expected_review_version > 0),
    suggested_category TEXT CHECK(suggested_category IS NULL OR suggested_category IN (
        'food', 'delivery', 'cafe', 'groceries', 'housing_utilities',
        'transportation', 'ott_subscriptions', 'shopping', 'health', 'leisure',
        'education', 'travel', 'insurance_finance_tax', 'gifts_dues', 'other'
    )),
    confidence INTEGER CHECK(confidence IS NULL OR confidence BETWEEN 0 AND 100),
    disposition TEXT CHECK(disposition IS NULL OR disposition IN (
        'confirmed', 'provisional', 'review_required', 'privacy_skipped'
    )),
    created_at TEXT NOT NULL,
    applied_at TEXT,
    PRIMARY KEY(batch_request_id, review_id),
    UNIQUE(batch_request_id, event_id),
    CHECK((suggested_category IS NULL) = (confidence IS NULL)),
    CHECK(
        (disposition IS NULL AND suggested_category IS NULL AND applied_at IS NULL)
        OR (disposition = 'privacy_skipped'
            AND suggested_category IS NULL AND confidence IS NULL
            AND applied_at IS NOT NULL)
        OR (disposition IN ('confirmed', 'provisional', 'review_required')
            AND suggested_category IS NOT NULL AND confidence IS NOT NULL
            AND applied_at IS NOT NULL)
    )
);

CREATE TABLE expense_ai_classification_receipts (
    request_id TEXT PRIMARY KEY NOT NULL,
    target_month_start TEXT NOT NULL,
    terminal_status TEXT NOT NULL
        CHECK(terminal_status = 'no_candidates'),
    result_json TEXT NOT NULL CHECK(result_json =
        '{"itemGroupCount":0,"reviewCount":0,"resultCount":0,"confirmedCount":0,"provisionalCount":0,"reviewRequiredCount":0,"privacySkippedCount":0,"versionConflictCount":0,"actualCostMicrousd":0}'
    ),
    created_at TEXT NOT NULL,
    completed_at TEXT NOT NULL
);

CREATE TRIGGER expense_ai_classification_receipts_immutable_update
BEFORE UPDATE ON expense_ai_classification_receipts
BEGIN
    SELECT RAISE(ABORT, 'expense AI classification receipts are immutable');
END;

CREATE TRIGGER expense_ai_classification_receipts_immutable_delete
BEFORE DELETE ON expense_ai_classification_receipts
BEGIN
    SELECT RAISE(ABORT, 'expense AI classification receipts cannot be deleted');
END;

CREATE INDEX idx_expense_ai_classification_batches_quota
    ON expense_ai_classification_batches(quota_month_start, attempt_number);
CREATE INDEX idx_expense_ai_classification_batches_status
    ON expense_ai_classification_batches(batch_status, created_at);
CREATE UNIQUE INDEX idx_expense_ai_classification_batches_active_input
    ON expense_ai_classification_batches(input_sha256, prompt_version)
    WHERE batch_status IN ('claimed', 'staged', 'applied');
CREATE INDEX idx_expense_ai_classification_items_item
    ON expense_ai_classification_items(batch_request_id, item_id, review_id);
CREATE INDEX idx_expense_ai_classification_items_review
    ON expense_ai_classification_items(review_id, applied_at DESC);
