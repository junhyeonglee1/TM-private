CREATE TABLE ai_budget_ledger (
    id TEXT PRIMARY KEY,
    request_id TEXT NOT NULL,
    entry_kind TEXT NOT NULL CHECK (entry_kind IN ('reservation', 'settlement')),
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    operation TEXT NOT NULL,
    budget_month TEXT NOT NULL CHECK (
        length(budget_month) = 7 AND substr(budget_month, 5, 1) = '-'
    ),
    amount_microusd INTEGER NOT NULL,
    input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
    cached_input_tokens INTEGER CHECK (
        cached_input_tokens IS NULL OR cached_input_tokens >= 0
    ),
    output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0),
    total_tokens INTEGER CHECK (total_tokens IS NULL OR total_tokens >= 0),
    outcome TEXT NOT NULL CHECK (
        outcome IN ('reserved', 'succeeded', 'preflight_failed', 'upstream_cost_estimate')
    ),
    created_at TEXT NOT NULL,
    UNIQUE(request_id, entry_kind),
    CHECK (
        (entry_kind = 'reservation' AND amount_microusd > 0 AND outcome = 'reserved')
        OR entry_kind = 'settlement'
    )
) STRICT;

CREATE INDEX idx_ai_budget_ledger_month_created
    ON ai_budget_ledger(budget_month, created_at, id);

CREATE TRIGGER ai_budget_ledger_no_update
BEFORE UPDATE ON ai_budget_ledger
BEGIN
    SELECT RAISE(ABORT, 'AI budget ledger is append-only');
END;

CREATE TRIGGER ai_budget_ledger_no_delete
BEFORE DELETE ON ai_budget_ledger
BEGIN
    SELECT RAISE(ABORT, 'AI budget ledger cannot be deleted');
END;
