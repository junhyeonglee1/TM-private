use std::path::PathBuf;

use rusqlite::{Connection, functions::FunctionFlags, params};
use tempfile::{Builder, TempDir};
use tm_core::{
    DEFAULT_TM_HOME, Error, Result, STOCK_WATCHLIST_LIMIT, StockMarket, TmCore, TmHome,
    UpsertStockWatchlistItemInput,
};
use uuid::Uuid;

const SCHEMA_ELEVEN_MIGRATIONS: [&str; 11] = [
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
];

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-stock-watchlist-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn schema_eleven_fixture(prefix: &str) -> Result<(TempDir, PathBuf)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new().prefix(prefix).tempdir_in(test_runs)?;
    let database_path = temporary.path().join("data").join("tm.sqlite3");
    std::fs::create_dir_all(
        database_path.parent().ok_or_else(|| {
            Error::Invariant("schema 11 fixture database has no parent".to_owned())
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
    for (index, migration) in SCHEMA_ELEVEN_MIGRATIONS.iter().enumerate() {
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

fn input(
    market: StockMarket,
    ticker: impl Into<String>,
    display_name: impl Into<String>,
) -> UpsertStockWatchlistItemInput {
    UpsertStockWatchlistItemInput {
        market,
        ticker: ticker.into(),
        display_name: display_name.into(),
    }
}

#[test]
fn normalizes_symbols_updates_duplicates_and_deletes_idempotently() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let created =
        core.upsert_stock_watchlist_item(input(StockMarket::Nasdaq, " aapl ", " Apple "))?;
    assert_eq!(created.symbol, "NASDAQ:AAPL");
    assert_eq!(created.ticker, "AAPL");
    assert_eq!(created.display_name, "Apple");

    let updated =
        core.upsert_stock_watchlist_item(input(StockMarket::Nasdaq, "AAPL", "Apple Inc."))?;
    assert_eq!(updated.symbol, created.symbol);
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(updated.display_name, "Apple Inc.");
    assert_eq!(core.stock_watchlist()?.len(), 1);

    core.delete_stock_watchlist_item(" nasdaq:aapl ")?;
    core.delete_stock_watchlist_item("NASDAQ:AAPL")?;
    assert!(core.stock_watchlist()?.is_empty());
    Ok(())
}

#[test]
fn rejects_unsupported_markets_tickers_and_names() -> Result<()> {
    let (_temporary, core) = fixture()?;
    for invalid in [
        input(StockMarket::Krx, "5930", "Samsung"),
        input(StockMarket::Krx, "00593A", "Samsung"),
        input(StockMarket::Nyse, "", "Empty"),
        input(StockMarket::Nyse, "TOO-LONG-123", "Long"),
        input(StockMarket::Amex, "BAD$", "Bad"),
        input(StockMarket::Nasdaq, "AAPL", "   "),
        input(StockMarket::Nasdaq, "AAPL", "x".repeat(81)),
    ] {
        assert!(matches!(
            core.upsert_stock_watchlist_item(invalid),
            Err(Error::InvalidInput(_))
        ));
    }
    assert!(matches!(
        core.delete_stock_watchlist_item("OTC:ABC"),
        Err(Error::InvalidInput(_))
    ));
    assert!(core.stock_watchlist()?.is_empty());
    Ok(())
}

#[test]
fn enforces_fifty_item_limit_without_blocking_existing_updates() -> Result<()> {
    let (_temporary, core) = fixture()?;
    for index in 0..STOCK_WATCHLIST_LIMIT {
        core.upsert_stock_watchlist_item(input(
            StockMarket::Nasdaq,
            format!("T{index:02}"),
            format!("Ticker {index}"),
        ))?;
    }
    assert_eq!(core.stock_watchlist()?.len(), STOCK_WATCHLIST_LIMIT);
    assert!(matches!(
        core.upsert_stock_watchlist_item(input(StockMarket::Nyse, "EXTRA", "Fifty-first item")),
        Err(Error::InvalidInput(_))
    ));

    let updated =
        core.upsert_stock_watchlist_item(input(StockMarket::Nasdaq, "T00", "Updated first item"))?;
    assert_eq!(updated.display_name, "Updated first item");
    assert_eq!(core.stock_watchlist()?.len(), STOCK_WATCHLIST_LIMIT);
    Ok(())
}

#[test]
fn migrates_schema_eleven_with_a_pre_migration_backup() -> Result<()> {
    let (temporary, _database_path) = schema_eleven_fixture("tm-schema11-stock-watchlist-")?;

    let migrated = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(migrated.health()?.schema_version, 17);
    assert!(migrated.stock_watchlist()?.is_empty());
    assert!(
        migrated
            .list_backups()?
            .iter()
            .any(|backup| backup.trigger == "pre_migration")
    );
    Ok(())
}

#[test]
fn watchlist_is_included_in_export_and_migration_manifest() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.upsert_stock_watchlist_item(input(StockMarket::Krx, "005930", "삼성전자"))?;

    let manifest = core.migration_manifest()?;
    assert_eq!(manifest.tables["stock_watchlist_items"].row_count, 1);

    let export = core.create_export()?;
    let exported = std::fs::read_to_string(export.json_path)?;
    let document: serde_json::Value = serde_json::from_str(&exported)?;
    assert_eq!(
        document["tables"]["stock_watchlist_items"][0]["symbol"],
        "KRX:005930"
    );
    Ok(())
}

#[test]
fn backup_restore_preserves_watchlist_items() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.upsert_stock_watchlist_item(input(StockMarket::Nyse, "BRK.B", "Berkshire Hathaway"))?;
    let backup = core.create_backup()?;
    core.delete_stock_watchlist_item("NYSE:BRK.B")?;
    assert!(core.stock_watchlist()?.is_empty());

    core.restore_backup(&backup.path)?;
    assert_eq!(core.stock_watchlist()?[0].symbol, "NYSE:BRK.B");
    Ok(())
}
