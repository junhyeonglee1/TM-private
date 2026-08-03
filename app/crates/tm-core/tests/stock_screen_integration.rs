use std::path::PathBuf;

use chrono::{Duration, NaiveDate};
use rusqlite::{Connection, functions::FunctionFlags, params};
use tempfile::{Builder, TempDir};
use tm_core::{
    DEFAULT_TM_HOME, Error, ListStockScreenResultsInput, Result, StockAiReportStart,
    StockDailyBarInput, StockScreenBandFilter, StockScreenDirection, StockScreenHorizon,
    StockScreenStartInput, StockUniverseMemberInput, StoreStockMarketDataInput,
    StoreStockUniverseInput, TmCore, TmHome,
};
use uuid::Uuid;

const UNIVERSE_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const MARKET_SHA: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SCHEMA_TWELVE_MIGRATIONS: [&str; 12] = [
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_change_requests.sql"),
    include_str!("../migrations/0003_change_request_strict_cas.sql"),
    include_str!("../migrations/0004_controlled_mutations.sql"),
    include_str!("../migrations/0005_ai_budget_guard.sql"),
    include_str!("../migrations/0006_assistant_action_approvals.sql"),
    include_str!("../migrations/0007_assistant_memory.sql"),
    include_str!("../migrations/0008_durable_scheduler.sql"),
    include_str!("../migrations/0009_device_auth.sql"),
    include_str!("../migrations/0010_task_reports.sql"),
    include_str!("../migrations/0011_calendar_events.sql"),
    include_str!("../migrations/0012_stock_watchlist.sql"),
];

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-stock-screen-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn schema_twelve_fixture(prefix: &str) -> Result<(TempDir, PathBuf)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new().prefix(prefix).tempdir_in(test_runs)?;
    let database_path = temporary.path().join("data").join("tm.sqlite3");
    std::fs::create_dir_all(
        database_path.parent().ok_or_else(|| {
            Error::Invariant("schema 12 fixture database has no parent".to_owned())
        })?,
    )?;
    let connection = Connection::open(&database_path)?;
    connection.create_scalar_function("tm_uuid_v7", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(Uuid::now_v7().to_string())
    })?;
    connection.create_scalar_function("tm_now_utc", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok("2026-07-24T00:00:00.000Z".to_owned())
    })?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    for (index, migration) in SCHEMA_TWELVE_MIGRATIONS.iter().enumerate() {
        let version = i64::try_from(index + 1)
            .map_err(|error| Error::Invariant(format!("invalid fixture version: {error}")))?;
        connection.execute_batch(migration)?;
        connection.execute(
            "INSERT INTO schema_migrations(version, name, applied_at)
             VALUES (?1, ?2, '2026-07-24T00:00:00.000Z')",
            params![version, format!("schema-{version}-fixture")],
        )?;
        connection.pragma_update(None, "user_version", version)?;
    }
    drop(connection);
    Ok((temporary, database_path))
}

fn market_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 24).expect("valid synthetic date")
}

fn store_universe(core: &TmCore, count: usize) -> Result<tm_core::StockUniverseSnapshot> {
    core.store_stock_universe(StoreStockUniverseInput {
        name: "S&P 500 constituents (synthetic test)".to_owned(),
        effective_date: market_date(),
        source_url: "https://example.test/constituents.csv".to_owned(),
        source_revision: format!("fixture-{count}"),
        source_sha256: UNIVERSE_SHA.to_owned(),
        license_name: "PDDL-1.0".to_owned(),
        license_url: "https://example.test/license".to_owned(),
        attribution_text: "Synthetic fixture derived from public-domain data.".to_owned(),
        members: (0..count)
            .map(|index| StockUniverseMemberInput {
                ticker: format!("T{index:04}"),
                display_name: format!("Synthetic company {index}"),
                sector: Some("Synthetic".to_owned()),
                sub_industry: None,
            })
            .collect(),
    })
}

fn session_dates() -> Vec<NaiveDate> {
    (0..22)
        .map(|index| market_date() - Duration::days(i64::from(21 - index)))
        .collect()
}

fn store_full_prices(core: &TmCore, count: usize) -> Result<()> {
    let dates = session_dates();
    let mut bars = Vec::new();
    for (date_index, session_date) in dates.iter().enumerate() {
        for ticker_index in 0..count {
            let close = if date_index == 21 {
                match ticker_index {
                    0 => 110_000_000,
                    1 => 120_000_000,
                    2 => 90_000_000,
                    3 => 80_000_000,
                    _ => 100_000_000,
                }
            } else {
                100_000_000
            };
            bars.push(StockDailyBarInput {
                ticker: format!("T{ticker_index:04}"),
                session_date: *session_date,
                close_microusd: close,
            });
        }
    }
    core.store_stock_market_data(StoreStockMarketDataInput {
        source: "synthetic".to_owned(),
        feed: "test".to_owned(),
        adjustment: "split".to_owned(),
        source_sha256: MARKET_SHA.to_owned(),
        sessions: dates,
        bars,
    })?;
    Ok(())
}

#[test]
fn calculates_exact_bands_and_returns_stable_paginated_results() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 10)?;
    assert_eq!(
        core.get_stock_universe_snapshot(&universe.id)?,
        Some(universe.clone())
    );
    assert!(
        core.get_stock_universe_snapshot("missing-snapshot")?
            .is_none()
    );
    assert!(matches!(
        core.get_stock_universe_snapshot(" "),
        Err(Error::InvalidInput(_))
    ));
    store_full_prices(&core, 10)?;
    let start_input = StockScreenStartInput {
        run_id: Some("stable-stock-screen-run".to_owned()),
        market_date: market_date(),
        universe_snapshot_id: Some(universe.id.clone()),
    };
    let started = core.begin_stock_screen_run(start_input.clone())?;
    let replayed = core.begin_stock_screen_run(start_input)?;
    assert_eq!(replayed.id, started.id);
    assert_eq!(
        core.get_stock_screen_run(&started.id)?
            .expect("persisted screen run")
            .id,
        started.id
    );
    assert!(matches!(
        core.begin_stock_screen_run(StockScreenStartInput {
            run_id: Some(started.id.clone()),
            market_date: market_date() - Duration::days(1),
            universe_snapshot_id: Some(universe.id.clone()),
        }),
        Err(Error::Conflict(_))
    ));
    let completed = core.complete_stock_screen_run(&started.id, &universe.id, MARKET_SHA)?;
    assert_eq!(completed.status, "succeeded");
    assert_eq!(completed.result_count, 4);
    let terminal_replay = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: Some(completed.id.clone()),
        market_date: market_date(),
        universe_snapshot_id: None,
    })?;
    assert_eq!(terminal_replay.status, "succeeded");

    let latest = core.get_latest_stock_screen_as_of(market_date())?;
    assert!(!latest.stale);
    assert_eq!(latest.ai_budget.hard_limit_microusd, 2_000_000);
    assert_eq!(latest.ai_budget.operation, "stock_daily_report");
    assert_eq!(latest.ai_budget.committed_microusd, 0);
    assert_eq!(latest.ai_budget.remaining_microusd, 2_000_000);
    let success = latest.latest_success.expect("successful screen");
    assert_eq!(success.coverage.current_pct, 100.0);
    assert_eq!(success.counts.up_5_ten_to_twenty, 1);
    assert_eq!(success.counts.up_5_twenty_plus, 1);
    assert_eq!(success.counts.down_5_ten_to_twenty, 1);
    assert_eq!(success.counts.down_5_twenty_plus, 1);
    assert_eq!(success.top3.len(), 3);

    let first = core.list_stock_screen_results(ListStockScreenResultsInput {
        run_id: completed.id.clone(),
        horizon: StockScreenHorizon::Five,
        direction: StockScreenDirection::Up,
        band: None,
        cursor: None,
        limit: Some(1),
    })?;
    assert_eq!(first.total, 2);
    assert_eq!(first.items.len(), 1);
    assert_eq!(first.items[0].ticker, "T0001");
    assert_eq!(first.items[0].return_micros, 200_000);
    let second = core.list_stock_screen_results(ListStockScreenResultsInput {
        run_id: completed.id.clone(),
        horizon: StockScreenHorizon::Five,
        direction: StockScreenDirection::Up,
        band: Some(StockScreenBandFilter::TenToTwenty),
        cursor: None,
        limit: Some(50),
    })?;
    assert_eq!(second.total, 1);
    assert_eq!(second.items[0].ticker, "T0000");
    assert_eq!(second.items[0].return_micros, 100_000);

    let ai_id = format!("stock-ai-{}", completed.id);
    let report = core.begin_stock_ai_report(StockAiReportStart {
        id: &ai_id,
        screen_run_id: &completed.id,
        prompt_version: "stock-daily-v1",
        model: "gpt-5.4-nano-2026-03-17",
    })?;
    let replayed_report = core.begin_stock_ai_report(StockAiReportStart {
        id: &ai_id,
        screen_run_id: &completed.id,
        prompt_version: "stock-daily-v1",
        model: "gpt-5.4-nano-2026-03-17",
    })?;
    assert_eq!(report.id, replayed_report.id);
    assert!(report.request_started_at.is_none());
    assert!(core.try_mark_stock_ai_report_calling(&ai_id)?);
    assert!(!core.try_mark_stock_ai_report_calling(&ai_id)?);
    let calling = core
        .get_stock_ai_report(&ai_id)?
        .expect("calling AI report");
    assert!(calling.request_started_at.is_some());
    assert_eq!(
        core.mark_stock_ai_report_calling(&ai_id)?
            .request_started_at,
        calling.request_started_at
    );
    assert_eq!(
        core.get_stock_ai_report(&ai_id)?
            .expect("persisted AI report")
            .id,
        ai_id
    );
    Ok(())
}

fn coverage_run(present: usize) -> Result<String> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 1_000)?;
    let dates = session_dates();
    let mut bars = Vec::new();
    for (index, session_date) in dates.iter().enumerate() {
        let count = if matches!(index, 0 | 16 | 21) {
            present
        } else {
            1
        };
        for ticker_index in 0..count {
            bars.push(StockDailyBarInput {
                ticker: format!("T{ticker_index:04}"),
                session_date: *session_date,
                close_microusd: 100_000_000,
            });
        }
    }
    core.store_stock_market_data(StoreStockMarketDataInput {
        source: "synthetic".to_owned(),
        feed: "test".to_owned(),
        adjustment: "split".to_owned(),
        source_sha256: MARKET_SHA.to_owned(),
        sessions: dates,
        bars,
    })?;
    let started = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: None,
        market_date: market_date(),
        universe_snapshot_id: Some(universe.id.clone()),
    })?;
    Ok(core
        .complete_stock_screen_run(&started.id, &universe.id, MARKET_SHA)?
        .status)
}

#[test]
fn coverage_gate_accepts_98_percent_and_rejects_97_point_9() -> Result<()> {
    assert_eq!(coverage_run(980)?, "no_candidates");
    assert_eq!(coverage_run(979)?, "coverage_failed");
    Ok(())
}

#[test]
fn screen_completion_requires_a_persisted_market_data_batch() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 10)?;
    let run = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: Some("missing-market-batch".to_owned()),
        market_date: market_date(),
        universe_snapshot_id: Some(universe.id.clone()),
    })?;
    let missing_hash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
    assert!(matches!(
        core.complete_stock_screen_run(&run.id, &universe.id, missing_hash),
        Err(Error::NotFound {
            entity: "stock market data batch",
            ..
        })
    ));
    assert_eq!(
        core.get_stock_screen_run(&run.id)?
            .expect("screen run remains recoverable")
            .status,
        "started"
    );
    Ok(())
}

#[test]
fn rejects_all_duplicate_bars_and_duplicate_sessions() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let date = market_date();
    let base = StockDailyBarInput {
        ticker: "AAPL".to_owned(),
        session_date: date,
        close_microusd: 200_000_000,
    };
    assert!(matches!(
        core.store_stock_market_data(StoreStockMarketDataInput {
            source: "synthetic".to_owned(),
            feed: "test".to_owned(),
            adjustment: "split".to_owned(),
            source_sha256: MARKET_SHA.to_owned(),
            sessions: vec![date],
            bars: vec![base.clone(), base.clone()],
        }),
        Err(Error::InvalidInput(_))
    ));

    let mut conflicting = base;
    conflicting.close_microusd += 1;
    assert!(matches!(
        core.store_stock_market_data(StoreStockMarketDataInput {
            source: "synthetic".to_owned(),
            feed: "test".to_owned(),
            adjustment: "split".to_owned(),
            source_sha256: MARKET_SHA.to_owned(),
            sessions: vec![date],
            bars: vec![
                StockDailyBarInput {
                    ticker: "AAPL".to_owned(),
                    session_date: date,
                    close_microusd: 200_000_000,
                },
                conflicting,
            ],
        }),
        Err(Error::InvalidInput(_))
    ));
    assert!(matches!(
        core.store_stock_market_data(StoreStockMarketDataInput {
            source: "synthetic".to_owned(),
            feed: "test".to_owned(),
            adjustment: "split".to_owned(),
            source_sha256: MARKET_SHA.to_owned(),
            sessions: vec![date, date],
            bars: Vec::new(),
        }),
        Err(Error::InvalidInput(_))
    ));

    let idempotent = StoreStockMarketDataInput {
        source: "synthetic".to_owned(),
        feed: "test".to_owned(),
        adjustment: "split".to_owned(),
        source_sha256: MARKET_SHA.to_owned(),
        sessions: vec![date],
        bars: vec![StockDailyBarInput {
            ticker: "AAPL".to_owned(),
            session_date: date,
            close_microusd: 200_000_000,
        }],
    };
    let first = core.store_stock_market_data(idempotent.clone())?;
    let replayed = core.store_stock_market_data(idempotent.clone())?;
    assert_eq!(replayed, first);
    let mut conflicting_metadata = idempotent;
    conflicting_metadata.source = "different-source".to_owned();
    assert!(matches!(
        core.store_stock_market_data(conflicting_metadata),
        Err(Error::Conflict(_))
    ));
    let manifest = core.migration_manifest()?;
    assert_eq!(manifest.tables["stock_market_data_batches"].row_count, 1);
    assert_eq!(manifest.tables["stock_daily_bars"].row_count, 1);
    Ok(())
}

#[test]
fn schema_twelve_migration_preserves_scheduler_rows_and_foreign_keys() -> Result<()> {
    let (temporary, database_path) = schema_twelve_fixture("tm-schema12-stock-screen-")?;
    let connection = Connection::open(&database_path)?;
    connection.execute_batch(
        "INSERT INTO scheduler_jobs(
            id, job_key, kind, schedule_type, interval_seconds, local_time,
            timezone, enabled, max_attempts, misfire_grace_seconds, coalesce,
            next_run_at, last_scheduled_at, created_at, updated_at
         ) VALUES (
            'migration-canary-job', 'scheduler.canary', 'scheduler.canary',
            'interval', 300, NULL, 'UTC', 1, 5, 60, 1,
            '2026-06-01T00:00:00.000Z', NULL,
            '2026-06-01T00:00:00.000Z', '2026-06-01T00:00:00.000Z'
         );
         INSERT INTO scheduler_runs(
            id, job_id, scheduled_for, status, attempt_count, max_attempts,
            idempotency_key, available_at, lease_owner, lease_acquired_at,
            lease_expires_at, last_error, result_json, created_at, updated_at,
            started_at, completed_at, dead_letter_at, skipped_at
         )
         SELECT 'migration-running', id, '2026-06-01T00:00:00.000Z', 'running',
                1, 5, 'migration-running-key', '2026-06-01T00:00:00.000Z',
                'migration-worker', '2026-06-01T00:00:00.000Z',
                '2026-06-01T00:05:00.000Z', NULL, NULL,
                '2026-06-01T00:00:00.000Z', '2026-06-01T00:00:00.000Z',
                '2026-06-01T00:00:00.000Z', NULL, NULL, NULL
         FROM scheduler_jobs WHERE job_key = 'scheduler.canary';
         INSERT INTO scheduler_runs(
            id, job_id, scheduled_for, status, attempt_count, max_attempts,
            idempotency_key, available_at, lease_owner, lease_acquired_at,
            lease_expires_at, last_error, result_json, created_at, updated_at,
            started_at, completed_at, dead_letter_at, skipped_at
         )
         SELECT 'migration-retry', id, '2026-06-01T01:00:00.000Z', 'retry_wait',
                1, 5, 'migration-retry-key', '2026-06-01T01:01:00.000Z',
                NULL, NULL, NULL, 'retry', NULL,
                '2026-06-01T01:00:00.000Z', '2026-06-01T01:00:00.000Z',
                '2026-06-01T01:00:00.000Z', NULL, NULL, NULL
         FROM scheduler_jobs WHERE job_key = 'scheduler.canary';
         INSERT INTO scheduler_runs(
            id, job_id, scheduled_for, status, attempt_count, max_attempts,
            idempotency_key, available_at, lease_owner, lease_acquired_at,
            lease_expires_at, last_error, result_json, created_at, updated_at,
            started_at, completed_at, dead_letter_at, skipped_at
         )
         SELECT 'migration-dead', id, '2026-06-01T02:00:00.000Z', 'dead_letter',
                5, 5, 'migration-dead-key', '2026-06-01T02:00:00.000Z',
                NULL, NULL, NULL, 'dead', NULL,
                '2026-06-01T02:00:00.000Z', '2026-06-01T02:00:00.000Z',
                '2026-06-01T02:00:00.000Z', '2026-06-01T02:05:00.000Z',
                '2026-06-01T02:05:00.000Z', NULL
         FROM scheduler_jobs WHERE job_key = 'scheduler.canary';",
    )?;
    let before: i64 =
        connection.query_row("SELECT count(*) FROM scheduler_jobs", [], |row| row.get(0))?;
    let before_runs: i64 =
        connection.query_row("SELECT count(*) FROM scheduler_runs", [], |row| row.get(0))?;
    drop(connection);

    let migrated = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(migrated.health()?.schema_version, 16);
    let connection = Connection::open(migrated.home().database_path())?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    let after: i64 =
        connection.query_row("SELECT count(*) FROM scheduler_jobs", [], |row| row.get(0))?;
    let after_runs: i64 =
        connection.query_row("SELECT count(*) FROM scheduler_runs", [], |row| row.get(0))?;
    let foreign_key_errors: i64 =
        connection.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    assert_eq!(before, after);
    assert_eq!(before_runs, after_runs);
    assert_eq!(foreign_key_errors, 0);
    for status in ["running", "retry_wait", "dead_letter"] {
        let count: i64 = connection.query_row(
            "SELECT count(*) FROM scheduler_runs
             WHERE id LIKE 'migration-%' AND status = ?1",
            [status],
            |row| row.get(0),
        )?;
        assert_eq!(count, 1, "migration did not preserve {status}");
    }
    assert!(
        migrated
            .list_backups()?
            .iter()
            .any(|backup| backup.trigger == "pre_migration")
    );
    Ok(())
}

#[test]
fn stock_tables_are_in_export_and_manifest() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 2)?;
    assert_eq!(core.stock_universe_members(&universe.id)?.len(), 2);
    let manifest = core.migration_manifest()?;
    assert_eq!(manifest.tables["stock_universe_snapshots"].row_count, 1);
    assert_eq!(manifest.tables["stock_universe_members"].row_count, 2);
    assert!(manifest.tables.contains_key("stock_market_data_batches"));
    assert!(manifest.tables.contains_key("stock_screen_runs"));
    assert!(manifest.tables.contains_key("stock_ai_reports"));

    let export = core.create_export()?;
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(export.json_path)?)?;
    assert_eq!(
        document["tables"]["stock_universe_snapshots"][0]["source_sha256"],
        UNIVERSE_SHA
    );
    assert_eq!(
        document["tables"]["stock_universe_snapshots"][0]["attribution_text"],
        "Synthetic fixture derived from public-domain data."
    );
    Ok(())
}

#[test]
fn identical_universe_hash_creates_a_fresh_snapshot_for_a_new_effective_date() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let first = store_universe(&core, 2)?;
    let retry = store_universe(&core, 2)?;
    assert_eq!(retry.id, first.id);
    let next = core.store_stock_universe(StoreStockUniverseInput {
        name: "S&P 500 constituents (synthetic test)".to_owned(),
        effective_date: market_date() + Duration::days(1),
        source_url: "https://example.test/constituents.csv".to_owned(),
        source_revision: "fixture-next-date".to_owned(),
        source_sha256: UNIVERSE_SHA.to_owned(),
        license_name: "PDDL-1.0".to_owned(),
        license_url: "https://example.test/license".to_owned(),
        attribution_text: "Synthetic fixture derived from public-domain data.".to_owned(),
        members: (0..2)
            .map(|index| StockUniverseMemberInput {
                ticker: format!("T{index:04}"),
                display_name: format!("Synthetic company {index}"),
                sector: Some("Synthetic".to_owned()),
                sub_industry: None,
            })
            .collect(),
    })?;
    assert_ne!(next.id, first.id);
    assert!(next.fetched_at >= first.fetched_at);
    Ok(())
}

#[test]
fn a_new_fetch_replaces_old_session_bars_and_preserves_spy_only_sessions() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 10)?;
    store_full_prices(&core, 10)?;
    let original_run = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: Some("original-market-batch".to_owned()),
        market_date: market_date(),
        universe_snapshot_id: Some(universe.id.clone()),
    })?;
    let original_completed =
        core.complete_stock_screen_run(&original_run.id, &universe.id, MARKET_SHA)?;
    assert_eq!(original_completed.status, "succeeded");

    let dates = session_dates();
    let missing_entire_session = dates[16];
    let replacement_sha = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let mut replacement = Vec::new();
    for session_date in &dates {
        if *session_date == missing_entire_session {
            continue;
        }
        for ticker_index in 1..10 {
            replacement.push(StockDailyBarInput {
                ticker: format!("T{ticker_index:04}"),
                session_date: *session_date,
                close_microusd: 100_000_000,
            });
        }
    }
    core.store_stock_market_data(StoreStockMarketDataInput {
        source: "synthetic".to_owned(),
        feed: "test".to_owned(),
        adjustment: "split".to_owned(),
        source_sha256: replacement_sha.to_owned(),
        sessions: dates,
        bars: replacement,
    })?;
    let run = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: Some("replace-missing-bars".to_owned()),
        market_date: market_date(),
        universe_snapshot_id: Some(universe.id.clone()),
    })?;
    let completed = core.complete_stock_screen_run(&run.id, &universe.id, replacement_sha)?;
    assert_eq!(completed.status, "coverage_failed");
    assert_eq!(completed.current_covered, 9);
    assert_eq!(completed.baseline_5_covered, 0);
    assert_eq!(completed.baseline_21_covered, 9);
    assert_eq!(
        completed.failure_code.as_deref(),
        Some("coverage_below_98_percent")
    );
    assert_eq!(
        core.get_stock_screen_run(&original_run.id)?
            .expect("original screen run")
            .market_data_sha256
            .as_deref(),
        Some(MARKET_SHA)
    );
    let manifest = core.migration_manifest()?;
    assert_eq!(manifest.tables["stock_market_data_batches"].row_count, 2);
    let export = core.create_export()?;
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(export.json_path)?)?;
    let batches = document["tables"]["stock_market_data_batches"]
        .as_array()
        .expect("market batch export");
    assert!(batches.iter().any(|batch| {
        batch["source_sha256"] == MARKET_SHA
            && batch["session_count"] == 22
            && batch["bar_count"] == 220
    }));
    assert!(batches.iter().any(|batch| {
        batch["source_sha256"] == replacement_sha
            && batch["session_count"] == 22
            && batch["bar_count"] == 189
    }));
    Ok(())
}

#[test]
fn restore_cannot_rewind_issued_stock_results() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let universe = store_universe(&core, 10)?;
    let before_market_batch = core.create_backup()?;
    store_full_prices(&core, 10)?;
    assert!(matches!(
        core.restore_backup(&before_market_batch.path),
        Err(Error::Conflict(_))
    ));
    let before_result = core.create_backup()?;
    let run = core.begin_stock_screen_run(StockScreenStartInput {
        run_id: Some("anti-rewind-stock-run".to_owned()),
        market_date: market_date(),
        universe_snapshot_id: None,
    })?;
    core.complete_stock_screen_run(&run.id, &universe.id, MARKET_SHA)?;
    assert!(matches!(
        core.restore_backup(&before_result.path),
        Err(Error::Conflict(_))
    ));

    let matching = core.create_backup()?;
    core.restore_backup(&matching.path)?;
    assert_eq!(
        core.get_stock_screen_run(&run.id)?
            .expect("restored stock run")
            .status,
        "succeeded"
    );
    Ok(())
}
