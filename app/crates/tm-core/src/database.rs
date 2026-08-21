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

pub(crate) const SCHEMA_VERSION: i64 = 17;
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
const EXPENSE_REPORTING_MIGRATION: &str = include_str!("../migrations/0015_expense_reporting.sql");
const EXPENSE_AI_CLASSIFICATION_MIGRATION: &str =
    include_str!("../migrations/0016_expense_ai_classification.sql");
const MAIL_MONITORING_MIGRATION: &str = include_str!("../migrations/0017_mail_monitoring.sql");
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
        if current_version < 15 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(EXPENSE_REPORTING_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (15, 'expense-and-recurring-reporting', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 15_i64)?;
            transaction.commit()?;
        }
        if current_version < 16 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(EXPENSE_AI_CLASSIFICATION_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (16, 'expense-ai-hybrid-classification', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 16_i64)?;
            transaction.commit()?;
        }
        if current_version < 17 {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute_batch(MAIL_MONITORING_MIGRATION)?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (17, 'read-only-mail-monitoring', ?1)",
                [now_utc()],
            )?;
            transaction.pragma_update(None, "user_version", 17_i64)?;
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

    if version < 15 {
        return Ok(());
    }

    for table in [
        "expense_crypto_metadata",
        "expense_sources",
        "expense_import_batches",
        "expense_import_preview_sessions",
        "expense_raw_rows",
        "expense_postings",
        "expense_events",
        "expense_event_postings",
        "expense_allocations",
        "expense_reviews",
        "expense_rules",
        "recurring_expense_items",
        "recurring_expense_versions",
        "recurring_expense_occurrences",
        "expense_month_reports",
        "expense_ai_reports",
        "expense_ai_feedback",
        "expense_ai_request_bindings",
        "expense_ai_attempts",
        "expense_mutation_receipts",
    ] {
        let fragment = format!("createtable{table}");
        require_schema_object(connection, "table", table, &[fragment.as_str()])?;
    }
    require_schema_object(
        connection,
        "table",
        "expense_ai_reports",
        &[
            "cached_input_tokensintegernotnulldefault0",
            "total_tokensintegernotnulldefault0",
        ],
    )?;
    require_schema_object(
        connection,
        "table",
        "expense_ai_attempts",
        &[
            "report_month_starttextnotnull",
            "attempt_statustextnotnulldefault'claimed'",
            "result_jsontextcheck(result_jsonisnullorjson_valid(result_json))",
            "failure_codetext",
            "completed_attext",
        ],
    )?;
    require_schema_object(
        connection,
        "table",
        "expense_ai_request_bindings",
        &[
            "report_month_starttextnotnull",
            "aggregate_sha256textnotnull",
        ],
    )?;

    for (name, fragments) in [
        (
            "expense_raw_rows_immutable_update",
            &[
                "beforeupdateonexpense_raw_rows",
                "raise(abort,'expenserawrowsareimmutable')",
            ][..],
        ),
        (
            "expense_raw_rows_immutable_delete",
            &[
                "beforedeleteonexpense_raw_rows",
                "raise(abort,'expenserawrowsareimmutable')",
            ][..],
        ),
        (
            "expense_postings_immutable_update",
            &[
                "beforeupdateonexpense_postings",
                "raise(abort,'expensepostingsareimmutable')",
            ][..],
        ),
        (
            "expense_postings_immutable_delete",
            &[
                "beforedeleteonexpense_postings",
                "raise(abort,'expensepostingsareimmutable')",
            ][..],
        ),
        (
            "expense_event_postings_immutable_update",
            &[
                "beforeupdateonexpense_event_postings",
                "raise(abort,'expenseeventpostinglinksareimmutable')",
            ][..],
        ),
        (
            "expense_event_postings_immutable_delete",
            &[
                "beforedeleteonexpense_event_postings",
                "raise(abort,'expenseeventpostinglinksareimmutable')",
            ][..],
        ),
        (
            "recurring_expense_versions_immutable_update",
            &[
                "beforeupdateonrecurring_expense_versions",
                "raise(abort,'recurringexpenseversionsareimmutable')",
            ][..],
        ),
        (
            "recurring_expense_versions_immutable_delete",
            &[
                "beforedeleteonrecurring_expense_versions",
                "raise(abort,'recurringexpenseversionsareimmutable')",
            ][..],
        ),
    ] {
        require_schema_object(connection, "trigger", name, fragments)?;
    }

    for index in [
        "idx_expense_import_preview_expiry",
        "idx_expense_postings_merchant",
        "idx_expense_events_month",
        "idx_expense_reviews_queue",
        "idx_recurring_expense_versions_effective",
        "idx_recurring_occurrences_due",
        "idx_expense_ai_attempts_month",
    ] {
        require_schema_object(connection, "index", index, &["createindex"])?;
    }
    require_schema_object(
        connection,
        "index",
        "idx_expense_allocations_personal_unique",
        &[
            "createuniqueindex",
            "onexpense_allocations(event_id)",
            "whereallocation_kind='personal'",
        ],
    )?;
    require_schema_object(
        connection,
        "index",
        "idx_recurring_occurrences_actual_event_unique",
        &[
            "createuniqueindex",
            "onrecurring_expense_occurrences(actual_event_id)",
            "whereactual_event_idisnotnull",
        ],
    )?;
    require_schema_object(
        connection,
        "index",
        "idx_expense_allocations_settlement_event_unique",
        &[
            "createuniqueindex",
            "onexpense_allocations(event_id)",
            "whereallocation_kindin('settlement_received','settlement_sent')",
        ],
    )?;
    require_schema_object_exact(
        connection,
        "index",
        "idx_expense_rules_classification_unique",
        "createuniqueindexidx_expense_rules_classification_uniqueonexpense_rules(merchant_blind_index,coalesce(payment_method_fingerprint,''))whererule_kind='classification'",
    )?;
    require_schema_object_exact(
        connection,
        "index",
        "idx_expense_rules_recurring_match_unique",
        "createuniqueindexidx_expense_rules_recurring_match_uniqueonexpense_rules(recurring_expense_id,merchant_blind_index,coalesce(payment_method_fingerprint,''))whererule_kind='recurring_match'",
    )?;

    for (table, forbidden_columns) in [
        (
            "expense_postings",
            &["merchant", "counterparty", "memo"][..],
        ),
        ("recurring_expense_items", &["name", "vendor", "memo"][..]),
    ] {
        let pragma = format!("PRAGMA table_info(\"{table}\")");
        let mut statement = connection.prepare(&pragma)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(column) = forbidden_columns
            .iter()
            .find(|column| columns.iter().any(|candidate| candidate == **column))
        {
            return Err(Error::Invariant(format!(
                "schema 15 forbids plaintext expense column {table}.{column}"
            )));
        }
    }

    let invalid_crypto_metadata: i64 = connection.query_row(
        "SELECT count(*) FROM expense_crypto_metadata
         WHERE singleton_key <> 'expense-data-key-probe' OR key_version <> 1",
        [],
        |row| row.get(0),
    )?;
    if invalid_crypto_metadata != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 expense crypto metadata must use the singleton v1 probe; found {invalid_crypto_metadata} invalid rows"
        )));
    }

    let raw_rows_without_postings: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_raw_rows AS raw
         LEFT JOIN expense_postings AS posting ON posting.raw_row_id = raw.id
         WHERE posting.id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if raw_rows_without_postings != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 requires every expense raw row to have a posting; found {raw_rows_without_postings} orphan raw rows"
        )));
    }

    let inconsistent_posting_sources: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_postings AS posting
         LEFT JOIN expense_raw_rows AS raw ON raw.id = posting.raw_row_id
         WHERE raw.id IS NULL OR raw.source_id <> posting.source_id",
        [],
        |row| row.get(0),
    )?;
    if inconsistent_posting_sources != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 requires every expense posting to reference its raw row source; found {inconsistent_posting_sources} inconsistent postings"
        )));
    }

    let invalid_event_posting_links: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_event_postings AS link
         LEFT JOIN expense_events AS event ON event.id = link.event_id
         LEFT JOIN expense_postings AS posting ON posting.id = link.posting_id
         WHERE event.id IS NULL OR posting.id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if invalid_event_posting_links != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 expense event-posting links must reference both sides; found {invalid_event_posting_links} invalid links"
        )));
    }

    let unlinked_postings: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_postings AS posting
         LEFT JOIN expense_event_postings AS link ON link.posting_id = posting.id
         WHERE link.posting_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    if unlinked_postings != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 requires every expense posting to belong to an economic event; found {unlinked_postings} unlinked postings"
        )));
    }

    let events_without_primary_links: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_events AS event
         WHERE event.primary_posting_id IS NOT NULL
           AND NOT EXISTS(
               SELECT 1 FROM expense_event_postings AS link
               WHERE link.event_id = event.id
                 AND link.posting_id = event.primary_posting_id
                 AND link.posting_role = 'primary'
           )",
        [],
        |row| row.get(0),
    )?;
    if events_without_primary_links != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 requires each posted expense event to link its primary posting; found {events_without_primary_links} inconsistent events"
        )));
    }

    let inconsistent_primary_links: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_event_postings AS link
         JOIN expense_events AS event ON event.id = link.event_id
         WHERE link.posting_role = 'primary'
           AND event.primary_posting_id IS NOT link.posting_id",
        [],
        |row| row.get(0),
    )?;
    if inconsistent_primary_links != 0 {
        return Err(Error::Invariant(format!(
            "schema 15 primary expense links must agree with their event; found {inconsistent_primary_links} inconsistent links"
        )));
    }

    if version < 16 {
        return Ok(());
    }

    require_schema_object(
        connection,
        "table",
        "expense_events",
        &[
            "classification_sourcetextnotnulldefault'deterministic'",
            "classification_confidenceinteger",
            "check(classification_sourcein('deterministic','user_rule','manual','ai'))",
            "check(classification_confidenceisnullorclassification_confidencebetween0and100)",
        ],
    )?;
    require_schema_object(
        connection,
        "table",
        "expense_ai_classification_batches",
        &[
            "target_month_starttextnotnull",
            "attempt_numberintegernotnullcheck(attempt_numberbetween1and12)",
            "batch_statustextnotnulldefault'claimed'",
            "check(batch_statusin('claimed','staged','applied','failed','stale'))",
            "result_jsontextcheck(result_jsonisnullorjson_valid(result_json))",
            "item_group_countintegernotnull",
            "review_countintegernotnull",
            "privacy_skipped_countintegernotnulldefault0",
            "version_conflict_countintegernotnulldefault0",
            "cost_microusdintegernotnulldefault0",
            "failure_codetextcheck(failure_codeisnullorfailure_codein(",
        ],
    )?;
    require_schema_object(
        connection,
        "table",
        "expense_ai_classification_items",
        &[
            "batch_request_idtextnotnull",
            "event_idtextnotnull",
            "review_idtextnotnull",
            "expected_event_versionintegernotnull",
            "expected_review_versionintegernotnull",
            "confidenceinteger",
            "dispositiontext",
            "check(dispositionisnullordispositionin('confirmed','provisional','review_required','privacy_skipped'))",
        ],
    )?;
    require_schema_object(
        connection,
        "table",
        "expense_ai_classification_receipts",
        &[
            "request_idtextprimarykeynotnull",
            "target_month_starttextnotnull",
            "terminal_statustextnotnull",
            "check(terminal_status='no_candidates')",
            "result_jsontextnotnull",
            "check(result_json='{\"itemgroupcount\":0,\"reviewcount\":0,\"resultcount\":0,\"confirmedcount\":0,\"provisionalcount\":0,\"reviewrequiredcount\":0,\"privacyskippedcount\":0,\"versionconflictcount\":0,\"actualcostmicrousd\":0}')",
            "completed_attextnotnull",
        ],
    )?;
    for index in [
        "idx_expense_ai_classification_batches_quota",
        "idx_expense_ai_classification_batches_status",
        "idx_expense_ai_classification_items_item",
        "idx_expense_ai_classification_items_review",
    ] {
        require_schema_object(connection, "index", index, &["createindex"])?;
    }
    require_schema_object(
        connection,
        "index",
        "idx_expense_ai_classification_batches_active_input",
        &[
            "createuniqueindex",
            "onexpense_ai_classification_batches(input_sha256,prompt_version)",
            "wherebatch_statusin('claimed','staged','applied')",
        ],
    )?;
    require_schema_object(
        connection,
        "trigger",
        "expense_ai_classification_receipts_immutable_update",
        &["beforeupdateonexpense_ai_classification_receipts"],
    )?;
    require_schema_object(
        connection,
        "trigger",
        "expense_ai_classification_receipts_immutable_delete",
        &["beforedeleteonexpense_ai_classification_receipts"],
    )?;

    for table in [
        "expense_ai_classification_batches",
        "expense_ai_classification_items",
        "expense_ai_classification_receipts",
    ] {
        let pragma = format!("PRAGMA table_info(\"{table}\")");
        let mut statement = connection.prepare(&pragma)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(column) = [
            "merchant",
            "merchant_name",
            "merchant_blind_index",
            "payment_method_fingerprint",
            "counterparty",
            "memo",
            "prompt",
            "prompt_text",
        ]
        .iter()
        .find(|column| columns.iter().any(|candidate| candidate == **column))
        {
            return Err(Error::Invariant(format!(
                "schema 16 forbids duplicated sensitive expense classification column {table}.{column}"
            )));
        }
    }

    let overlapping_receipts: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_ai_classification_receipts AS receipt
         JOIN expense_ai_classification_batches AS batch
           ON batch.request_id = receipt.request_id",
        [],
        |row| row.get(0),
    )?;
    if overlapping_receipts != 0 {
        return Err(Error::Invariant(format!(
            "schema 16 classification request IDs cannot be both receipts and batches; found {overlapping_receipts} overlaps"
        )));
    }

    let invalid_no_candidate_receipts: i64 = connection.query_row(
        "SELECT count(*)
         FROM expense_ai_classification_receipts AS receipt
         WHERE receipt.result_json !=
            '{\"itemGroupCount\":0,\"reviewCount\":0,\"resultCount\":0,\"confirmedCount\":0,\"provisionalCount\":0,\"reviewRequiredCount\":0,\"privacySkippedCount\":0,\"versionConflictCount\":0,\"actualCostMicrousd\":0}'",
        [],
        |row| row.get(0),
    )?;
    if invalid_no_candidate_receipts != 0 {
        return Err(Error::Invariant(format!(
            "schema 16 no-candidate receipts must contain only the fixed zero-result shape; found {invalid_no_candidate_receipts} invalid rows"
        )));
    }

    if version < 17 {
        return Ok(());
    }

    for table in [
        "mail_crypto_metadata",
        "mail_accounts",
        "mail_credentials",
        "mail_sync_state",
        "mail_items",
        "mail_feedback",
        "mail_rules",
        "mail_oauth_states",
        "mail_webhook_events",
        "mail_triage_batches",
        "mail_triage_items",
        "mail_reports",
        "mail_report_items",
        "mail_sync_events",
        "mail_mutation_receipts",
    ] {
        let fragment = format!("createtable{table}");
        require_schema_object(connection, "table", table, &[fragment.as_str()])?;
    }
    require_schema_object(
        connection,
        "table",
        "scheduler_jobs",
        &[
            "'mail.gmail_watch'",
            "'mail.gmail_reconcile'",
            "'mail.naver_poll'",
            "'mail.triage'",
            "'mail.digest.morning'",
            "'mail.digest.evening'",
            "'mail.retention'",
        ],
    )?;
    for index in [
        "idx_mail_items_queue",
        "idx_mail_items_account",
        "idx_mail_items_expiry",
        "idx_mail_oauth_states_expiry",
        "idx_mail_triage_batches_quota",
        "idx_mail_reports_date",
        "idx_mail_sync_events_account",
    ] {
        require_schema_object(connection, "index", index, &["createindex"])?;
    }
    require_schema_object(
        connection,
        "trigger",
        "mail_triage_batches_identity_immutable",
        &["beforeupdateonmail_triage_batches", "old.status<>'claimed'"],
    )?;
    require_schema_object(
        connection,
        "trigger",
        "mail_triage_items_guarded_completion",
        &[
            "beforeupdateonmail_triage_items",
            "old.importance_scoreisnotnull",
            "batch.status='claimed'",
        ],
    )?;
    require_schema_object(
        connection,
        "trigger",
        "mail_mutation_receipts_no_update",
        &["beforeupdateonmail_mutation_receipts"],
    )?;
    require_schema_object(
        connection,
        "trigger",
        "mail_mutation_receipts_no_delete",
        &["beforedeleteonmail_mutation_receipts"],
    )?;
    for (table, forbidden_columns) in [
        ("mail_accounts", &["email", "display_name"][..]),
        (
            "mail_credentials",
            &["refresh_token", "app_password", "secret"][..],
        ),
        (
            "mail_items",
            &[
                "sender",
                "sender_domain",
                "subject",
                "summary",
                "body",
                "html",
            ][..],
        ),
        ("mail_reports", &["summary", "body", "html"][..]),
    ] {
        let pragma = format!("PRAGMA table_info(\"{table}\")");
        let mut statement = connection.prepare(&pragma)?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if let Some(column) = forbidden_columns
            .iter()
            .find(|column| columns.iter().any(|candidate| candidate == **column))
        {
            return Err(Error::Invariant(format!(
                "schema 17 forbids plaintext mail column {table}.{column}"
            )));
        }
    }
    let invalid_crypto_metadata: i64 = connection.query_row(
        "SELECT count(*) FROM mail_crypto_metadata
         WHERE singleton_key <> 'mail-data-key-probe' OR key_version <> 1",
        [],
        |row| row.get(0),
    )?;
    if invalid_crypto_metadata != 0 {
        return Err(Error::Invariant(format!(
            "schema 17 mail crypto metadata must use the singleton v1 probe; found {invalid_crypto_metadata} invalid rows"
        )));
    }
    let provider_mutations: i64 = connection.query_row(
        "SELECT count(*) FROM mail_sync_events WHERE changed_provider_state <> 0",
        [],
        |row| row.get(0),
    )?;
    if provider_mutations != 0 {
        return Err(Error::Invariant(format!(
            "schema 17 read-only mail sync forbids provider state changes; found {provider_mutations} invalid rows"
        )));
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
            Error::Invariant(format!("schema required {object_type} is missing: {name}"))
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
            "schema {object_type} {name} is missing required SQL fragment: {fragment}"
        )));
    }
    Ok(())
}

fn require_schema_object_exact(
    connection: &Connection,
    object_type: &str,
    name: &str,
    expected_sql: &str,
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
            Error::Invariant(format!("schema required {object_type} is missing: {name}"))
        })?;
    let normalized = sql
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized != expected_sql {
        return Err(Error::Invariant(format!(
            "schema {object_type} {name} does not match its required SQL definition"
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
