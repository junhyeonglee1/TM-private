use std::{
    fs::{File, OpenOptions},
    ops::{Deref, DerefMut},
    path::Path,
    time::Duration,
};

use chrono::{DateTime, LocalResult, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Asia::Seoul;
use fs2::FileExt;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
    functions::FunctionFlags, params,
};
use uuid::Uuid;

use crate::{Error, Result, TmHome};

pub(crate) const SCHEMA_VERSION: i64 = 14;
const INITIAL_MIGRATION: &str = include_str!("../migrations/0001_initial.sql");
const CHANGE_REQUESTS_MIGRATION: &str = include_str!("../migrations/0002_change_requests.sql");
const CHANGE_REQUESTS_STRICT_CAS_MIGRATION: &str =
    include_str!("../migrations/0003_change_request_strict_cas.sql");
const CONTROLLED_MUTATIONS_MIGRATION: &str =
    include_str!("../migrations/0004_controlled_mutations.sql");
const AI_BUDGET_GUARD_MIGRATION: &str = include_str!("../migrations/0005_ai_budget_guard.sql");
const ASSISTANT_ACTION_APPROVALS_MIGRATION: &str =
    include_str!("../migrations/0006_assistant_action_approvals.sql");
const ASSISTANT_MEMORY_MIGRATION: &str = include_str!("../migrations/0007_assistant_memory.sql");
const DURABLE_SCHEDULER_MIGRATION: &str = include_str!("../migrations/0008_durable_scheduler.sql");
const DEVICE_AUTH_MIGRATION: &str = include_str!("../migrations/0009_device_auth.sql");
const TASK_REPORTS_MIGRATION: &str = include_str!("../migrations/0010_task_reports.sql");
const CALENDAR_EVENTS_MIGRATION: &str = include_str!("../migrations/0011_calendar_events.sql");
const STOCK_WATCHLIST_MIGRATION: &str = include_str!("../migrations/0012_stock_watchlist.sql");
const STOCK_DAILY_SCREEN_MIGRATION: &str =
    include_str!("../migrations/0013_stock_daily_screen.sql");
const UNCATEGORIZED_PROJECT_MIGRATION: &str =
    include_str!("../migrations/0014_uncategorized_project.sql");
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

        let final_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        validate_schema_semantics(&connection, final_version)?;

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
        if current_version < 7 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(ASSISTANT_MEMORY_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (7, 'assistant-memory-and-context-budget', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 7_i64)?;
            transaction.commit()?;
        }
        if current_version < 8 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(DURABLE_SCHEDULER_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (8, 'durable-scheduler-and-job-queue', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 8_i64)?;
            transaction.commit()?;
        }
        if current_version < 9 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(DEVICE_AUTH_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (9, 'mobile-device-pairing-and-auth', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 9_i64)?;
            transaction.commit()?;
        }
        if current_version < 10 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(TASK_REPORTS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (10, 'today-task-ai-reports', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 10_i64)?;
            transaction.commit()?;
        }
        if current_version < 11 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(CALENDAR_EVENTS_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (11, 'personal-calendar-and-monthly-recurrence', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 11_i64)?;
            transaction.commit()?;
        }
        if current_version < 12 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(STOCK_WATCHLIST_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (12, 'read-only-stock-watchlist', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 12_i64)?;
            transaction.commit()?;
        }
        if current_version < 13 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(STOCK_DAILY_SCREEN_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (13, 'sp500-daily-stock-screen', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 13_i64)?;
            transaction.commit()?;
        }
        if current_version < 14 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(UNCATEGORIZED_PROJECT_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (14, 'uncategorized-system-project', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 14_i64)?;
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

pub(crate) fn validate_schema_semantics(connection: &Connection, version: i64) -> Result<()> {
    if version < 14 {
        return Ok(());
    }
    if !(14..=SCHEMA_VERSION).contains(&version) {
        return Err(Error::Invariant(format!(
            "schema semantic validation does not support version {version}"
        )));
    }

    let (system_count, valid_system_count, invalid_system_key_count): (i64, i64, i64) = connection
        .query_row(
            "SELECT
                coalesce(sum(CASE WHEN system_key = 'uncategorized' THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE
                    WHEN system_key = 'uncategorized'
                     AND name = '기타'
                     AND archived_at IS NULL
                     AND deleted_at IS NULL
                    THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE
                    WHEN system_key IS NOT NULL AND system_key <> 'uncategorized'
                    THEN 1 ELSE 0 END), 0)
             FROM projects",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if system_count != 1 || valid_system_count != 1 || invalid_system_key_count != 0 {
        return Err(Error::Invariant(format!(
            "schema 14 requires exactly one active project named 기타 with system_key uncategorized; found system={system_count}, valid={valid_system_count}, invalid_keys={invalid_system_key_count}"
        )));
    }

    let invalid_task_projects: i64 = connection.query_row(
        "SELECT count(*)
         FROM tasks
         LEFT JOIN projects ON projects.id = tasks.project_id
         WHERE tasks.project_id IS NULL OR projects.id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if invalid_task_projects != 0 {
        return Err(Error::Invariant(format!(
            "schema 14 requires every task to reference a project; found {invalid_task_projects} missing project references"
        )));
    }

    require_schema_object(
        connection,
        "table",
        "projects",
        &[
            "system_keytext",
            "check(system_keyisnullorsystem_key='uncategorized')",
        ],
    )?;
    require_schema_object(
        connection,
        "index",
        "idx_projects_system_key",
        &[
            "createuniqueindexidx_projects_system_key",
            "onprojects(system_key)",
            "wheresystem_keyisnotnull",
        ],
    )?;

    for (name, fragments) in [
        (
            "tasks_project_required_insert",
            &[
                "beforeinsertontasks",
                "new.project_idisnull",
                "raise(abort,'taskprojectisrequired')",
            ][..],
        ),
        (
            "tasks_project_required_update",
            &[
                "beforeupdateofproject_idontasks",
                "new.project_idisnull",
                "raise(abort,'taskprojectisrequired')",
            ][..],
        ),
        (
            "projects_uncategorized_protect_update",
            &[
                "beforeupdateonprojects",
                "old.system_key='uncategorized'",
                "new.system_keyisnotold.system_key",
                "new.nameisnotold.name",
                "new.archived_atisnotold.archived_at",
                "new.deleted_atisnotold.deleted_at",
                "raise(abort,'uncategorizedprojectidentityandactivestateareimmutable')",
            ][..],
        ),
        (
            "projects_uncategorized_protect_delete",
            &[
                "beforedeleteonprojects",
                "old.system_key='uncategorized'",
                "raise(abort,'uncategorizedprojectcannotbedeleted')",
            ][..],
        ),
        (
            "projects_uncategorized_name_reserved_insert",
            &[
                "beforeinsertonprojects",
                "new.system_keyisnull",
                "trim(new.name)='기타'",
                "raise(abort,'projectname기타isreserved')",
            ][..],
        ),
        (
            "projects_uncategorized_name_reserved_update",
            &[
                "beforeupdateofname,system_keyonprojects",
                "new.system_keyisnull",
                "trim(new.name)='기타'",
                "trim(old.name)<>'기타'",
                "raise(abort,'projectname기타isreserved')",
            ][..],
        ),
    ] {
        require_schema_object(connection, "trigger", name, fragments)?;
    }

    Ok(())
}

fn require_schema_object(
    connection: &Connection,
    object_type: &str,
    name: &str,
    required_fragments: &[&str],
) -> Result<()> {
    let sql = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = ?1 AND name = ?2",
            params![object_type, name],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .ok_or_else(|| {
            Error::Invariant(format!(
                "schema 14 required {object_type} is missing: {name}"
            ))
        })?;
    let normalized = sql
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some(fragment) = required_fragments
        .iter()
        .find(|fragment| !normalized.contains(*fragment))
    {
        return Err(Error::Invariant(format!(
            "schema 14 {object_type} {name} is missing required SQL fragment: {fragment}"
        )));
    }
    Ok(())
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
