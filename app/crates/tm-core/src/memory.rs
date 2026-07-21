use std::{collections::HashSet, str::FromStr};

use chrono::{DateTime, Datelike, Days, NaiveDate, SecondsFormat, Utc};
use rusqlite::{
    Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params, types::Type,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    Error, Result, TmCore,
    database::{new_id, now_utc},
    error::{invalid, not_found},
};

const MAX_MEMORY_TITLE_CHARS: usize = 200;
const MAX_MEMORY_BODY_CHARS: usize = 4_000;
const MAX_MEMORY_SEARCH_QUERY_CHARS: usize = 256;
pub const MEMORY_CONTEXT_MAX_ITEMS: usize = 12;
pub const MEMORY_CONTEXT_MAX_BYTES: usize = 6 * 1024;

macro_rules! memory_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(Error::Invariant(format!(
                        "unknown {} value: {other}", stringify!($name)
                    ))),
                }
            }
        }
    };
}

memory_enum!(MemoryKind {
    Preference => "preference",
    Goal => "goal",
    Routine => "routine",
    Constraint => "constraint",
    Reference => "reference",
    Summary => "summary",
});

memory_enum!(MemorySensitivity {
    Normal => "normal",
    Private => "private",
    Restricted => "restricted",
});

memory_enum!(MemoryRetention {
    UntilDeleted => "until_deleted",
    Daily90d => "daily_90d",
    Weekly365d => "weekly_365d",
    Monthly1095d => "monthly_1095d",
});

memory_enum!(MemorySourceType {
    Explicit => "explicit",
    Task => "task",
    Note => "note",
    Worklog => "worklog",
    Session => "session",
    MemoryRollup => "memory_rollup",
});

memory_enum!(MemoryPeriodKind {
    Daily => "daily",
    Weekly => "weekly",
    Monthly => "monthly",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateMemoryInput {
    pub kind: MemoryKind,
    pub title: String,
    pub body: String,
    pub sensitivity: MemorySensitivity,
    pub openai_allowed: bool,
    pub retention: MemoryRetention,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryPatch {
    pub kind: MemoryKind,
    pub title: String,
    pub body: String,
    pub sensitivity: MemorySensitivity,
    pub openai_allowed: bool,
    pub retention: MemoryRetention,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMemory {
    pub id: String,
    pub kind: MemoryKind,
    pub title: String,
    pub body: String,
    pub source_type: MemorySourceType,
    pub source_id: Option<String>,
    pub provenance: Value,
    pub sensitivity: MemorySensitivity,
    pub openai_allowed: bool,
    pub retention: MemoryRetention,
    pub expires_at: Option<String>,
    pub period_kind: Option<MemoryPeriodKind>,
    pub period_start: Option<NaiveDate>,
    pub period_end: Option<NaiveDate>,
    pub summary_key: Option<String>,
    pub revision: u64,
    pub content_sha256: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchFilter {
    pub query: String,
    pub kind: Option<MemoryKind>,
    pub openai_only: bool,
    pub max_items: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResult {
    pub items: Vec<AssistantMemory>,
    pub bytes_used: usize,
    pub omitted: usize,
    pub retrieval: &'static str,
    pub vector_service_used: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMaintenanceReport {
    pub expired: usize,
    pub source_deleted: usize,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySummaryRegenerationReport {
    pub daily: Option<String>,
    pub weekly: Option<String>,
    pub monthly: Option<String>,
    pub regenerated_at: String,
    pub openai_calls: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMemoryEvent {
    pub id: String,
    pub memory_id: String,
    pub event_type: String,
    pub revision: u64,
    pub actor: String,
    pub request_id: String,
    pub content_sha256: String,
    pub metadata: Option<Value>,
    pub created_at: String,
}

impl TmCore {
    pub fn list_assistant_memories(&self, include_deleted: bool) -> Result<Vec<AssistantMemory>> {
        self.run_memory_maintenance(Utc::now())?;
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM assistant_memories
             WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY updated_at DESC, id DESC LIMIT 500"
        ))?;
        let rows = statement.query_map([i64::from(include_deleted)], map_memory)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_assistant_memory(&self, memory_id: &str) -> Result<AssistantMemory> {
        validate_uuid("memory ID", memory_id)?;
        self.run_memory_maintenance(Utc::now())?;
        let connection = self.database.connect()?;
        query_memory(&connection, memory_id)
    }

    pub fn list_assistant_memory_events(
        &self,
        memory_id: &str,
    ) -> Result<Vec<AssistantMemoryEvent>> {
        validate_uuid("memory ID", memory_id)?;
        let connection = self.database.connect()?;
        query_memory(&connection, memory_id)?;
        let mut statement = connection.prepare(
            "SELECT id, memory_id, event_type, revision, actor, request_id,
                    content_sha256, metadata_json, created_at
             FROM assistant_memory_events WHERE memory_id = ?1
             ORDER BY created_at, id",
        )?;
        let rows = statement.query_map([memory_id], map_memory_event)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn search_assistant_memories(
        &self,
        filter: MemorySearchFilter,
    ) -> Result<MemorySearchResult> {
        validate_search_filter(&filter)?;
        self.run_memory_maintenance(Utc::now())?;
        let query = memory_fts_query(&filter.query)?;
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {MEMORY_COLUMNS_PREFIXED}
             FROM assistant_memory_search
             JOIN assistant_memories AS memory
               ON memory.id = assistant_memory_search.memory_id
             WHERE assistant_memory_search MATCH ?1
               AND memory.deleted_at IS NULL
               AND (?2 IS NULL OR memory.kind = ?2)
               AND (?3 = 0 OR (memory.openai_allowed = 1 AND memory.sensitivity = 'normal'))
               AND (memory.expires_at IS NULL OR memory.expires_at > ?4)
             ORDER BY bm25(assistant_memory_search), memory.updated_at DESC, memory.id DESC
             LIMIT 200"
        ))?;
        let kind = filter.kind.map(MemoryKind::as_str);
        let now = now_utc();
        let rows = statement.query_map(
            params![query, kind, i64::from(filter.openai_only), now],
            map_memory,
        )?;
        let candidates = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(apply_context_budget(
            candidates,
            filter.max_items,
            filter.max_bytes,
        ))
    }

    pub fn run_memory_maintenance(&self, as_of: DateTime<Utc>) -> Result<MemoryMaintenanceReport> {
        let checked_at = as_of.to_rfc3339_opts(SecondsFormat::Millis, true);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let expired_ids = query_ids(
                    transaction,
                    "SELECT id FROM assistant_memories
                     WHERE deleted_at IS NULL AND expires_at IS NOT NULL AND expires_at <= ?1",
                    &checked_at,
                )?;
                let mut expired = 0;
                for memory_id in expired_ids {
                    if soft_delete_memory(
                        transaction,
                        &memory_id,
                        "retention",
                        "memory-retention",
                        "expired",
                        &checked_at,
                    )? {
                        expired += 1;
                    }
                }

                let invalid_source_ids = invalid_source_memory_ids(transaction)?;
                let mut source_deleted = 0;
                for memory_id in invalid_source_ids {
                    if soft_delete_memory(
                        transaction,
                        &memory_id,
                        "system",
                        "memory-source-cleanup",
                        "source_deleted",
                        &checked_at,
                    )? {
                        source_deleted += 1;
                    }
                }

                Ok(MemoryMaintenanceReport {
                    expired,
                    source_deleted,
                    checked_at,
                })
            })
    }

    pub fn regenerate_memory_summaries(
        &self,
        as_of: NaiveDate,
    ) -> Result<MemorySummaryRegenerationReport> {
        let regenerated_at = now_utc();
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let daily = regenerate_rollup(
                    transaction,
                    MemoryPeriodKind::Daily,
                    as_of,
                    as_of,
                    &regenerated_at,
                )?;
                let days_from_monday = u64::from(as_of.weekday().num_days_from_monday());
                let weekly_start = as_of
                    .checked_sub_days(Days::new(days_from_monday))
                    .ok_or_else(|| invalid("weekly summary date is out of range"))?;
                let weekly_end = weekly_start
                    .checked_add_days(Days::new(6))
                    .ok_or_else(|| invalid("weekly summary date is out of range"))?;
                let weekly = regenerate_rollup(
                    transaction,
                    MemoryPeriodKind::Weekly,
                    weekly_start,
                    weekly_end,
                    &regenerated_at,
                )?;
                let monthly_start = NaiveDate::from_ymd_opt(as_of.year(), as_of.month(), 1)
                    .ok_or_else(|| invalid("monthly summary date is invalid"))?;
                let next_month = if as_of.month() == 12 {
                    NaiveDate::from_ymd_opt(as_of.year() + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(as_of.year(), as_of.month() + 1, 1)
                }
                .ok_or_else(|| invalid("monthly summary date is out of range"))?;
                let monthly_end = next_month
                    .pred_opt()
                    .ok_or_else(|| invalid("monthly summary date is out of range"))?;
                let monthly = regenerate_rollup(
                    transaction,
                    MemoryPeriodKind::Monthly,
                    monthly_start,
                    monthly_end,
                    &regenerated_at,
                )?;
                Ok(MemorySummaryRegenerationReport {
                    daily,
                    weekly,
                    monthly,
                    regenerated_at,
                    openai_calls: 0,
                })
            })
    }
}

fn regenerate_rollup(
    transaction: &Transaction<'_>,
    period_kind: MemoryPeriodKind,
    period_start: NaiveDate,
    period_end: NaiveDate,
    now: &str,
) -> Result<Option<String>> {
    let source_period_kind = match period_kind {
        MemoryPeriodKind::Daily => None,
        MemoryPeriodKind::Weekly => Some(MemoryPeriodKind::Daily),
        MemoryPeriodKind::Monthly => Some(MemoryPeriodKind::Weekly),
    };
    let mut statement = if source_period_kind.is_some() {
        transaction.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM assistant_memories
             WHERE deleted_at IS NULL AND kind = 'summary' AND period_kind = ?1
               AND period_start >= ?2 AND period_start <= ?3
             ORDER BY period_start, id"
        ))?
    } else {
        transaction.prepare(&format!(
            "SELECT {MEMORY_COLUMNS} FROM assistant_memories
             WHERE deleted_at IS NULL AND kind <> 'summary'
               AND substr(updated_at, 1, 10) >= ?2 AND substr(updated_at, 1, 10) <= ?3
             ORDER BY updated_at, id"
        ))?
    };
    let source_kind = source_period_kind.map(MemoryPeriodKind::as_str);
    let sources = statement
        .query_map(
            params![
                source_kind,
                period_start.format("%Y-%m-%d").to_string(),
                period_end.format("%Y-%m-%d").to_string()
            ],
            map_memory,
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let summary_key = format!(
        "{}:{}",
        period_kind.as_str(),
        period_start.format("%Y-%m-%d")
    );
    let existing_id = transaction
        .query_row(
            "SELECT id FROM assistant_memories WHERE summary_key = ?1",
            [&summary_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    if sources.is_empty() {
        if let Some(memory_id) = existing_id {
            soft_delete_memory(
                transaction,
                &memory_id,
                "system",
                "memory-rollup-empty",
                "source_deleted",
                now,
            )?;
        }
        return Ok(None);
    }

    let title = format!(
        "{} memory summary: {} to {}",
        period_kind.as_str(),
        period_start,
        period_end
    );
    let body = render_rollup_body(&sources);
    let sensitivity = rollup_sensitivity(&sources);
    let openai_allowed = sources
        .iter()
        .all(|source| source.openai_allowed && source.sensitivity == MemorySensitivity::Normal);
    let (retention, retention_days) = match period_kind {
        MemoryPeriodKind::Daily => (MemoryRetention::Daily90d, 90),
        MemoryPeriodKind::Weekly => (MemoryRetention::Weekly365d, 365),
        MemoryPeriodKind::Monthly => (MemoryRetention::Monthly1095d, 1_095),
    };
    let expires_on = period_end
        .checked_add_days(Days::new(retention_days))
        .ok_or_else(|| invalid("summary retention date is out of range"))?;
    let expires_at = format!("{}T00:00:00.000Z", expires_on.format("%Y-%m-%d"));
    let provenance = json!({
        "generator": "tm-local-deterministic-rollup-v1",
        "sourceCount": sources.len(),
        "sourceHashes": sources.iter().map(|source| &source.content_sha256).collect::<Vec<_>>()
    });
    let provenance_json = serde_json::to_string(&provenance)?;
    let content_sha256 = memory_content_hash(
        MemoryKind::Summary,
        &title,
        &body,
        sensitivity,
        openai_allowed,
        retention,
    );
    let memory_id = if let Some(memory_id) = existing_id {
        let current = query_memory(transaction, &memory_id)?;
        if current.content_sha256 != content_sha256 || current.deleted_at.is_some() {
            transaction.execute(
                "UPDATE assistant_memories SET title = ?1, body = ?2,
                    provenance_json = ?3, sensitivity = ?4, openai_allowed = ?5,
                    retention = ?6, expires_at = ?7, period_start = ?8, period_end = ?9,
                    revision = revision + 1, content_sha256 = ?10, updated_at = ?11,
                    deleted_at = NULL WHERE id = ?12",
                params![
                    title,
                    body,
                    provenance_json,
                    sensitivity.as_str(),
                    i64::from(openai_allowed),
                    retention.as_str(),
                    expires_at,
                    period_start.format("%Y-%m-%d").to_string(),
                    period_end.format("%Y-%m-%d").to_string(),
                    content_sha256,
                    now,
                    memory_id,
                ],
            )?;
            let updated = query_memory(transaction, &memory_id)?;
            insert_memory_event(
                transaction,
                &memory_id,
                "regenerated",
                updated.revision,
                "system",
                "memory-rollup-regenerate",
                &content_sha256,
                Some(&json!({"periodKind": period_kind})),
                now,
            )?;
        }
        memory_id
    } else {
        let memory_id = new_id();
        transaction.execute(
            "INSERT INTO assistant_memories(
                id, kind, title, body, source_type, source_id, provenance_json,
                sensitivity, openai_allowed, retention, expires_at, period_kind,
                period_start, period_end, summary_key, revision, content_sha256,
                created_at, updated_at, deleted_at
             ) VALUES (?1, 'summary', ?2, ?3, 'memory_rollup', ?4, ?5, ?6, ?7,
                       ?8, ?9, ?10, ?11, ?12, ?4, 1, ?13, ?14, ?14, NULL)",
            params![
                memory_id,
                title,
                body,
                summary_key,
                provenance_json,
                sensitivity.as_str(),
                i64::from(openai_allowed),
                retention.as_str(),
                expires_at,
                period_kind.as_str(),
                period_start.format("%Y-%m-%d").to_string(),
                period_end.format("%Y-%m-%d").to_string(),
                content_sha256,
                now,
            ],
        )?;
        insert_memory_event(
            transaction,
            &memory_id,
            "created",
            1,
            "system",
            "memory-rollup-create",
            &content_sha256,
            Some(&json!({"periodKind": period_kind})),
            now,
        )?;
        memory_id
    };
    transaction.execute(
        "DELETE FROM assistant_memory_sources WHERE memory_id = ?1",
        [&memory_id],
    )?;
    for source in sources {
        transaction.execute(
            "INSERT INTO assistant_memory_sources(
                memory_id, source_type, source_id, source_revision, source_updated_at, created_at
             ) VALUES (?1, 'memory', ?2, ?3, ?4, ?5)",
            params![
                memory_id,
                source.id,
                as_i64(source.revision, "source memory revision")?,
                source.updated_at,
                now,
            ],
        )?;
    }
    Ok(Some(memory_id))
}

fn render_rollup_body(sources: &[AssistantMemory]) -> String {
    let mut body = format!(
        "Deterministic local summary of {} memories.\n",
        sources.len()
    );
    for source in sources.iter().take(50) {
        let line = format!("- [{}] {}\n", source.kind.as_str(), source.title.trim());
        if body.len().saturating_add(line.len()) > MAX_MEMORY_BODY_CHARS {
            break;
        }
        body.push_str(&line);
    }
    body.trim().to_owned()
}

fn rollup_sensitivity(sources: &[AssistantMemory]) -> MemorySensitivity {
    if sources
        .iter()
        .any(|source| source.sensitivity == MemorySensitivity::Restricted)
    {
        MemorySensitivity::Restricted
    } else if sources
        .iter()
        .any(|source| source.sensitivity == MemorySensitivity::Private)
    {
        MemorySensitivity::Private
    } else {
        MemorySensitivity::Normal
    }
}

pub(crate) fn create_memory_in_transaction(
    transaction: &Transaction<'_>,
    input: &CreateMemoryInput,
    actor: &str,
    request_id: &str,
) -> Result<AssistantMemory> {
    validate_create_memory(input)?;
    let id = new_id();
    let now = now_utc();
    let title = input.title.trim();
    let body = input.body.trim();
    let content_sha256 = memory_content_hash(
        input.kind,
        title,
        body,
        input.sensitivity,
        input.openai_allowed,
        input.retention,
    );
    transaction.execute(
        "INSERT INTO assistant_memories(
            id, kind, title, body, source_type, source_id, provenance_json,
            sensitivity, openai_allowed, retention, expires_at, period_kind,
            period_start, period_end, summary_key, revision, content_sha256,
            created_at, updated_at, deleted_at
         ) VALUES (?1, ?2, ?3, ?4, 'explicit', NULL, ?5, ?6, ?7, ?8,
                   NULL, NULL, NULL, NULL, NULL, 1, ?9, ?10, ?10, NULL)",
        params![
            id,
            input.kind.as_str(),
            title,
            body,
            serde_json::to_string(&json!({"origin": "explicit_user_request"}))?,
            input.sensitivity.as_str(),
            i64::from(input.openai_allowed),
            input.retention.as_str(),
            content_sha256,
            now,
        ],
    )?;
    insert_memory_event(
        transaction,
        &id,
        "created",
        1,
        actor,
        request_id,
        &content_sha256,
        None,
        &now,
    )?;
    query_memory(transaction, &id)
}

pub(crate) fn update_memory_in_transaction(
    transaction: &Transaction<'_>,
    memory_id: &str,
    expected_revision: u64,
    patch: &MemoryPatch,
    actor: &str,
    request_id: &str,
) -> Result<AssistantMemory> {
    validate_uuid("memory ID", memory_id)?;
    validate_memory_fields(
        patch.kind,
        &patch.title,
        &patch.body,
        patch.sensitivity,
        patch.openai_allowed,
        patch.retention,
    )?;
    let current = query_memory(transaction, memory_id)?;
    require_active_revision(&current, expected_revision)?;
    if current.kind == MemoryKind::Summary || current.source_type != MemorySourceType::Explicit {
        return Err(Error::Conflict(
            "derived memories must be regenerated, not edited".to_owned(),
        ));
    }
    if current.kind == patch.kind
        && current.title.trim() == patch.title.trim()
        && current.body.trim() == patch.body.trim()
        && current.sensitivity == patch.sensitivity
        && current.openai_allowed == patch.openai_allowed
        && current.retention == patch.retention
    {
        return Err(invalid("memory update must change at least one field"));
    }
    let title = patch.title.trim();
    let body = patch.body.trim();
    let content_sha256 = memory_content_hash(
        patch.kind,
        title,
        body,
        patch.sensitivity,
        patch.openai_allowed,
        patch.retention,
    );
    let now = now_utc();
    let changed = transaction.execute(
        "UPDATE assistant_memories SET
            kind = ?1, title = ?2, body = ?3, sensitivity = ?4,
            openai_allowed = ?5, retention = ?6, content_sha256 = ?7,
            revision = revision + 1, updated_at = ?8
         WHERE id = ?9 AND revision = ?10 AND deleted_at IS NULL",
        params![
            patch.kind.as_str(),
            title,
            body,
            patch.sensitivity.as_str(),
            i64::from(patch.openai_allowed),
            patch.retention.as_str(),
            content_sha256,
            now,
            memory_id,
            as_i64(expected_revision, "memory revision")?,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict("memory changed concurrently".to_owned()));
    }
    insert_memory_event(
        transaction,
        memory_id,
        "updated",
        expected_revision + 1,
        actor,
        request_id,
        &content_sha256,
        None,
        &now,
    )?;
    query_memory(transaction, memory_id)
}

pub(crate) fn delete_memory_in_transaction(
    transaction: &Transaction<'_>,
    memory_id: &str,
    expected_revision: u64,
    actor: &str,
    request_id: &str,
) -> Result<AssistantMemory> {
    validate_uuid("memory ID", memory_id)?;
    let current = query_memory(transaction, memory_id)?;
    require_active_revision(&current, expected_revision)?;
    let now = now_utc();
    let changed = transaction.execute(
        "UPDATE assistant_memories SET deleted_at = ?1, updated_at = ?1,
            revision = revision + 1
         WHERE id = ?2 AND revision = ?3 AND deleted_at IS NULL",
        params![
            now,
            memory_id,
            as_i64(expected_revision, "memory revision")?
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict("memory changed concurrently".to_owned()));
    }
    insert_memory_event(
        transaction,
        memory_id,
        "deleted",
        expected_revision + 1,
        actor,
        request_id,
        &current.content_sha256,
        None,
        &now,
    )?;
    query_memory(transaction, memory_id)
}

pub(crate) fn query_memory(connection: &Connection, memory_id: &str) -> Result<AssistantMemory> {
    connection
        .query_row(
            &format!("SELECT {MEMORY_COLUMNS} FROM assistant_memories WHERE id = ?1"),
            [memory_id],
            map_memory,
        )
        .optional()?
        .ok_or_else(|| not_found("assistant memory", memory_id))
}

pub(crate) fn validate_create_memory(input: &CreateMemoryInput) -> Result<()> {
    validate_memory_fields(
        input.kind,
        &input.title,
        &input.body,
        input.sensitivity,
        input.openai_allowed,
        input.retention,
    )?;
    if input.kind == MemoryKind::Summary {
        return Err(invalid("summary memories can only be generated by TM"));
    }
    Ok(())
}

pub(crate) fn validate_memory_patch(patch: &MemoryPatch) -> Result<()> {
    validate_memory_fields(
        patch.kind,
        &patch.title,
        &patch.body,
        patch.sensitivity,
        patch.openai_allowed,
        patch.retention,
    )?;
    if patch.kind == MemoryKind::Summary {
        return Err(invalid("summary memories can only be regenerated by TM"));
    }
    Ok(())
}

fn validate_memory_fields(
    kind: MemoryKind,
    title: &str,
    body: &str,
    sensitivity: MemorySensitivity,
    openai_allowed: bool,
    retention: MemoryRetention,
) -> Result<()> {
    let title = title.trim();
    let body = body.trim();
    if title.is_empty() || title.chars().count() > MAX_MEMORY_TITLE_CHARS {
        return Err(invalid(format!(
            "memory title must contain 1-{MAX_MEMORY_TITLE_CHARS} characters"
        )));
    }
    if body.is_empty() || body.chars().count() > MAX_MEMORY_BODY_CHARS {
        return Err(invalid(format!(
            "memory body must contain 1-{MAX_MEMORY_BODY_CHARS} characters"
        )));
    }
    if openai_allowed && sensitivity != MemorySensitivity::Normal {
        return Err(invalid(
            "only normal-sensitivity memories may be sent to OpenAI",
        ));
    }
    if kind != MemoryKind::Summary && retention != MemoryRetention::UntilDeleted {
        return Err(invalid(
            "explicit memories must use the until_deleted retention policy",
        ));
    }
    reject_prohibited_secret_material(title)?;
    reject_prohibited_secret_material(body)?;
    Ok(())
}

fn reject_prohibited_secret_material(value: &str) -> Result<()> {
    let lower = value.to_lowercase();
    let forbidden = [
        "password",
        "비밀번호",
        "api key",
        "api_key",
        "recovery code",
        "복구 코드",
        "bearer ",
        "begin private key",
        "주민등록번호",
        "카드번호",
        "계좌번호",
    ];
    if forbidden.iter().any(|needle| lower.contains(needle))
        || contains_long_digit_sequence(value, 12)
    {
        return Err(invalid(
            "memory contains prohibited credentials or full financial/government identifiers",
        ));
    }
    Ok(())
}

fn contains_long_digit_sequence(value: &str, minimum: usize) -> bool {
    let mut run = 0;
    for character in value.chars() {
        if character.is_ascii_digit() {
            run += 1;
            if run >= minimum {
                return true;
            }
        } else if !matches!(character, '-' | ' ') {
            run = 0;
        }
    }
    false
}

fn validate_search_filter(filter: &MemorySearchFilter) -> Result<()> {
    let query = filter.query.trim();
    if query.is_empty() || query.chars().count() > MAX_MEMORY_SEARCH_QUERY_CHARS {
        return Err(invalid(format!(
            "memory search query must contain 1-{MAX_MEMORY_SEARCH_QUERY_CHARS} characters"
        )));
    }
    if !(1..=MEMORY_CONTEXT_MAX_ITEMS).contains(&filter.max_items) {
        return Err(invalid(format!(
            "memory search maxItems must be 1-{MEMORY_CONTEXT_MAX_ITEMS}"
        )));
    }
    if !(256..=MEMORY_CONTEXT_MAX_BYTES).contains(&filter.max_bytes) {
        return Err(invalid(format!(
            "memory search maxBytes must be 256-{MEMORY_CONTEXT_MAX_BYTES}"
        )));
    }
    Ok(())
}

fn memory_fts_query(query: &str) -> Result<String> {
    let terms = query
        .split_whitespace()
        .map(|term| {
            term.chars()
                .filter(|character| character.is_alphanumeric() || *character == '_')
                .collect::<String>()
        })
        .filter(|term| !term.is_empty())
        .take(16)
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(invalid("memory search query has no searchable terms"));
    }
    Ok(terms.join(" OR "))
}

fn apply_context_budget(
    candidates: Vec<AssistantMemory>,
    max_items: usize,
    max_bytes: usize,
) -> MemorySearchResult {
    let total = candidates.len();
    let mut items = Vec::new();
    let mut bytes_used: usize = 0;
    for memory in candidates {
        if items.len() >= max_items {
            break;
        }
        let item_bytes = memory
            .title
            .len()
            .saturating_add(memory.body.len())
            .saturating_add(96);
        if bytes_used.saturating_add(item_bytes) > max_bytes {
            continue;
        }
        bytes_used += item_bytes;
        items.push(memory);
    }
    MemorySearchResult {
        omitted: total.saturating_sub(items.len()),
        items,
        bytes_used,
        retrieval: "sqlite_fts5_structured_filters",
        vector_service_used: false,
    }
}

fn invalid_source_memory_ids(transaction: &Transaction<'_>) -> Result<Vec<String>> {
    let mut ids = HashSet::new();
    for (source_type, table) in [
        ("task", "tasks"),
        ("note", "notes"),
        ("worklog", "worklogs"),
        ("session", "work_sessions"),
    ] {
        let sql = format!(
            "SELECT source.memory_id
             FROM assistant_memory_sources AS source
             LEFT JOIN {table} AS entity ON entity.id = source.source_id
             WHERE source.source_type = ?1
               AND (entity.id IS NULL OR entity.deleted_at IS NOT NULL)"
        );
        let mut statement = transaction.prepare(&sql)?;
        let rows = statement.query_map([source_type], |row| row.get::<_, String>(0))?;
        for row in rows {
            ids.insert(row?);
        }
    }
    let mut statement = transaction.prepare(
        "SELECT source.memory_id
         FROM assistant_memory_sources AS source
         LEFT JOIN assistant_memories AS parent ON parent.id = source.source_id
         WHERE source.source_type = 'memory'
           AND (parent.id IS NULL OR parent.deleted_at IS NOT NULL)",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    for row in rows {
        ids.insert(row?);
    }
    Ok(ids.into_iter().collect())
}

fn query_ids(transaction: &Transaction<'_>, sql: &str, value: &str) -> Result<Vec<String>> {
    let mut statement = transaction.prepare(sql)?;
    let rows = statement.query_map([value], |row| row.get::<_, String>(0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn soft_delete_memory(
    transaction: &Transaction<'_>,
    memory_id: &str,
    actor: &str,
    request_id: &str,
    event_type: &str,
    now: &str,
) -> Result<bool> {
    let memory = query_memory(transaction, memory_id)?;
    if memory.deleted_at.is_some() {
        return Ok(false);
    }
    let changed = transaction.execute(
        "UPDATE assistant_memories SET deleted_at = ?1, updated_at = ?1,
            revision = revision + 1 WHERE id = ?2 AND deleted_at IS NULL",
        params![now, memory_id],
    )?;
    if changed == 1 {
        insert_memory_event(
            transaction,
            memory_id,
            event_type,
            memory.revision + 1,
            actor,
            request_id,
            &memory.content_sha256,
            None,
            now,
        )?;
    }
    Ok(changed == 1)
}

fn require_active_revision(memory: &AssistantMemory, expected_revision: u64) -> Result<()> {
    if memory.deleted_at.is_some() {
        return Err(Error::Conflict("memory is deleted".to_owned()));
    }
    if expected_revision == 0 || memory.revision != expected_revision {
        return Err(Error::Conflict(format!(
            "memory revision conflict: expected {expected_revision}, found {}",
            memory.revision
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_memory_event(
    transaction: &Transaction<'_>,
    memory_id: &str,
    event_type: &str,
    revision: u64,
    actor: &str,
    request_id: &str,
    content_sha256: &str,
    metadata: Option<&Value>,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO assistant_memory_events(
            id, memory_id, event_type, revision, actor, request_id,
            content_sha256, metadata_json, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            new_id(),
            memory_id,
            event_type,
            as_i64(revision, "memory revision")?,
            actor,
            request_id,
            content_sha256,
            metadata.map(serde_json::to_string).transpose()?,
            created_at,
        ],
    )?;
    Ok(())
}

fn memory_content_hash(
    kind: MemoryKind,
    title: &str,
    body: &str,
    sensitivity: MemorySensitivity,
    openai_allowed: bool,
    retention: MemoryRetention,
) -> String {
    let value = json!({
        "kind": kind,
        "title": title,
        "body": body,
        "sensitivity": sensitivity,
        "openaiAllowed": openai_allowed,
        "retention": retention,
    });
    format!("{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn map_memory(row: &Row<'_>) -> rusqlite::Result<AssistantMemory> {
    let provenance_json = row.get::<_, String>(6)?;
    Ok(AssistantMemory {
        id: row.get(0)?,
        kind: parse_enum(row, 1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        source_type: parse_enum(row, 4)?,
        source_id: row.get(5)?,
        provenance: serde_json::from_str(&provenance_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
        })?,
        sensitivity: parse_enum(row, 7)?,
        openai_allowed: row.get(8)?,
        retention: parse_enum(row, 9)?,
        expires_at: row.get(10)?,
        period_kind: parse_optional_enum(row, 11)?,
        period_start: parse_optional_date(row, 12)?,
        period_end: parse_optional_date(row, 13)?,
        summary_key: row.get(14)?,
        revision: as_u64_from_row(row, 15, "memory revision")?,
        content_sha256: row.get(16)?,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        deleted_at: row.get(19)?,
    })
}

fn map_memory_event(row: &Row<'_>) -> rusqlite::Result<AssistantMemoryEvent> {
    let revision = as_u64_from_row(row, 3, "memory event revision")?;
    let metadata = row
        .get::<_, Option<String>>(7)?
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(7, Type::Text, Box::new(error))
        })?;
    Ok(AssistantMemoryEvent {
        id: row.get(0)?,
        memory_id: row.get(1)?,
        event_type: row.get(2)?,
        revision,
        actor: row.get(4)?,
        request_id: row.get(5)?,
        content_sha256: row.get(6)?,
        metadata,
        created_at: row.get(8)?,
    })
}

fn parse_enum<T: FromStr<Err = Error>>(row: &Row<'_>, index: usize) -> rusqlite::Result<T> {
    let value = row.get::<_, String>(index)?;
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn parse_optional_enum<T: FromStr<Err = Error>>(
    row: &Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            value.parse().map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

fn parse_optional_date(row: &Row<'_>, index: usize) -> rusqlite::Result<Option<NaiveDate>> {
    row.get::<_, Option<String>>(index)?
        .map(|value| {
            NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

fn as_i64(value: u64, field: &str) -> Result<i64> {
    i64::try_from(value).map_err(|_| invalid(format!("{field} is too large")))
}

fn as_u64_from_row(row: &Row<'_>, index: usize, field: &str) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value).map_err(|_| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            Type::Integer,
            invalid(format!("{field} must be non-negative")).into(),
        )
    })
}

fn validate_uuid(field: &str, value: &str) -> Result<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid(format!("{field} must be a UUID")))
}

const MEMORY_COLUMNS: &str = "id, kind, title, body, source_type, source_id,
    provenance_json, sensitivity, openai_allowed, retention, expires_at,
    period_kind, period_start, period_end, summary_key, revision, content_sha256,
    created_at, updated_at, deleted_at";

const MEMORY_COLUMNS_PREFIXED: &str = "memory.id, memory.kind, memory.title, memory.body,
    memory.source_type, memory.source_id, memory.provenance_json, memory.sensitivity,
    memory.openai_allowed, memory.retention, memory.expires_at, memory.period_kind,
    memory.period_start, memory.period_end, memory.summary_key, memory.revision,
    memory.content_sha256, memory.created_at, memory.updated_at, memory.deleted_at";
