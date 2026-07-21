use std::{
    fs::{self, File, OpenOptions},
    io::{BufReader, Read},
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use fs2::FileExt;
use rusqlite::{Connection, OpenFlags, OptionalExtension, backup::Backup, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use walkdir::WalkDir;
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::{
    Error, Result,
    database::{Database, SCHEMA_VERSION, database_lock, now_utc, register_runtime_functions},
};

const DATABASE_BACKUP_LIMIT: usize = 30;
const SOURCE_BACKUP_LIMIT: usize = 10;
const PROTECTED_CHANGE_REQUEST_PREDICATE: &str = "change_requests.attempt_count > 0
     OR change_requests.status IN ('approved', 'claimed', 'failed', 'completed', 'cancelled')
     OR EXISTS (
         SELECT 1 FROM change_request_events AS approval_history
         WHERE approval_history.change_request_id = change_requests.id
           AND approval_history.event_type IN (
               'approved', 'reapproved', 'approval_returned',
               'failed_returned_to_draft', 'claimed', 'completed',
               'failed', 'abandoned'
           )
     )";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupArtifact {
    pub path: String,
    pub created_at: String,
    pub sha256: String,
    pub byte_size: u64,
}

pub(crate) fn online_backup_connection(
    source: &Connection,
    backup_directory: &Path,
    reason: &str,
) -> Result<BackupArtifact> {
    fs::create_dir_all(backup_directory)?;
    let lock = backup_lock(backup_directory)?;
    FileExt::lock_exclusive(&lock)?;
    let result = online_backup_connection_inner(source, backup_directory, reason, None);
    FileExt::unlock(&lock)?;
    result
}

fn online_backup_connection_inner(
    source: &Connection,
    backup_directory: &Path,
    reason: &str,
    protected: Option<&Path>,
) -> Result<BackupArtifact> {
    let created_at = now_utc();
    let path = backup_directory.join(unique_name("tm", reason, "sqlite3"));
    let mut destination = Connection::open(&path)?;
    {
        let backup = Backup::new(source, &mut destination)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
    }
    validate_database(&path, false)?;
    let artifact = artifact_for(&path, created_at)?;
    retain_newest(
        backup_directory,
        "sqlite3",
        DATABASE_BACKUP_LIMIT,
        protected,
    )?;
    Ok(artifact)
}

pub(crate) fn create_database_backup(
    database_path: &Path,
    backup_directory: &Path,
    reason: &str,
) -> Result<BackupArtifact> {
    fs::create_dir_all(backup_directory)?;
    let database_lock = database_lock(database_path)?;
    FileExt::lock_shared(&database_lock)?;
    let source = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    online_backup_connection(&source, backup_directory, reason)
}

pub(crate) fn restore_database(
    database_path: &Path,
    backup_directory: &Path,
    selected_backup: &Path,
) -> Result<BackupArtifact> {
    if selected_backup == database_path {
        return Err(Error::InvalidInput(
            "the active database cannot be used as its own restore source".to_owned(),
        ));
    }
    validate_database(selected_backup, true)?;
    if fs::canonicalize(selected_backup)? == fs::canonicalize(database_path)? {
        return Err(Error::InvalidInput(
            "the active database cannot be used as its own restore source".to_owned(),
        ));
    }
    fs::create_dir_all(backup_directory)?;
    let maintenance_lock = database_lock(database_path)?;
    FileExt::lock_exclusive(&maintenance_lock)?;
    let backup_lock = backup_lock(backup_directory)?;
    FileExt::lock_exclusive(&backup_lock)?;

    let current = Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    current.busy_timeout(Duration::from_secs(15))?;
    let delivery_ledger = read_delivery_ledger(&current)?;
    let change_request_ledger = read_change_request_ledger(&current)?;
    let mutation_ledger = read_mutation_ledger(&current)?;
    let ai_budget_ledger = read_ai_budget_ledger(&current)?;
    let assistant_action_ledger = read_assistant_action_ledger(&current)?;
    let assistant_memory_ledger = read_assistant_memory_ledger(&current)?;
    let safety_backup = online_backup_connection_inner(
        &current,
        backup_directory,
        "pre-restore",
        Some(selected_backup),
    )?;
    drop(current);

    let source = Connection::open_with_flags(
        selected_backup,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )?;
    validate_change_request_restore_source(&source, &change_request_ledger)?;
    validate_mutation_restore_source(&source, &mutation_ledger)?;
    validate_ai_budget_restore_source(&source, &ai_budget_ledger)?;
    validate_assistant_action_restore_source(&source, &assistant_action_ledger)?;
    validate_assistant_memory_restore_source(&source, &assistant_memory_ledger)?;
    let mut destination = Connection::open(database_path)?;
    destination.busy_timeout(Duration::from_secs(15))?;
    register_runtime_functions(&destination)?;
    {
        let backup = Backup::new(&source, &mut destination)?;
        backup.run_to_completion(128, Duration::from_millis(10), None)?;
    }
    destination.execute_batch("PRAGMA foreign_keys = ON;")?;
    let restored_version: i64 =
        destination.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Database::migrate(&mut destination, restored_version)?;
    merge_delivery_ledger(&destination, &delivery_ledger)?;
    merge_change_request_ledger(&mut destination, &change_request_ledger)?;
    merge_scheduler_ledger(&destination, Path::new(&safety_backup.path))?;
    destination.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(destination);
    validate_database(database_path, true)?;
    retain_newest(
        backup_directory,
        "sqlite3",
        DATABASE_BACKUP_LIMIT,
        Some(selected_backup),
    )?;
    FileExt::unlock(&backup_lock)?;
    FileExt::unlock(&maintenance_lock)?;
    Ok(safety_backup)
}

pub(crate) fn create_source_snapshot(
    source_directory: &Path,
    backup_directory: &Path,
) -> Result<BackupArtifact> {
    fs::create_dir_all(backup_directory)?;
    let lock = backup_lock(backup_directory)?;
    lock.lock_exclusive()?;
    let created_at = now_utc();
    let output_path = backup_directory.join(unique_name("tm-source", "snapshot", "zip"));
    let output = File::create(&output_path)?;
    let mut archive = ZipWriter::new(output);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o644);

    for entry in WalkDir::new(source_directory).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(source_directory).map_err(|_| {
            Error::Invariant(format!("path escaped source root: {}", path.display()))
        })?;
        if relative.as_os_str().is_empty() || should_skip(relative) {
            continue;
        }
        let archive_name = relative.to_string_lossy().replace('\\', "/");
        if entry.file_type().is_dir() {
            archive.add_directory(format!("{archive_name}/"), options)?;
        } else if entry.file_type().is_file() {
            archive.start_file(archive_name, options)?;
            let mut input = File::open(path)?;
            std::io::copy(&mut input, &mut archive)?;
        }
    }
    archive.finish()?;
    let artifact = artifact_for(&output_path, created_at)?;
    retain_newest(
        backup_directory,
        "zip",
        SOURCE_BACKUP_LIMIT,
        Some(&output_path),
    )?;
    FileExt::unlock(&lock)?;
    Ok(artifact)
}

fn should_skip(relative: &Path) -> bool {
    let blocked_component = relative.components().any(|component| {
        let value = component.as_os_str().to_string_lossy().to_ascii_lowercase();
        matches!(
            value.as_str(),
            ".git"
                | "node_modules"
                | "target"
                | ".vite"
                | "coverage"
                | "dist"
                | "attachments"
                | "logs"
        )
    });
    if blocked_component {
        return true;
    }

    let Some(file_name) = relative.file_name() else {
        return false;
    };
    let file_name = file_name.to_string_lossy().to_ascii_lowercase();
    file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name.ends_with(".log")
        || file_name.ends_with(".sqlite")
        || file_name.ends_with(".sqlite3")
        || file_name.contains(".sqlite-")
        || file_name.contains(".sqlite3-")
        || file_name.ends_with(".pem")
        || file_name.ends_with(".key")
        || file_name.ends_with(".p12")
        || file_name.ends_with(".pfx")
}

fn backup_lock(directory: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(".tm-backup.lock"))?)
}

fn validate_database(path: &Path, require_tm_schema: bool) -> Result<()> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
    let has_core_tables = if require_tm_schema {
        connection
            .query_row(
                "SELECT count(*) = 2 FROM sqlite_schema
                 WHERE type = 'table' AND name IN ('tasks', 'digest_deliveries')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    } else {
        true
    };
    let has_foreign_key_violation = {
        let mut statement = connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
        let mut rows = statement
            .query([])
            .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
        rows.next()
            .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?
            .is_some()
    };
    if integrity != "ok"
        || (require_tm_schema && !(1..=SCHEMA_VERSION).contains(&version))
        || !has_core_tables
        || has_foreign_key_violation
    {
        return Err(Error::InvalidBackup(path.to_path_buf()));
    }
    Ok(())
}

fn artifact_for(path: &Path, created_at: String) -> Result<BackupArtifact> {
    let metadata = path.metadata()?;
    Ok(BackupArtifact {
        path: path.to_string_lossy().into_owned(),
        created_at,
        sha256: sha256_file(path)?,
        byte_size: metadata.len(),
    })
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn unique_name(prefix: &str, reason: &str, extension: &str) -> String {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    format!(
        "{prefix}-{reason}-{timestamp}-{}.{}",
        Uuid::now_v7(),
        extension
    )
}

fn retain_newest(
    directory: &Path,
    extension: &str,
    limit: usize,
    protected: Option<&Path>,
) -> Result<()> {
    let mut candidates: Vec<(PathBuf, std::time::SystemTime)> = fs::read_dir(directory)?
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(extension))
        })
        .filter_map(|path| {
            let modified = path.metadata().ok()?.modified().ok()?;
            Some((path, modified))
        })
        .collect();
    candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| right.0.cmp(&left.0)));

    let protected_is_candidate = protected
        .is_some_and(|protected_path| candidates.iter().any(|(path, _)| path == protected_path));
    let ordinary_limit = limit.saturating_sub(usize::from(protected_is_candidate));
    let mut retained_ordinary = 0_usize;
    for (obsolete, _) in candidates {
        if protected.is_some_and(|protected_path| obsolete == protected_path) {
            continue;
        }
        if retained_ordinary < ordinary_limit {
            retained_ordinary += 1;
            continue;
        }
        fs::remove_file(obsolete)?;
    }
    Ok(())
}

#[derive(Debug)]
struct DeliveryLedgerRow {
    delivery_key: String,
    kind: String,
    digest_date: String,
    status: String,
    attempt_count: i64,
    claimed_at: String,
    claim_expires_at: String,
    sent_at: Option<String>,
    slack_ref: Option<String>,
    failed_at: Option<String>,
    failure_reason: Option<String>,
    updated_at: String,
}

fn read_delivery_ledger(connection: &Connection) -> Result<Vec<DeliveryLedgerRow>> {
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'digest_deliveries')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT delivery_key, kind, digest_date, status, attempt_count,
                claimed_at, claim_expires_at, sent_at, slack_ref,
                failed_at, failure_reason, updated_at
         FROM digest_deliveries WHERE status IN ('sent', 'claimed')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(DeliveryLedgerRow {
            delivery_key: row.get(0)?,
            kind: row.get(1)?,
            digest_date: row.get(2)?,
            status: row.get(3)?,
            attempt_count: row.get(4)?,
            claimed_at: row.get(5)?,
            claim_expires_at: row.get(6)?,
            sent_at: row.get(7)?,
            slack_ref: row.get(8)?,
            failed_at: row.get(9)?,
            failure_reason: row.get(10)?,
            updated_at: row.get(11)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn merge_delivery_ledger(connection: &Connection, preserved: &[DeliveryLedgerRow]) -> Result<()> {
    for row in preserved {
        let restored_status: Option<String> = connection
            .query_row(
                "SELECT status FROM digest_deliveries WHERE delivery_key = ?1",
                [&row.delivery_key],
                |result| result.get(0),
            )
            .optional()?;
        if restored_status.as_deref() == Some("sent") {
            continue;
        }
        if restored_status.is_none() {
            connection.execute(
                "INSERT INTO digest_deliveries(
                    delivery_key, kind, digest_date, status, attempt_count,
                    claimed_at, claim_expires_at, sent_at, slack_ref,
                    failed_at, failure_reason, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.delivery_key,
                    row.kind,
                    row.digest_date,
                    row.status,
                    row.attempt_count,
                    row.claimed_at,
                    row.claim_expires_at,
                    row.sent_at,
                    row.slack_ref,
                    row.failed_at,
                    row.failure_reason,
                    row.updated_at,
                ],
            )?;
            continue;
        }
        let preserved_wins = row.status == "sent"
            || (row.status == "claimed" && restored_status.as_deref() != Some("sent"));
        if preserved_wins {
            connection.execute(
                "UPDATE digest_deliveries
                 SET status = ?2, attempt_count = ?3, claimed_at = ?4,
                     claim_expires_at = ?5, sent_at = ?6, slack_ref = ?7,
                     failed_at = ?8, failure_reason = ?9, updated_at = ?10
                 WHERE delivery_key = ?1 AND status <> 'sent'",
                params![
                    row.delivery_key,
                    row.status,
                    row.attempt_count,
                    row.claimed_at,
                    row.claim_expires_at,
                    row.sent_at,
                    row.slack_ref,
                    row.failed_at,
                    row.failure_reason,
                    row.updated_at,
                ],
            )?;
        }
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MutationLedger {
    idempotency_rows: Vec<String>,
    audit_rows: Vec<String>,
}

fn read_mutation_ledger(connection: &Connection) -> Result<MutationLedger> {
    let has_idempotency: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type = 'table' AND name = 'mutation_idempotency_records'
         )",
        [],
        |row| row.get(0),
    )?;
    let has_audit: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type = 'table' AND name = 'mutation_audit_events'
         )",
        [],
        |row| row.get(0),
    )?;
    if !has_idempotency && !has_audit {
        return Ok(MutationLedger::default());
    }
    if !has_idempotency || !has_audit {
        return Err(Error::Invariant(
            "mutation ledger tables must exist together".to_owned(),
        ));
    }

    let idempotency_rows = canonical_json_rows(
        connection,
        "SELECT json_array(
            idempotency_key, operation, request_sha256, actor, request_id,
            resource_type, resource_id, resource_version, response_json, created_at
         )
         FROM mutation_idempotency_records ORDER BY idempotency_key",
    )?;
    let audit_rows = canonical_json_rows(
        connection,
        "SELECT json_array(
            id, idempotency_key, actor, request_id, operation, resource_type,
            resource_id, expected_version, resulting_version, before_json,
            after_json, result, approval_policy, created_at
         )
         FROM mutation_audit_events ORDER BY id",
    )?;
    Ok(MutationLedger {
        idempotency_rows,
        audit_rows,
    })
}

fn canonical_json_rows(connection: &Connection, sql: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn validate_mutation_restore_source(source: &Connection, current: &MutationLedger) -> Result<()> {
    if current.idempotency_rows.is_empty() && current.audit_rows.is_empty() {
        return Ok(());
    }
    let restored = read_mutation_ledger(source)?;
    if &restored != current {
        return Err(Error::Conflict(
            "restore would alter the append-only mutation and idempotency ledger".to_owned(),
        ));
    }
    Ok(())
}

fn read_ai_budget_ledger(connection: &Connection) -> Result<Vec<String>> {
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type = 'table' AND name = 'ai_budget_ledger'
         )",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(Vec::new());
    }
    canonical_json_rows(
        connection,
        "SELECT json_array(
            id, request_id, entry_kind, provider, model, operation, budget_month,
            amount_microusd, input_tokens, cached_input_tokens, output_tokens,
            total_tokens, outcome, created_at
         )
         FROM ai_budget_ledger ORDER BY id",
    )
}

fn validate_ai_budget_restore_source(source: &Connection, current: &[String]) -> Result<()> {
    if current.is_empty() {
        return Ok(());
    }
    let restored = read_ai_budget_ledger(source)?;
    if restored != current {
        return Err(Error::Conflict(
            "restore would alter the append-only AI cost and budget ledger".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AssistantActionLedger {
    requests: Vec<String>,
    events: Vec<String>,
}

fn read_assistant_action_ledger(connection: &Connection) -> Result<AssistantActionLedger> {
    let has_requests: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type = 'table' AND name = 'assistant_action_requests'
         )",
        [],
        |row| row.get(0),
    )?;
    let has_events: bool = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_schema
            WHERE type = 'table' AND name = 'assistant_action_events'
         )",
        [],
        |row| row.get(0),
    )?;
    if !has_requests && !has_events {
        return Ok(AssistantActionLedger::default());
    }
    if !has_requests || !has_events {
        return Err(Error::Invariant(
            "assistant action ledger tables must exist together".to_owned(),
        ));
    }
    let requests = canonical_json_rows(
        connection,
        "SELECT json_array(
            id, operation, status, revision, payload_json, payload_sha256,
            origin_request_id, execution_idempotency_key, approval_idempotency_key,
            result_json, failure_code, created_at, expires_at, approved_at,
            executing_at, completed_at, terminal_at
         )
         FROM assistant_action_requests ORDER BY id",
    )?;
    let events = canonical_json_rows(
        connection,
        "SELECT json_array(
            id, action_id, event_type, from_status, to_status, revision, actor,
            request_id, payload_sha256, metadata_json, created_at
         )
         FROM assistant_action_events ORDER BY id",
    )?;
    Ok(AssistantActionLedger { requests, events })
}

fn validate_assistant_action_restore_source(
    source: &Connection,
    current: &AssistantActionLedger,
) -> Result<()> {
    if current.requests.is_empty() && current.events.is_empty() {
        return Ok(());
    }
    let restored = read_assistant_action_ledger(source)?;
    if &restored != current {
        return Err(Error::Conflict(
            "restore would alter the immutable assistant action approval ledger".to_owned(),
        ));
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
struct AssistantMemoryLedger {
    memories: Vec<String>,
    sources: Vec<String>,
    events: Vec<String>,
}

fn read_assistant_memory_ledger(connection: &Connection) -> Result<AssistantMemoryLedger> {
    let table_names = [
        "assistant_memories",
        "assistant_memory_sources",
        "assistant_memory_events",
    ];
    let mut exists = Vec::with_capacity(table_names.len());
    for table in table_names {
        exists.push(connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1
             )",
            [table],
            |row| row.get::<_, bool>(0),
        )?);
    }
    if exists.iter().all(|value| !value) {
        return Ok(AssistantMemoryLedger::default());
    }
    if exists.iter().any(|value| !value) {
        return Err(Error::Invariant(
            "assistant memory ledger tables must exist together".to_owned(),
        ));
    }
    let memories = canonical_json_rows(
        connection,
        "SELECT json_array(
            id, kind, title, body, source_type, source_id, provenance_json,
            sensitivity, openai_allowed, retention, expires_at, period_kind,
            period_start, period_end, summary_key, revision, content_sha256,
            created_at, updated_at, deleted_at
         ) FROM assistant_memories ORDER BY id",
    )?;
    let sources = canonical_json_rows(
        connection,
        "SELECT json_array(
            memory_id, source_type, source_id, source_revision, source_updated_at, created_at
         ) FROM assistant_memory_sources ORDER BY memory_id, source_type, source_id",
    )?;
    let events = canonical_json_rows(
        connection,
        "SELECT json_array(
            id, memory_id, event_type, revision, actor, request_id,
            content_sha256, metadata_json, created_at
         ) FROM assistant_memory_events ORDER BY id",
    )?;
    Ok(AssistantMemoryLedger {
        memories,
        sources,
        events,
    })
}

fn validate_assistant_memory_restore_source(
    source: &Connection,
    current: &AssistantMemoryLedger,
) -> Result<()> {
    if current.memories.is_empty() && current.sources.is_empty() && current.events.is_empty() {
        return Ok(());
    }
    let restored = read_assistant_memory_ledger(source)?;
    if &restored != current {
        return Err(Error::Conflict(
            "restore would alter the assistant memory and provenance ledger".to_owned(),
        ));
    }
    Ok(())
}

fn merge_scheduler_ledger(connection: &Connection, preserved_database: &Path) -> Result<()> {
    connection.execute(
        "ATTACH DATABASE ?1 AS scheduler_preserved",
        [preserved_database.to_string_lossy().as_ref()],
    )?;
    let merge_result = connection.execute_batch(
        "BEGIN IMMEDIATE;
         INSERT INTO scheduler_jobs(
            id, job_key, kind, schedule_type, interval_seconds, local_time, timezone,
            enabled, max_attempts, misfire_grace_seconds, coalesce, next_run_at,
            last_scheduled_at, created_at, updated_at
         )
         SELECT id, job_key, kind, schedule_type, interval_seconds, local_time, timezone,
                enabled, max_attempts, misfire_grace_seconds, coalesce, next_run_at,
                last_scheduled_at, created_at, updated_at
         FROM scheduler_preserved.scheduler_jobs WHERE true
         ON CONFLICT(id) DO UPDATE SET
            enabled = excluded.enabled,
            max_attempts = excluded.max_attempts,
            misfire_grace_seconds = excluded.misfire_grace_seconds,
            coalesce = excluded.coalesce,
            next_run_at = excluded.next_run_at,
            last_scheduled_at = excluded.last_scheduled_at,
            updated_at = excluded.updated_at;

         INSERT INTO scheduler_runs(
            id, job_id, scheduled_for, status, attempt_count, max_attempts,
            idempotency_key, available_at, lease_owner, lease_acquired_at,
            lease_expires_at, last_error, result_json, created_at, updated_at,
            started_at, completed_at, dead_letter_at, skipped_at
         )
         SELECT id, job_id, scheduled_for, status, attempt_count, max_attempts,
                idempotency_key, available_at, lease_owner, lease_acquired_at,
                lease_expires_at, last_error, result_json, created_at, updated_at,
                started_at, completed_at, dead_letter_at, skipped_at
         FROM scheduler_preserved.scheduler_runs WHERE true
         ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            attempt_count = excluded.attempt_count,
            available_at = excluded.available_at,
            lease_owner = excluded.lease_owner,
            lease_acquired_at = excluded.lease_acquired_at,
            lease_expires_at = excluded.lease_expires_at,
            last_error = excluded.last_error,
            result_json = excluded.result_json,
            updated_at = excluded.updated_at,
            started_at = excluded.started_at,
            completed_at = excluded.completed_at,
            dead_letter_at = excluded.dead_letter_at,
            skipped_at = excluded.skipped_at;

         INSERT OR IGNORE INTO scheduler_attempts(
            id, run_id, attempt_number, worker_id, started_at, completed_at,
            outcome, error, created_at
         )
         SELECT id, run_id, attempt_number, worker_id, started_at, completed_at,
                outcome, error, created_at
         FROM scheduler_preserved.scheduler_attempts;

         INSERT OR IGNORE INTO scheduler_effects(
            idempotency_key, run_id, job_kind, result_json, applied_at
         )
         SELECT idempotency_key, run_id, job_kind, result_json, applied_at
         FROM scheduler_preserved.scheduler_effects;
         COMMIT;",
    );
    if merge_result.is_err() {
        let _ = connection.execute_batch("ROLLBACK;");
    }
    let detach_result = connection.execute_batch("DETACH DATABASE scheduler_preserved;");
    merge_result?;
    detach_result?;
    Ok(())
}

#[derive(Debug, Default)]
struct ChangeRequestLedger {
    requests: Vec<ChangeRequestLedgerRow>,
    events: Vec<ChangeRequestEventLedgerRow>,
}

#[derive(Debug)]
struct ChangeRequestLedgerRow {
    id: String,
    kind: String,
    project_id: Option<String>,
    task_id: Option<String>,
    title: String,
    description: String,
    desired_outcome: String,
    reproduction_steps: String,
    priority: i64,
    status: String,
    revision: i64,
    approved_revision: Option<i64>,
    attempt_count: i64,
    requested_by: String,
    updated_by: String,
    created_at: String,
    updated_at: String,
    approved_at: Option<String>,
    approved_by: Option<String>,
    claim_key: Option<String>,
    claimed_at: Option<String>,
    claimed_by: Option<String>,
    completed_at: Option<String>,
    completed_by: Option<String>,
    failed_at: Option<String>,
    failed_by: Option<String>,
    cancelled_at: Option<String>,
    cancelled_by: Option<String>,
    result_summary: Option<String>,
    patch_ref: Option<String>,
    failure_reason: Option<String>,
    cancellation_reason: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ChangeRequestEventLedgerRow {
    id: String,
    change_request_id: String,
    event_type: String,
    from_status: Option<String>,
    to_status: String,
    revision: i64,
    actor: String,
    details_json: String,
    created_at: String,
}

fn read_change_request_ledger(connection: &Connection) -> Result<ChangeRequestLedger> {
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'change_requests')",
        [],
        |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(ChangeRequestLedger::default());
    }

    let request_sql = format!(
        "SELECT id, kind, project_id, task_id, title, description, desired_outcome,
                reproduction_steps, priority, status, revision, approved_revision,
                attempt_count, requested_by, updated_by, created_at, updated_at,
                approved_at, approved_by, claim_key, claimed_at, claimed_by,
                completed_at, completed_by, failed_at, failed_by,
                cancelled_at, cancelled_by, result_summary, patch_ref,
                failure_reason, cancellation_reason
         FROM change_requests
         WHERE {PROTECTED_CHANGE_REQUEST_PREDICATE}"
    );
    let mut request_statement = connection.prepare(&request_sql)?;
    let requests = request_statement
        .query_map([], |row| {
            Ok(ChangeRequestLedgerRow {
                id: row.get(0)?,
                kind: row.get(1)?,
                project_id: row.get(2)?,
                task_id: row.get(3)?,
                title: row.get(4)?,
                description: row.get(5)?,
                desired_outcome: row.get(6)?,
                reproduction_steps: row.get(7)?,
                priority: row.get(8)?,
                status: row.get(9)?,
                revision: row.get(10)?,
                approved_revision: row.get(11)?,
                attempt_count: row.get(12)?,
                requested_by: row.get(13)?,
                updated_by: row.get(14)?,
                created_at: row.get(15)?,
                updated_at: row.get(16)?,
                approved_at: row.get(17)?,
                approved_by: row.get(18)?,
                claim_key: row.get(19)?,
                claimed_at: row.get(20)?,
                claimed_by: row.get(21)?,
                completed_at: row.get(22)?,
                completed_by: row.get(23)?,
                failed_at: row.get(24)?,
                failed_by: row.get(25)?,
                cancelled_at: row.get(26)?,
                cancelled_by: row.get(27)?,
                result_summary: row.get(28)?,
                patch_ref: row.get(29)?,
                failure_reason: row.get(30)?,
                cancellation_reason: row.get(31)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(request_statement);

    let event_sql = format!(
        "SELECT id, change_request_id, event_type, from_status, to_status,
                revision, actor, details_json, created_at
         FROM change_request_events
         WHERE change_request_id IN (
             SELECT id FROM change_requests
             WHERE {PROTECTED_CHANGE_REQUEST_PREDICATE}
         )"
    );
    let mut event_statement = connection.prepare(&event_sql)?;
    let events = event_statement
        .query_map([], |row| {
            Ok(ChangeRequestEventLedgerRow {
                id: row.get(0)?,
                change_request_id: row.get(1)?,
                event_type: row.get(2)?,
                from_status: row.get(3)?,
                to_status: row.get(4)?,
                revision: row.get(5)?,
                actor: row.get(6)?,
                details_json: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ChangeRequestLedger { requests, events })
}

fn merge_change_request_ledger(
    connection: &mut Connection,
    ledger: &ChangeRequestLedger,
) -> Result<()> {
    if ledger.requests.is_empty() && ledger.events.is_empty() {
        return Ok(());
    }
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT INTO app_state(key, value_json, updated_at)
         VALUES ('change_request_restore', 'true', ?1)
         ON CONFLICT(key) DO UPDATE SET value_json = 'true', updated_at = excluded.updated_at",
        [now_utc()],
    )?;

    for row in &ledger.requests {
        transaction.execute(
            "INSERT INTO change_requests(
                id, kind, project_id, task_id, title, description, desired_outcome,
                reproduction_steps, priority, status, revision, approved_revision,
                attempt_count, requested_by, updated_by, created_at, updated_at,
                approved_at, approved_by, claim_key, claimed_at, claimed_by,
                completed_at, completed_by, failed_at, failed_by,
                cancelled_at, cancelled_by, result_summary, patch_ref,
                failure_reason, cancellation_reason
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22,
                ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32
             )
             ON CONFLICT(id) DO UPDATE SET
                kind = excluded.kind,
                project_id = excluded.project_id,
                task_id = excluded.task_id,
                title = excluded.title,
                description = excluded.description,
                desired_outcome = excluded.desired_outcome,
                reproduction_steps = excluded.reproduction_steps,
                priority = excluded.priority,
                status = excluded.status,
                revision = excluded.revision,
                approved_revision = excluded.approved_revision,
                attempt_count = excluded.attempt_count,
                requested_by = excluded.requested_by,
                updated_by = excluded.updated_by,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                approved_at = excluded.approved_at,
                approved_by = excluded.approved_by,
                claim_key = excluded.claim_key,
                claimed_at = excluded.claimed_at,
                claimed_by = excluded.claimed_by,
                completed_at = excluded.completed_at,
                completed_by = excluded.completed_by,
                failed_at = excluded.failed_at,
                failed_by = excluded.failed_by,
                cancelled_at = excluded.cancelled_at,
                cancelled_by = excluded.cancelled_by,
                result_summary = excluded.result_summary,
                patch_ref = excluded.patch_ref,
                failure_reason = excluded.failure_reason,
                cancellation_reason = excluded.cancellation_reason",
            params![
                row.id,
                row.kind,
                row.project_id,
                row.task_id,
                row.title,
                row.description,
                row.desired_outcome,
                row.reproduction_steps,
                row.priority,
                row.status,
                row.revision,
                row.approved_revision,
                row.attempt_count,
                row.requested_by,
                row.updated_by,
                row.created_at,
                row.updated_at,
                row.approved_at,
                row.approved_by,
                row.claim_key,
                row.claimed_at,
                row.claimed_by,
                row.completed_at,
                row.completed_by,
                row.failed_at,
                row.failed_by,
                row.cancelled_at,
                row.cancelled_by,
                row.result_summary,
                row.patch_ref,
                row.failure_reason,
                row.cancellation_reason,
            ],
        )?;
    }

    for row in &ledger.events {
        transaction.execute(
            "INSERT OR IGNORE INTO change_request_events(
                id, change_request_id, event_type, from_status, to_status,
                revision, actor, details_json, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                row.id,
                row.change_request_id,
                row.event_type,
                row.from_status,
                row.to_status,
                row.revision,
                row.actor,
                row.details_json,
                row.created_at,
            ],
        )?;
    }
    transaction.execute(
        "DELETE FROM app_state WHERE key = 'change_request_restore'",
        [],
    )?;
    transaction.commit()?;
    Ok(())
}

fn validate_change_request_restore_source(
    source: &Connection,
    ledger: &ChangeRequestLedger,
) -> Result<()> {
    let has_change_requests: bool = source.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'change_requests')",
        [],
        |row| row.get(0),
    )?;
    for current in &ledger.requests {
        validate_restore_reference(
            source,
            "projects",
            current.project_id.as_deref(),
            &current.id,
        )?;
        validate_restore_reference(source, "tasks", current.task_id.as_deref(), &current.id)?;
        if !has_change_requests {
            continue;
        }
        let restored: Option<(String, i64, i64, String)> = source
            .query_row(
                "SELECT status, attempt_count, revision, updated_at
                 FROM change_requests WHERE id = ?1",
                [&current.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((restored_status, restored_attempt, restored_revision, restored_updated_at)) =
            restored
        else {
            let has_orphan_event: bool = source.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM change_request_events WHERE change_request_id = ?1
                 )",
                [&current.id],
                |row| row.get(0),
            )?;
            if has_orphan_event {
                return Err(Error::Conflict(format!(
                    "backup contains orphan change request events for {}",
                    current.id
                )));
            }
            continue;
        };
        let restored_terminal = matches!(restored_status.as_str(), "completed" | "cancelled");
        if restored_terminal && restored_status != current.status {
            return Err(Error::Conflict(format!(
                "restore would replace terminal change request {} ({restored_status}) with {}",
                current.id, current.status
            )));
        }
        if restored_attempt > current.attempt_count
            || restored_revision > current.revision
            || restored_updated_at > current.updated_at
        {
            return Err(Error::Conflict(format!(
                "backup contains a newer conflicting change request ledger for {}",
                current.id
            )));
        }
        validate_change_request_event_ancestry(source, ledger, &current.id)?;
    }
    Ok(())
}

fn validate_change_request_event_ancestry(
    source: &Connection,
    ledger: &ChangeRequestLedger,
    request_id: &str,
) -> Result<()> {
    let mut statement = source.prepare(
        "SELECT id, change_request_id, event_type, from_status, to_status,
                revision, actor, details_json, created_at
         FROM change_request_events WHERE change_request_id = ?1",
    )?;
    let restored_events = statement
        .query_map([request_id], |row| {
            Ok(ChangeRequestEventLedgerRow {
                id: row.get(0)?,
                change_request_id: row.get(1)?,
                event_type: row.get(2)?,
                from_status: row.get(3)?,
                to_status: row.get(4)?,
                revision: row.get(5)?,
                actor: row.get(6)?,
                details_json: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for restored in restored_events {
        let Some(current) = ledger.events.iter().find(|event| event.id == restored.id) else {
            return Err(Error::Conflict(format!(
                "backup contains a divergent change request event for {request_id}: {}",
                restored.id
            )));
        };
        if current != &restored {
            return Err(Error::Conflict(format!(
                "backup changed append-only event {} for change request {request_id}",
                restored.id
            )));
        }
    }
    Ok(())
}

fn validate_restore_reference(
    source: &Connection,
    table: &str,
    candidate: Option<&str>,
    request_id: &str,
) -> Result<()> {
    let Some(candidate) = candidate else {
        return Ok(());
    };
    let sql = match table {
        "projects" => "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
        "tasks" => "SELECT EXISTS(SELECT 1 FROM tasks WHERE id = ?1)",
        _ => {
            return Err(Error::Invariant(format!(
                "unsupported change request reference table: {table}"
            )));
        }
    };
    let exists: bool = source.query_row(sql, [candidate], |row| row.get(0))?;
    if !exists {
        return Err(Error::Conflict(format!(
            "cannot restore without changing change request {request_id}: referenced {table} row {candidate} is absent"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::should_skip;

    #[test]
    fn source_snapshot_excludes_generated_and_sensitive_content() {
        for path in [
            ".env",
            ".env.local",
            "data/tm.sqlite3",
            "data/tm.sqlite3-wal",
            "logs/tm.log",
            "attachments/private.txt",
            "node_modules/package/index.js",
            "certificates/signing.pfx",
        ] {
            assert!(should_skip(Path::new(path)), "expected to skip {path}");
        }

        for path in ["src/main.rs", "docs/patches/README.md", "src/data/model.ts"] {
            assert!(!should_skip(Path::new(path)), "expected to keep {path}");
        }
    }
}
