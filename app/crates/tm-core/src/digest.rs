use std::{fmt, str::FromStr};

use chrono::{Duration, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    database::{Database, now_utc, seoul_day_bounds},
    error::not_found,
};

const CLAIM_LEASE_MINUTES: i64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DigestKind {
    Morning,
    Evening,
}

impl DigestKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Morning => "morning",
            Self::Evening => "evening",
        }
    }
}

impl fmt::Display for DigestKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for DigestKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "morning" => Ok(Self::Morning),
            "evening" => Ok(Self::Evening),
            other => Err(Error::InvalidInput(format!(
                "digest kind must be morning or evening, got {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestTaskFact {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub status: String,
    pub priority: u8,
    pub due_date: Option<NaiveDate>,
    pub day_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestSessionFact {
    pub id: String,
    pub goal: String,
    pub result: String,
    pub blockers: String,
    pub next_action: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestWorkLogFact {
    pub id: String,
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestFacts {
    pub timezone: String,
    pub generated_at: String,
    pub yesterday_unfinished: Vec<DigestTaskFact>,
    pub today_planned: Vec<DigestTaskFact>,
    pub due_soon: Vec<DigestTaskFact>,
    pub blocked_tasks: Vec<DigestTaskFact>,
    pub recommended_priorities: Vec<DigestTaskFact>,
    pub today_completed: Vec<DigestTaskFact>,
    pub session_results: Vec<DigestSessionFact>,
    pub worklogs: Vec<DigestWorkLogFact>,
    pub unfinished: Vec<DigestTaskFact>,
    pub blocker_notes: Vec<String>,
    pub tomorrow_candidates: Vec<DigestTaskFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestDelivery {
    pub delivery_key: String,
    pub kind: DigestKind,
    pub date: NaiveDate,
    pub status: String,
    pub attempt_count: u32,
    pub claimed_at: String,
    pub claim_expires_at: String,
    pub sent_at: Option<String>,
    pub slack_ref: Option<String>,
    pub failed_at: Option<String>,
    pub failure_reason: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DigestPreparation {
    pub delivery_key: String,
    pub should_send: bool,
    pub kind: DigestKind,
    pub date: NaiveDate,
    pub status: String,
    pub facts: DigestFacts,
}

pub(crate) fn prepare(
    database: &Database,
    kind: DigestKind,
    date: NaiveDate,
) -> Result<DigestPreparation> {
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let delivery_key = format!("tm:{}:{date}", kind.as_str());
        let now = now_utc();
        let expires = (Utc::now() + Duration::minutes(CLAIM_LEASE_MINUTES))
            .to_rfc3339_opts(SecondsFormat::Millis, true);
        let inserted = transaction.execute(
            "INSERT INTO digest_deliveries(
                delivery_key, kind, digest_date, status, attempt_count,
                claimed_at, claim_expires_at, updated_at
             ) VALUES (?1, ?2, ?3, 'claimed', 1, ?4, ?5, ?4)
             ON CONFLICT(delivery_key) DO NOTHING",
            params![delivery_key, kind.as_str(), date, now, expires],
        )?;
        let reclaimed = if inserted == 0 {
            transaction.execute(
                "UPDATE digest_deliveries
                 SET status = 'claimed',
                     attempt_count = attempt_count + 1,
                     claimed_at = ?2,
                     claim_expires_at = ?3,
                     failed_at = NULL,
                     failure_reason = NULL,
                     updated_at = ?2
                 WHERE delivery_key = ?1
                   AND status = 'failed'",
                params![delivery_key, now, expires],
            )?
        } else {
            0
        };
        let delivery = query_delivery(transaction, &delivery_key)?;
        let facts = collect_facts(transaction, kind, date)?;
        Ok(DigestPreparation {
            delivery_key,
            should_send: inserted == 1 || reclaimed == 1,
            kind,
            date,
            status: delivery.status,
            facts,
        })
    })
}

pub(crate) fn preview(
    database: &Database,
    kind: DigestKind,
    date: NaiveDate,
) -> Result<DigestFacts> {
    let connection = database.connect()?;
    collect_facts(&connection, kind, date)
}

pub(crate) fn complete(
    database: &Database,
    delivery_key: &str,
    slack_ref: &str,
) -> Result<DigestDelivery> {
    let slack_ref = slack_ref.trim();
    if slack_ref.is_empty() {
        return Err(Error::InvalidInput(
            "Slack reference cannot be empty".to_owned(),
        ));
    }
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let existing = query_delivery(transaction, delivery_key)?;
        if existing.status == "sent" {
            return Ok(existing);
        }
        if existing.status != "claimed" {
            return Err(Error::Conflict(format!(
                "delivery {delivery_key} is {}, not claimed",
                existing.status
            )));
        }
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE digest_deliveries
             SET status = 'sent', sent_at = ?2, slack_ref = ?3,
                 failed_at = NULL, failure_reason = NULL, updated_at = ?2
             WHERE delivery_key = ?1 AND status = 'claimed'",
            params![delivery_key, now, slack_ref],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "delivery {delivery_key} was not in claimed state"
            )));
        }
        query_delivery(transaction, delivery_key)
    })
}

pub(crate) fn fail(
    database: &Database,
    delivery_key: &str,
    reason: &str,
) -> Result<DigestDelivery> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(Error::InvalidInput(
            "failure reason cannot be empty".to_owned(),
        ));
    }
    database.transaction(TransactionBehavior::Immediate, |transaction| {
        let existing = query_delivery(transaction, delivery_key)?;
        if existing.status == "sent" {
            return Err(Error::Conflict(format!(
                "sent delivery {delivery_key} can never be failed or reclaimed"
            )));
        }
        if existing.status == "failed" {
            return Ok(existing);
        }
        let now = now_utc();
        let changed = transaction.execute(
            "UPDATE digest_deliveries
             SET status = 'failed', failed_at = ?2, failure_reason = ?3, updated_at = ?2
             WHERE delivery_key = ?1 AND status = 'claimed'",
            params![delivery_key, now, reason],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "delivery {delivery_key} was not in claimed state"
            )));
        }
        query_delivery(transaction, delivery_key)
    })
}

pub(crate) fn status(database: &Database) -> Result<Vec<DigestDelivery>> {
    let connection = database.connect()?;
    let mut statement = connection.prepare(
        "SELECT delivery_key, kind, digest_date, status, attempt_count,
                claimed_at, claim_expires_at, sent_at, slack_ref,
                failed_at, failure_reason, updated_at
         FROM digest_deliveries
         ORDER BY digest_date DESC, kind ASC",
    )?;
    let rows = statement.query_map([], map_delivery)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_delivery(transaction: &Transaction<'_>, key: &str) -> Result<DigestDelivery> {
    transaction
        .query_row(
            "SELECT delivery_key, kind, digest_date, status, attempt_count,
                    claimed_at, claim_expires_at, sent_at, slack_ref,
                    failed_at, failure_reason, updated_at
             FROM digest_deliveries WHERE delivery_key = ?1",
            [key],
            map_delivery,
        )
        .optional()?
        .ok_or_else(|| not_found("digest delivery", key))
}

fn map_delivery(row: &rusqlite::Row<'_>) -> rusqlite::Result<DigestDelivery> {
    let kind: String = row.get(1)?;
    Ok(DigestDelivery {
        delivery_key: row.get(0)?,
        kind: if kind == "morning" {
            DigestKind::Morning
        } else {
            DigestKind::Evening
        },
        date: row.get(2)?,
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
}

fn collect_facts(
    transaction: &Connection,
    kind: DigestKind,
    date: NaiveDate,
) -> Result<DigestFacts> {
    let yesterday = date
        .pred_opt()
        .ok_or_else(|| Error::InvalidInput(format!("date has no predecessor: {date}")))?;
    let due_end = date
        .checked_add_signed(Duration::days(3))
        .ok_or_else(|| Error::InvalidInput(format!("date is out of range: {date}")))?;
    let (day_start, day_end) = seoul_day_bounds(date)?;

    let yesterday_unfinished = query_tasks(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, d.status
         FROM task_day_entries d
         JOIN tasks t ON t.id = d.task_id
         LEFT JOIN projects p ON p.id = t.project_id
         WHERE d.entry_date = ?1 AND d.status = 'planned'
           AND t.deleted_at IS NULL AND t.status NOT IN ('done', 'cancelled')
         ORDER BY t.priority DESC, t.created_at ASC",
        [yesterday],
    )?;
    let today_planned = query_tasks(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, d.status
         FROM task_day_entries d
         JOIN tasks t ON t.id = d.task_id
         LEFT JOIN projects p ON p.id = t.project_id
         WHERE d.entry_date = ?1 AND d.status = 'planned' AND t.deleted_at IS NULL
         ORDER BY t.priority DESC, t.created_at ASC",
        [date],
    )?;
    let due_soon = query_tasks_two_dates(transaction, date, due_end)?;
    let blocked_tasks = query_tasks_no_params(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, NULL
         FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
         WHERE t.status = 'blocked' AND t.deleted_at IS NULL
         ORDER BY t.priority DESC, t.due_date ASC",
    )?;
    let recommended_priorities = query_tasks_no_params(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, NULL
         FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
         WHERE t.deleted_at IS NULL AND t.status IN ('inbox', 'todo', 'in_progress', 'blocked')
         ORDER BY CASE t.status WHEN 'blocked' THEN 0 WHEN 'in_progress' THEN 1 ELSE 2 END,
                  CASE WHEN t.due_date IS NULL THEN 1 ELSE 0 END,
                  t.due_date ASC, t.priority DESC, t.created_at ASC
         LIMIT 10",
    )?;
    let today_completed = query_tasks(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, d.status
         FROM task_day_entries d
         JOIN tasks t ON t.id = d.task_id
         LEFT JOIN projects p ON p.id = t.project_id
         WHERE d.entry_date = ?1 AND d.status = 'done' AND t.deleted_at IS NULL
         ORDER BY d.finalized_at ASC",
        [date],
    )?;
    let unfinished = today_planned.clone();
    let tomorrow_candidates = query_tasks_no_params(
        transaction,
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, NULL
         FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
         WHERE t.deleted_at IS NULL AND t.status IN ('todo', 'in_progress', 'blocked')
         ORDER BY CASE WHEN t.due_date IS NULL THEN 1 ELSE 0 END,
                  t.due_date ASC, t.priority DESC, t.created_at ASC
         LIMIT 15",
    )?;

    let session_results = query_session_facts(transaction, &day_start, &day_end)?;
    let worklogs = query_worklog_facts(transaction, date)?;
    let blocker_notes = session_results
        .iter()
        .filter_map(|session| {
            let text = session.blockers.trim();
            (!text.is_empty()).then(|| text.to_owned())
        })
        .collect();

    let mut facts = DigestFacts {
        timezone: "Asia/Seoul".to_owned(),
        generated_at: now_utc(),
        yesterday_unfinished,
        today_planned,
        due_soon,
        blocked_tasks,
        recommended_priorities,
        today_completed,
        session_results,
        worklogs,
        unfinished,
        blocker_notes,
        tomorrow_candidates,
    };
    if kind == DigestKind::Morning {
        facts.today_completed.clear();
        facts.session_results.clear();
        facts.worklogs.clear();
        facts.unfinished.clear();
        facts.blocker_notes.clear();
        facts.tomorrow_candidates.clear();
    } else {
        facts.yesterday_unfinished.clear();
        facts.today_planned.clear();
        facts.due_soon.clear();
        facts.recommended_priorities.clear();
    }
    Ok(facts)
}

fn query_tasks(
    transaction: &Connection,
    sql: &str,
    parameters: [NaiveDate; 1],
) -> Result<Vec<DigestTaskFact>> {
    let mut statement = transaction.prepare(sql)?;
    let rows = statement.query_map(parameters, map_task_fact)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_tasks_no_params(transaction: &Connection, sql: &str) -> Result<Vec<DigestTaskFact>> {
    let mut statement = transaction.prepare(sql)?;
    let rows = statement.query_map([], map_task_fact)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_tasks_two_dates(
    transaction: &Connection,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<DigestTaskFact>> {
    let mut statement = transaction.prepare(
        "SELECT t.id, t.title, t.project_id, p.name, t.status, t.priority, t.due_date, NULL
         FROM tasks t LEFT JOIN projects p ON p.id = t.project_id
         WHERE t.deleted_at IS NULL AND t.status NOT IN ('done', 'cancelled')
           AND t.due_date BETWEEN ?1 AND ?2
         ORDER BY t.due_date ASC, t.priority DESC",
    )?;
    let rows = statement.query_map(params![start, end], map_task_fact)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn map_task_fact(row: &rusqlite::Row<'_>) -> rusqlite::Result<DigestTaskFact> {
    Ok(DigestTaskFact {
        id: row.get(0)?,
        title: row.get(1)?,
        project_id: row.get(2)?,
        project_name: row.get(3)?,
        status: row.get(4)?,
        priority: row.get(5)?,
        due_date: row.get(6)?,
        day_status: row.get(7)?,
    })
}

fn query_session_facts(
    transaction: &Connection,
    start: &str,
    end: &str,
) -> Result<Vec<DigestSessionFact>> {
    let mut statement = transaction.prepare(
        "SELECT id, goal, result, blockers, next_action, ended_at
         FROM work_sessions
         WHERE deleted_at IS NULL AND status = 'completed' AND ended_at >= ?1 AND ended_at < ?2
         ORDER BY ended_at ASC",
    )?;
    let rows = statement.query_map(params![start, end], |row| {
        Ok(DigestSessionFact {
            id: row.get(0)?,
            goal: row.get(1)?,
            result: row.get(2)?,
            blockers: row.get(3)?,
            next_action: row.get(4)?,
            ended_at: row.get(5)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn query_worklog_facts(
    transaction: &Connection,
    date: NaiveDate,
) -> Result<Vec<DigestWorkLogFact>> {
    let mut statement = transaction.prepare(
        "SELECT id, title, body, session_id FROM worklogs
         WHERE deleted_at IS NULL AND log_date = ?1 ORDER BY created_at ASC",
    )?;
    let rows = statement.query_map([date], |row| {
        Ok(DigestWorkLogFact {
            id: row.get(0)?,
            title: row.get(1)?,
            body: row.get(2)?,
            session_id: row.get(3)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}
