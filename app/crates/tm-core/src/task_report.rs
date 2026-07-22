use chrono::NaiveDate;
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error, Result,
    database::{Database, new_id, now_utc},
    error::{invalid, not_found},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskReportRun {
    pub id: String,
    pub report_date: NaiveDate,
    pub actor: String,
    pub status: String,
    pub candidate_count: u32,
    pub prompt_version: String,
    pub model: String,
    pub response_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub result: Option<Value>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_microusd: u64,
    pub latency_ms: Option<u64>,
    pub failure_code: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub helpful: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct TaskReportCompletion {
    pub status: &'static str,
    pub response_id: Option<String>,
    pub upstream_request_id: Option<String>,
    pub result: Option<Value>,
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub estimated_cost_microusd: u64,
    pub latency_ms: u64,
    pub failure_code: Option<String>,
}

pub(crate) fn begin(
    database: &Database,
    id: &str,
    date: NaiveDate,
    actor: &str,
    candidate_count: usize,
    prompt_version: &str,
    model: &str,
    daily_limit: u32,
) -> Result<TaskReportRun> {
    validate_text("task report ID", id, 128)?;
    validate_text("task report actor", actor, 256)?;
    validate_text("task report prompt version", prompt_version, 64)?;
    validate_text("task report model", model, 128)?;
    if candidate_count == 0 || candidate_count > 20 || daily_limit == 0 {
        return Err(invalid("task report limits are invalid"));
    }
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let attempts: u32 = transaction.query_row(
            "SELECT count(*) FROM task_report_runs
             WHERE report_date = ?1 AND status <> 'no_tasks'",
            [date],
            |row| row.get(0),
        )?;
        if attempts >= daily_limit {
            return Err(Error::AiDailyLimitExceeded {
                operation: "task_report".to_owned(),
                limit: daily_limit,
            });
        }
        transaction.execute(
            "INSERT INTO task_report_runs(
                id, report_date, actor, status, candidate_count, prompt_version,
                model, estimated_cost_microusd, created_at
             ) VALUES (?1, ?2, ?3, 'started', ?4, ?5, ?6, 0, ?7)",
            params![
                id,
                date,
                actor,
                candidate_count as u32,
                prompt_version,
                model,
                now_utc()
            ],
        )?;
        query_run(transaction, id)
    })
}

pub(crate) fn record_no_tasks(
    database: &Database,
    id: &str,
    date: NaiveDate,
    actor: &str,
    prompt_version: &str,
    model: &str,
    result: &Value,
) -> Result<TaskReportRun> {
    validate_text("task report ID", id, 128)?;
    validate_text("task report actor", actor, 256)?;
    let result_json = serde_json::to_string(result)?;
    let now = now_utc();
    let connection = database.connect()?;
    connection.execute(
        "INSERT INTO task_report_runs(
            id, report_date, actor, status, candidate_count, prompt_version,
            model, result_json, estimated_cost_microusd, latency_ms, created_at, completed_at
         ) VALUES (?1, ?2, ?3, 'no_tasks', 0, ?4, ?5, ?6, 0, 0, ?7, ?7)",
        params![id, date, actor, prompt_version, model, result_json, now],
    )?;
    query_run(&connection, id)
}

pub(crate) fn complete(
    database: &Database,
    id: &str,
    completion: &TaskReportCompletion,
) -> Result<TaskReportRun> {
    if !matches!(completion.status, "succeeded" | "failed") {
        return Err(invalid("task report completion status is invalid"));
    }
    if completion.status == "succeeded" && completion.result.is_none() {
        return Err(invalid("successful task report requires a result"));
    }
    if completion.status == "failed" && completion.failure_code.is_none() {
        return Err(invalid("failed task report requires a failure code"));
    }
    let result_json = completion
        .result
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let connection = database.connect()?;
    let changed = connection.execute(
        "UPDATE task_report_runs
         SET status = ?2, response_id = ?3, upstream_request_id = ?4,
             result_json = ?5, input_tokens = ?6, cached_input_tokens = ?7,
             output_tokens = ?8, total_tokens = ?9, estimated_cost_microusd = ?10,
             latency_ms = ?11, failure_code = ?12, completed_at = ?13
         WHERE id = ?1 AND status = 'started'",
        params![
            id,
            completion.status,
            completion.response_id,
            completion.upstream_request_id,
            result_json,
            completion.input_tokens,
            completion.cached_input_tokens,
            completion.output_tokens,
            completion.total_tokens,
            completion.estimated_cost_microusd,
            completion.latency_ms,
            completion.failure_code,
            now_utc(),
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "task report {id} was not in started state"
        )));
    }
    query_run(&connection, id)
}

pub(crate) fn latest(database: &Database) -> Result<Option<TaskReportRun>> {
    let connection = database.connect()?;
    connection
        .query_row(
            &format!(
                "{} WHERE r.status IN ('succeeded', 'no_tasks')
                 ORDER BY r.report_date DESC, r.created_at DESC, r.id DESC LIMIT 1",
                RUN_SELECT
            ),
            [],
            map_run,
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn feedback(
    database: &Database,
    run_id: &str,
    helpful: bool,
    actor: &str,
) -> Result<TaskReportRun> {
    validate_text("task report actor", actor, 256)?;
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let run = query_run(transaction, run_id)?;
        if !matches!(run.status.as_str(), "succeeded" | "no_tasks") {
            return Err(Error::Conflict(
                "only a completed task report can be rated".to_owned(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO task_report_feedback(id, run_id, helpful, actor, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![new_id(), run_id, i64::from(helpful), actor, now_utc()],
            )
            .map_err(|error| {
                if matches!(
                    error.sqlite_error_code(),
                    Some(rusqlite::ErrorCode::ConstraintViolation)
                ) {
                    Error::Conflict("task report feedback was already recorded".to_owned())
                } else {
                    Error::Database(error)
                }
            })?;
        query_run(transaction, run_id)
    })
}

const RUN_SELECT: &str = "SELECT r.id, r.report_date, r.actor, r.status, r.candidate_count,
            r.prompt_version, r.model, r.response_id, r.upstream_request_id,
            r.result_json, r.input_tokens, r.cached_input_tokens, r.output_tokens,
            r.total_tokens, r.estimated_cost_microusd, r.latency_ms, r.failure_code,
            r.created_at, r.completed_at, f.helpful
     FROM task_report_runs r LEFT JOIN task_report_feedback f ON f.run_id = r.id";

fn query_run(connection: &rusqlite::Connection, id: &str) -> Result<TaskReportRun> {
    connection
        .query_row(&format!("{RUN_SELECT} WHERE r.id = ?1"), [id], map_run)
        .optional()?
        .ok_or_else(|| not_found("task report", id))
}

fn map_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskReportRun> {
    let result_json: Option<String> = row.get(9)?;
    let helpful: Option<i64> = row.get(19)?;
    Ok(TaskReportRun {
        id: row.get(0)?,
        report_date: row.get(1)?,
        actor: row.get(2)?,
        status: row.get(3)?,
        candidate_count: row.get(4)?,
        prompt_version: row.get(5)?,
        model: row.get(6)?,
        response_id: row.get(7)?,
        upstream_request_id: row.get(8)?,
        result: result_json
            .map(|value| serde_json::from_str(&value))
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    9,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        input_tokens: row.get(10)?,
        cached_input_tokens: row.get(11)?,
        output_tokens: row.get(12)?,
        total_tokens: row.get(13)?,
        estimated_cost_microusd: row.get(14)?,
        latency_ms: row.get(15)?,
        failure_code: row.get(16)?,
        created_at: row.get(17)?,
        completed_at: row.get(18)?,
        helpful: helpful.map(|value| value != 0),
    })
}

fn validate_text(field: &str, value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum || value.contains(['\r', '\n']) {
        return Err(invalid(format!("{field} has an invalid format")));
    }
    Ok(())
}
