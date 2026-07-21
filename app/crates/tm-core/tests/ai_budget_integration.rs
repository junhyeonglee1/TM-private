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

    let replayed = core.settle_ai_budget(&reservation, 500, Some(usage), "succeeded", policy())?;
    assert_eq!(replayed.committed_microusd, 500);
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
