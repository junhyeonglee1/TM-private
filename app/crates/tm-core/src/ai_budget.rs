use chrono::Utc;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use crate::{
    AiBudgetPolicy, AiBudgetReservation, AiBudgetStatus, AiTokenUsage, Error, Result,
    database::{Database, new_id, now_utc},
    error::invalid,
};

pub(crate) fn status(database: &Database, policy: AiBudgetPolicy) -> Result<AiBudgetStatus> {
    validate_policy(policy)?;
    let connection = database.connect()?;
    status_for_month(&connection, &month_utc(), policy)
}

pub(crate) fn reserve(
    database: &Database,
    request_id: &str,
    provider: &str,
    model: &str,
    operation: &str,
    maximum_cost_microusd: u64,
    policy: AiBudgetPolicy,
) -> Result<AiBudgetReservation> {
    validate_policy(policy)?;
    validate_label("request ID", request_id, 128)?;
    validate_label("AI provider", provider, 64)?;
    validate_label("AI model", model, 128)?;
    validate_label("AI operation", operation, 64)?;
    if maximum_cost_microusd == 0 || maximum_cost_microusd > policy.hard_limit_microusd {
        return Err(invalid(
            "AI maximum request cost must be positive and no greater than the monthly hard limit",
        ));
    }
    let requested = as_i64(maximum_cost_microusd, "AI maximum request cost")?;
    let hard_limit = as_i64(policy.hard_limit_microusd, "AI monthly hard limit")?;
    let month = month_utc();

    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let existing = transaction
            .query_row(
                "SELECT provider, model, operation, budget_month, amount_microusd
                 FROM ai_budget_ledger
                 WHERE request_id = ?1 AND entry_kind = 'reservation'",
                [request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;
        if let Some((saved_provider, saved_model, saved_operation, saved_month, saved_amount)) =
            existing
        {
            if saved_provider != provider
                || saved_model != model
                || saved_operation != operation
                || saved_month != month
                || saved_amount != requested
            {
                return Err(Error::Conflict(
                    "AI budget request ID was already used with different reservation data"
                        .to_owned(),
                ));
            }
            return Ok(AiBudgetReservation {
                request_id: request_id.to_owned(),
                budget_month: saved_month,
                reserved_microusd: maximum_cost_microusd,
            });
        }

        let current: i64 = transaction.query_row(
            "SELECT COALESCE(SUM(amount_microusd), 0)
             FROM ai_budget_ledger WHERE budget_month = ?1",
            [&month],
            |row| row.get(0),
        )?;
        if current.saturating_add(requested) > hard_limit {
            return Err(Error::AiBudgetExceeded {
                current_microusd: current.max(0) as u64,
                requested_microusd: maximum_cost_microusd,
                hard_limit_microusd: policy.hard_limit_microusd,
            });
        }

        transaction.execute(
            "INSERT INTO ai_budget_ledger(
                id, request_id, entry_kind, provider, model, operation, budget_month,
                amount_microusd, input_tokens, cached_input_tokens, output_tokens,
                total_tokens, outcome, created_at
             ) VALUES (?1, ?2, 'reservation', ?3, ?4, ?5, ?6, ?7,
                       NULL, NULL, NULL, NULL, 'reserved', ?8)",
            params![
                new_id(),
                request_id,
                provider,
                model,
                operation,
                month,
                requested,
                now_utc(),
            ],
        )?;
        Ok(AiBudgetReservation {
            request_id: request_id.to_owned(),
            budget_month: month,
            reserved_microusd: maximum_cost_microusd,
        })
    })
}

pub(crate) fn settle(
    database: &Database,
    reservation: &AiBudgetReservation,
    actual_cost_microusd: u64,
    usage: Option<AiTokenUsage>,
    outcome: &str,
    policy: AiBudgetPolicy,
) -> Result<AiBudgetStatus> {
    validate_policy(policy)?;
    if !matches!(
        outcome,
        "succeeded" | "preflight_failed" | "upstream_cost_estimate"
    ) {
        return Err(invalid("AI budget settlement outcome is invalid"));
    }
    if let Some(usage) = usage
        && (usage.cached_input_tokens > usage.input_tokens
            || usage.input_tokens.saturating_add(usage.output_tokens) > usage.total_tokens)
    {
        return Err(invalid("AI token usage is inconsistent"));
    }
    let actual = as_i64(actual_cost_microusd, "AI actual request cost")?;

    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let reservation_row = transaction
            .query_row(
                "SELECT provider, model, operation, budget_month, amount_microusd
                 FROM ai_budget_ledger
                 WHERE request_id = ?1 AND entry_kind = 'reservation'",
                [&reservation.request_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| {
                Error::Invariant("AI budget reservation is missing during settlement".to_owned())
            })?;
        let (provider, model, operation, month, reserved) = reservation_row;
        if month != reservation.budget_month
            || reserved != as_i64(reservation.reserved_microusd, "AI reserved cost")?
        {
            return Err(Error::Conflict(
                "AI budget reservation does not match the persisted ledger".to_owned(),
            ));
        }

        let already_settled: bool = transaction.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM ai_budget_ledger
                WHERE request_id = ?1 AND entry_kind = 'settlement'
             )",
            [&reservation.request_id],
            |row| row.get(0),
        )?;
        if !already_settled {
            let (input, cached, output, total) = usage.map_or((None, None, None, None), |usage| {
                (
                    Some(usage.input_tokens),
                    Some(usage.cached_input_tokens),
                    Some(usage.output_tokens),
                    Some(usage.total_tokens),
                )
            });
            transaction.execute(
                "INSERT INTO ai_budget_ledger(
                    id, request_id, entry_kind, provider, model, operation, budget_month,
                    amount_microusd, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, outcome, created_at
                 ) VALUES (?1, ?2, 'settlement', ?3, ?4, ?5, ?6, ?7,
                           ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    new_id(),
                    reservation.request_id,
                    provider,
                    model,
                    operation,
                    month,
                    actual.saturating_sub(reserved),
                    input
                        .map(|value| as_i64(value, "AI input tokens"))
                        .transpose()?,
                    cached
                        .map(|value| as_i64(value, "AI cached input tokens"))
                        .transpose()?,
                    output
                        .map(|value| as_i64(value, "AI output tokens"))
                        .transpose()?,
                    total
                        .map(|value| as_i64(value, "AI total tokens"))
                        .transpose()?,
                    outcome,
                    now_utc(),
                ],
            )?;
        }
        status_for_month(transaction, &month, policy)
    })
}

fn status_for_month(
    connection: &rusqlite::Connection,
    month: &str,
    policy: AiBudgetPolicy,
) -> Result<AiBudgetStatus> {
    let committed: i64 = connection.query_row(
        "SELECT COALESCE(SUM(amount_microusd), 0)
         FROM ai_budget_ledger WHERE budget_month = ?1",
        [month],
        |row| row.get(0),
    )?;
    let committed = committed.max(0) as u64;
    Ok(AiBudgetStatus {
        budget_month: month.to_owned(),
        warning_limit_microusd: policy.warning_limit_microusd,
        hard_limit_microusd: policy.hard_limit_microusd,
        committed_microusd: committed,
        remaining_microusd: policy.hard_limit_microusd.saturating_sub(committed),
        warning_reached: committed >= policy.warning_limit_microusd,
        hard_stop_reached: committed >= policy.hard_limit_microusd,
    })
}

fn validate_policy(policy: AiBudgetPolicy) -> Result<()> {
    if policy.warning_limit_microusd == 0
        || policy.hard_limit_microusd == 0
        || policy.warning_limit_microusd > policy.hard_limit_microusd
        || policy.hard_limit_microusd > i64::MAX as u64
    {
        return Err(invalid("AI monthly budget policy is invalid"));
    }
    Ok(())
}

fn validate_label(field: &str, value: &str, maximum_length: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum_length || value.contains(['\r', '\n']) {
        return Err(invalid(format!("{field} has an invalid format")));
    }
    Ok(())
}

fn as_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid(format!("{field} is too large")))
}

fn month_utc() -> String {
    Utc::now().format("%Y-%m").to_string()
}
