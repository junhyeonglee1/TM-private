use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use rusqlite::{Transaction, TransactionBehavior, types::ValueRef};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::{
    ExportArtifact, Result,
    backup::sha256_file,
    database::{Database, SCHEMA_VERSION, now_utc},
};

pub(crate) const EXPORTED_TABLES: &[&str] = &[
    "schema_migrations",
    "projects",
    "tasks",
    "checklist_items",
    "tags",
    "task_tags",
    "task_day_entries",
    "task_events",
    "work_sessions",
    "session_tasks",
    "worklogs",
    "notes",
    "entity_links",
    "attachments",
    "digest_deliveries",
    "change_requests",
    "change_request_events",
    "mutation_idempotency_records",
    "mutation_audit_events",
    "ai_budget_ledger",
    "assistant_action_requests",
    "assistant_action_events",
    "assistant_memories",
    "assistant_memory_sources",
    "assistant_memory_events",
    "scheduler_jobs",
    "scheduler_runs",
    "scheduler_attempts",
    "scheduler_effects",
    "app_state",
];

pub(crate) fn json_snapshot(database: &Database) -> Result<Value> {
    database.transaction(TransactionBehavior::Deferred, snapshot_in_transaction)
}

pub(crate) fn markdown_snapshot(database: &Database) -> Result<String> {
    let snapshot = json_snapshot(database)?;
    render_markdown(&snapshot)
}

pub(crate) fn create_artifacts(database: &Database) -> Result<ExportArtifact> {
    let snapshot = json_snapshot(database)?;
    let markdown = render_markdown(&snapshot)?;
    let exported_at = snapshot
        .get("exportedAt")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let suffix = Uuid::now_v7();
    let json_path = database
        .home()
        .exports_dir()
        .join(format!("tm-export-{suffix}.json"));
    let markdown_path = database
        .home()
        .exports_dir()
        .join(format!("tm-export-{suffix}.md"));
    write_new_file(
        &json_path,
        serde_json::to_string_pretty(&snapshot)?.as_bytes(),
    )?;
    write_new_file(&markdown_path, markdown.as_bytes())?;
    Ok(ExportArtifact {
        json_path: json_path.to_string_lossy().into_owned(),
        markdown_path: markdown_path.to_string_lossy().into_owned(),
        created_at: exported_at,
        sha256: sha256_file(&json_path)?,
    })
}

fn snapshot_in_transaction(transaction: &Transaction<'_>) -> Result<Value> {
    let exported_at = now_utc();
    let mut tables = Map::new();
    for table in EXPORTED_TABLES {
        tables.insert((*table).to_owned(), export_table(transaction, table)?);
    }
    Ok(json!({
        "format": "tm-full-export",
        "formatVersion": 1,
        "schemaVersion": SCHEMA_VERSION,
        "exportedAt": exported_at,
        "timezone": "Asia/Seoul",
        "tables": tables,
    }))
}

fn export_table(transaction: &Transaction<'_>, table: &str) -> Result<Value> {
    let pragma = format!("PRAGMA table_info(\"{table}\")");
    let mut column_statement = transaction.prepare(&pragma)?;
    let columns = column_statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let query = format!("SELECT * FROM \"{table}\"");
    let mut statement = transaction.prepare(&query)?;
    let mut rows = statement.query([])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let mut object = Map::new();
        for (index, column) in columns.iter().enumerate() {
            object.insert(column.clone(), value_to_json(row.get_ref(index)?));
        }
        output.push(Value::Object(object));
    }
    Ok(Value::Array(output))
}

fn value_to_json(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::from(value),
        ValueRef::Text(value) => Value::String(String::from_utf8_lossy(value).into_owned()),
        ValueRef::Blob(value) => Value::String(format!("hex:{}", encode_hex(value))),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn render_markdown(snapshot: &Value) -> Result<String> {
    let exported_at = snapshot
        .get("exportedAt")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut markdown = format!(
        "# TM 전체 내보내기\n\n- 내보낸 시각(UTC): `{exported_at}`\n- 사용자 시간대: `Asia/Seoul`\n- 스키마 버전: `{SCHEMA_VERSION}`\n\n"
    );
    if let Some(tables) = snapshot.get("tables").and_then(Value::as_object) {
        for table in EXPORTED_TABLES {
            let rows = tables.get(*table).cloned().unwrap_or_else(|| json!([]));
            let count = rows.as_array().map_or(0, Vec::len);
            markdown.push_str(&format!(
                "## `{table}` ({count})\n\n````json\n{}\n````\n\n",
                serde_json::to_string_pretty(&rows)?
            ));
        }
    }
    Ok(markdown)
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(contents)?;
    file.sync_all()?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn temporary_path(final_path: &Path) -> PathBuf {
    let file_name = final_path
        .file_name()
        .map_or_else(|| "tm-export".into(), |value| value.to_os_string());
    let mut temporary_name = file_name;
    temporary_name.push(format!(".{}.tmp", Uuid::now_v7()));
    final_path.with_file_name(temporary_name)
}
