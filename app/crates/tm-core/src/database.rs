use std::{
    fs::{File, OpenOptions},
    ops::{Deref, DerefMut},
    path::Path,
    time::Duration,
};

use chrono::{DateTime, LocalResult, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Asia::Seoul;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, functions::FunctionFlags};
use uuid::Uuid;

use crate::{Error, Result, TmHome};

pub(crate) const SCHEMA_VERSION: i64 = 6;
const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");
const CHANGE_REQUESTS_MIGRATION: &str = include_str!("../migrations/0002_change_requests.sql");
const CHANGE_REQUESTS_STRICT_CAS_MIGRATION: &str =
    include_str!("../migrations/0003_change_request_strict_cas.sql");
const CONTROLLED_MUTATIONS_MIGRATION: &str =
    include_str!("../migrations/0004_controlled_mutations.sql");
const AI_BUDGET_GUARD_MIGRATION: &str = include_str!("../migrations/0005_ai_budget_guard.sql");
const ASSISTANT_ACTION_APPROVALS_MIGRATION: &str =
    include_str!("../migrations/0006_assistant_action_approvals.sql");
const BUSY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub(crate) struct Database {
    home: TmHome,
}

pub(crate) struct ManagedConnection {
    connection: Connection,
    _maintenance_lock: File,
}

impl Deref for ManagedConnection {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl DerefMut for ManagedConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

impl Database {
    pub(crate) fn open(home: TmHome) -> Result<Self> {
        home.ensure_layout()?;
        let database_path = home.database_path();
        let maintenance_lock = database_lock(&database_path)?;
        FileExt::lock_exclusive(&maintenance_lock)?;
        let existed_with_content = database_path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
        let mut connection = Self::connect_path(&database_path)?;
        let current_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > SCHEMA_VERSION {
            return Err(Error::Invariant(format!(
                "database schema {current_version} is newer than supported schema {SCHEMA_VERSION}"
            )));
        }

        if current_version < SCHEMA_VERSION {
            if existed_with_content {
                crate::backup::online_backup_connection(
                    &connection,
                    &home.database_backups_dir(),
                    "pre-migration",
                )?;
            }
            Self::migrate(&mut connection, current_version)?;
        }

        Ok(Self { home })
    }

    fn connect_path(path: &Path) -> Result<Connection> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )?;
        connection.busy_timeout(BUSY_TIMEOUT)?;
        register_runtime_functions(&connection)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA wal_autocheckpoint = 1000;",
        )?;
        Ok(connection)
    }

    pub(crate) fn migrate(connection: &mut Connection, current_version: i64) -> Result<()> {
        if current_version < 1 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(INITIAL_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (1, 'local-first-foundation', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 1_i64)?;
            transaction.commit()?;
        }
        if current_version < 2 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CHANGE_REQUESTS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (2, 'manual-change-request-queue', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 2_i64)?;
            transaction.commit()?;
        }
        if current_version < 3 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CHANGE_REQUESTS_STRICT_CAS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (3, 'change-request-strict-cas', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 3_i64)?;
            transaction.commit()?;
        }
        if current_version < 4 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CONTROLLED_MUTATIONS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (4, 'controlled-mutations', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 4_i64)?;
            transaction.commit()?;
        }
        if current_version < 5 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(AI_BUDGET_GUARD_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (5, 'ai-budget-guard', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 5_i64)?;
            transaction.commit()?;
        }
        if current_version < 6 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(ASSISTANT_ACTION_APPROVALS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (6, 'assistant-action-approvals', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 6_i64)?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub(crate) fn connect(&self) -> Result<ManagedConnection> {
        let path = self.home.database_path();
        let maintenance_lock = database_lock(&path)?;
        FileExt::lock_shared(&maintenance_lock)?;
        let connection = Self::connect_path(&path)?;
        Ok(ManagedConnection {
            connection,
            _maintenance_lock: maintenance_lock,
        })
    }

    pub(crate) fn transaction<T>(
        &self,
        behavior: TransactionBehavior,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(behavior)?;
        let output = operation(&transaction)?;
        transaction.commit()?;
        Ok(output)
    }

    pub(crate) fn checkpoint(&self) -> Result<()> {
        self.connect()?
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(())
    }

    pub(crate) fn home(&self) -> &TmHome {
        &self.home
    }
}

pub(crate) fn register_runtime_functions(connection: &Connection) -> Result<()> {
    connection.create_scalar_function("tm_uuid_v7", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(new_id())
    })?;
    connection.create_scalar_function("tm_now_utc", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(now_utc())
    })?;
    Ok(())
}

pub(crate) fn database_lock(database_path: &Path) -> Result<File> {
    let parent = database_path.parent().ok_or_else(|| {
        Error::Invariant(format!(
            "database path has no parent: {}",
            database_path.display()
        ))
    })?;
    std::fs::create_dir_all(parent)?;
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join(".tm-database.lock"))?)
}

#[must_use]
pub(crate) fn new_id() -> String {
    Uuid::now_v7().to_string()
}

#[must_use]
pub(crate) fn now_utc() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[must_use]
pub(crate) fn today_seoul() -> NaiveDate {
    Utc::now().with_timezone(&Seoul).date_naive()
}

pub(crate) fn seoul_day_bounds(date: NaiveDate) -> Result<(String, String)> {
    let start_local = date.and_time(NaiveTime::MIN);
    let next_date = date
        .succ_opt()
        .ok_or_else(|| Error::InvalidInput(format!("date has no successor: {date}")))?;
    let end_local = next_date.and_time(NaiveTime::MIN);

    let to_utc = |local| match Seoul.from_local_datetime(&local) {
        LocalResult::Single(value) => Ok(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(earlier, _) => Ok(earlier.with_timezone(&Utc)),
        LocalResult::None => Err(Error::Invariant(format!(
            "unable to resolve Seoul local time: {local}"
        ))),
    };
    let start: DateTime<Utc> = to_utc(start_local)?;
    let end: DateTime<Utc> = to_utc(end_local)?;
    Ok((
        start.to_rfc3339_opts(SecondsFormat::Millis, true),
        end.to_rfc3339_opts(SecondsFormat::Millis, true),
    ))
}
