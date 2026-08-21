use tempfile::TempDir;
use tm_core::{AiBudgetPolicy, AiTokenUsage, Error, Result, TmCore, TmHome};

fn fixture() -> Result<(TempDir, TmCore)> {
    let temporary = tempfile::Builder::new().prefix("tm-ai-budget-").tempdir()?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn policy() -> AiBudgetPolicy {
    AiBudgetPolicy {
        warning_limit_microusd: 10_000_000,
        hard_limit_microusd: 20_000_000,
    }
}

#[test]
fn reservation_and_settlement_track_actual_cost_without_double_charging() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let reservation = core.reserve_ai_budget(
        "019b0000-0000-7000-8000-000000000001",
        "openai",
        "gpt-5.6",
        "probe",
        10_000,
        policy(),
    )?;
    assert_eq!(
        core.get_ai_budget_reservation(&reservation.request_id)?,
        Some(reservation.clone())
    );
    assert!(
        core.get_ai_budget_settlement(&reservation.request_id)?
            .is_none()
    );
    assert_eq!(core.ai_budget_status(policy())?.committed_microusd, 10_000);

    let usage = AiTokenUsage {
        input_tokens: 40,
        cached_input_tokens: 10,
        output_tokens: 8,
        total_tokens: 48,
    };
    let settled = core.settle_ai_budget(&reservation, 500, Some(usage), "succeeded", policy())?;
    assert_eq!(settled.committed_microusd, 500);
    assert_eq!(settled.remaining_microusd, 19_999_500);
    let persisted_settlement = core
        .get_ai_budget_settlement(&reservation.request_id)?
        .expect("persisted settlement");
    assert_eq!(persisted_settlement.actual_cost_microusd, 500);
    assert_eq!(persisted_settlement.outcome, "succeeded");

    let replayed = core.settle_ai_budget(&reservation, 500, Some(usage), "succeeded", policy())?;
    assert_eq!(replayed.committed_microusd, 500);
    assert!(matches!(
        core.settle_ai_budget(&reservation, 501, Some(usage), "succeeded", policy()),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        core.settle_ai_budget(
            &reservation,
            500,
            Some(usage),
            "upstream_cost_estimate",
            policy()
        ),
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        core.settle_ai_budget(
            &reservation,
            500,
            Some(AiTokenUsage {
                output_tokens: 9,
                total_tokens: 49,
                ..usage
            }),
            "succeeded",
            policy()
        ),
        Err(Error::Conflict(_))
    ));
    Ok(())
}

#[test]
fn concurrent_budget_reservations_cannot_cross_the_hard_limit() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.reserve_ai_budget(
        "019b0000-0000-7000-8000-000000000002",
        "openai",
        "gpt-5.6",
        "assistant",
        19_999_999,
        policy(),
    )?;

    let rejected = core.reserve_ai_budget(
        "019b0000-0000-7000-8000-000000000003",
        "openai",
        "gpt-5.6",
        "assistant",
        2,
        policy(),
    );
    assert!(matches!(rejected, Err(Error::AiBudgetExceeded { .. })));
    assert_eq!(
        core.ai_budget_status(policy())?.committed_microusd,
        19_999_999
    );
    Ok(())
}

#[test]
fn preflight_failure_releases_the_full_reservation() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let reservation = core.reserve_ai_budget(
        "019b0000-0000-7000-8000-000000000004",
        "openai",
        "gpt-5.6",
        "probe",
        10_000,
        policy(),
    )?;
    let status = core.settle_ai_budget(&reservation, 0, None, "preflight_failed", policy())?;
    assert_eq!(status.committed_microusd, 0);
    assert_eq!(status.remaining_microusd, 20_000_000);
    Ok(())
}

#[test]
fn restore_cannot_rewind_the_append_only_ai_cost_ledger() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let before_usage = core.create_backup()?;
    let reservation = core.reserve_ai_budget(
        "019b0000-0000-7000-8000-000000000005",
        "openai",
        "gpt-5.6",
        "probe",
        10_000,
        policy(),
    )?;
    core.settle_ai_budget(&reservation, 250, None, "upstream_cost_estimate", policy())?;

    let rewind = core.restore_backup(&before_usage.path);
    assert!(matches!(rewind, Err(Error::Conflict(_))));

    let matching = core.create_backup()?;
    core.restore_backup(&matching.path)?;
    assert_eq!(core.ai_budget_status(policy())?.committed_microusd, 250);
    Ok(())
}

#[test]
fn stock_operation_cap_is_atomic_and_isolated_from_other_ai_operations() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let first = core.reserve_ai_budget_with_operation_limit(
        "019b0000-0000-7000-8000-000000000006",
        "openai",
        "gpt-5.4-nano-2026-03-17",
        "stock_daily_report",
        1_500_000,
        policy(),
        2_000_000,
    )?;
    let rejected = core.reserve_ai_budget_with_operation_limit(
        "019b0000-0000-7000-8000-000000000007",
        "openai",
        "gpt-5.4-nano-2026-03-17",
        "stock_daily_report",
        500_001,
        policy(),
        2_000_000,
    );
    assert!(matches!(
        rejected,
        Err(Error::AiBudgetExceeded {
            hard_limit_microusd: 2_000_000,
            ..
        })
    ));
    core.reserve_ai_budget_with_operation_limit(
        "019b0000-0000-7000-8000-000000000008",
        "openai",
        "gpt-5.4-nano-2026-03-17",
        "task_report",
        500_001,
        policy(),
        2_000_000,
    )?;
    let before_settlement = core.ai_operation_budget_status("stock_daily_report", 2_000_000)?;
    assert_eq!(before_settlement.committed_microusd, 1_500_000);
    assert_eq!(before_settlement.remaining_microusd, 500_000);

    core.settle_ai_budget(&first, 1_000, None, "succeeded", policy())?;
    let settled = core.ai_operation_budget_status("stock_daily_report", 2_000_000)?;
    assert_eq!(settled.committed_microusd, 1_000);
    assert_eq!(settled.remaining_microusd, 1_999_000);
    Ok(())
}
