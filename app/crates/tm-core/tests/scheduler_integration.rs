use chrono::{TimeZone, Utc};
use tempfile::{Builder, TempDir};
use tm_core::{DEFAULT_TM_HOME, Result, TmCore, TmHome};

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-scheduler-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn instant(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
        .expect("valid UTC test time")
}

#[test]
fn canary_effect_is_exactly_once_across_duplicate_cycles_and_restart() -> Result<()> {
    let (temporary, core) = fixture()?;
    let now = instant(2026, 7, 22, 0, 0, 0);
    let first = core.run_scheduler_cycle("worker-a", now)?;
    assert_eq!(first.succeeded, 1);

    let duplicate = core.run_scheduler_cycle("worker-b", now)?;
    assert_eq!(duplicate.succeeded, 0);
    let runs = core.list_scheduler_runs()?;
    assert_eq!(
        runs.iter()
            .filter(|run| run.job_key == "scheduler.canary" && run.status == "succeeded")
            .count(),
        1
    );

    drop(core);
    let reopened = TmCore::open(TmHome::new(temporary.path()))?;
    let restarted =
        reopened.run_scheduler_cycle("worker-after-restart", now + chrono::Duration::minutes(1))?;
    assert_eq!(restarted.succeeded, 0);
    assert_eq!(
        reopened
            .list_scheduler_runs()?
            .iter()
            .filter(|run| run.job_key == "scheduler.canary" && run.status == "succeeded")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn expired_lease_is_recovered_without_duplicate_effect() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let now = instant(2026, 7, 22, 1, 0, 0);
    core.ensure_scheduler_defaults(now)?;
    core.reconcile_scheduler(now)?;
    let (first, recovered) = core.claim_scheduler_run("worker-crashed", now)?;
    assert_eq!(recovered, 0);
    let first = first.expect("initial claim");
    assert_eq!(first.attempt_number, 1);

    let recovery_time = now + chrono::Duration::seconds(301);
    let (during_backoff, recovered) = core.claim_scheduler_run("worker-recovery", recovery_time)?;
    assert!(during_backoff.is_none());
    assert_eq!(recovered, 1);

    let (second, recovered_again) = core.claim_scheduler_run(
        "worker-recovery",
        recovery_time + chrono::Duration::seconds(30),
    )?;
    assert_eq!(recovered_again, 0);
    let second = second.expect("recovered claim");
    assert_eq!(second.run_id, first.run_id);
    assert_eq!(second.attempt_number, 2);
    core.execute_scheduler_claim(&second, recovery_time + chrono::Duration::seconds(30))?;

    let run = core
        .list_scheduler_runs()?
        .into_iter()
        .find(|run| run.id == first.run_id)
        .expect("scheduler run");
    assert_eq!(run.status, "succeeded");
    assert_eq!(run.attempt_count, 2);
    Ok(())
}

#[test]
fn failures_follow_backoff_and_end_in_dead_letter() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let mut now = instant(2026, 7, 22, 2, 0, 0);
    core.ensure_scheduler_defaults(now)?;
    core.reconcile_scheduler(now)?;
    let (claim, _) = core.claim_scheduler_run("worker-failing", now)?;
    let mut claim = claim.expect("initial claim");
    let delays = [30, 120, 600, 3600];

    for attempt in 1..=5 {
        assert_eq!(claim.attempt_number, attempt);
        core.fail_scheduler_claim(&claim, "synthetic retryable failure", true, now)?;
        if attempt < 5 {
            now += chrono::Duration::seconds(delays[(attempt - 1) as usize]);
            let (next, _) = core.claim_scheduler_run("worker-failing", now)?;
            claim = next.expect("retry claim");
        }
    }

    let run = core
        .list_scheduler_runs()?
        .into_iter()
        .find(|run| run.id == claim.run_id)
        .expect("dead-letter run");
    assert_eq!(run.status, "dead_letter");
    assert_eq!(run.attempt_count, 5);
    assert_eq!(core.scheduler_status(now)?.dead_letter_count, 1);
    Ok(())
}

#[test]
fn missed_repeating_runs_are_coalesced_and_old_occurrence_is_audited() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let initial = instant(2026, 1, 1, 0, 0, 0);
    core.ensure_scheduler_defaults(initial)?;
    let resumed = initial + chrono::Duration::days(3) + chrono::Duration::minutes(5);
    core.run_scheduler_cycle("worker-resumed", resumed)?;

    let canary_runs = core
        .list_scheduler_runs()?
        .into_iter()
        .filter(|run| run.job_key == "scheduler.canary")
        .collect::<Vec<_>>();
    assert_eq!(canary_runs.len(), 2);
    assert_eq!(
        canary_runs
            .iter()
            .filter(|run| run.status == "skipped")
            .count(),
        1
    );
    assert_eq!(
        canary_runs
            .iter()
            .filter(|run| run.status == "succeeded")
            .count(),
        1
    );
    Ok(())
}

#[test]
fn restore_preserves_completed_scheduler_effect_ledger() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let now = instant(2026, 7, 22, 4, 0, 0);
    core.ensure_scheduler_defaults(now)?;
    let before_effect = core.create_backup()?;
    core.run_scheduler_cycle("worker-before-restore", now)?;
    let completed_id = core
        .list_scheduler_runs()?
        .into_iter()
        .find(|run| run.job_key == "scheduler.canary" && run.status == "succeeded")
        .expect("completed canary")
        .id;

    core.restore_backup(&before_effect.path)?;
    let preserved = core
        .list_scheduler_runs()?
        .into_iter()
        .find(|run| run.id == completed_id)
        .expect("preserved scheduler run");
    assert_eq!(preserved.status, "succeeded");
    assert_eq!(
        core.run_scheduler_cycle("worker-after-restore", now)?
            .succeeded,
        0
    );
    Ok(())
}
