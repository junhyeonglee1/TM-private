use std::str::FromStr;

use chrono::{DateTime, Duration, LocalResult, NaiveDate, NaiveTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::{Asia::Seoul, Tz};
use rusqlite::{OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Error, Result, TmCore, database::new_id, error::invalid,
    memory::run_memory_maintenance_in_transaction,
};

pub const SCHEDULER_POLL_SECONDS: u64 = 30;
pub const SCHEDULER_LEASE_SECONDS: i64 = 300;
pub const SCHEDULER_MAX_ATTEMPTS: i64 = 5;
pub const SCHEDULER_MISFIRE_GRACE_SECONDS: i64 = 86_400;
pub const SCHEDULER_QUIET_HOURS_START: &str = "22:00";
pub const SCHEDULER_QUIET_HOURS_END: &str = "07:00";
pub const SCHEDULER_TIMEZONE: &str = "Asia/Seoul";
const MAX_CLAIMS_PER_CYCLE: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerClaim {
    pub run_id: String,
    pub job_key: String,
    pub job_kind: String,
    pub scheduled_for: String,
    pub idempotency_key: String,
    pub attempt_number: i64,
    pub max_attempts: i64,
    pub worker_id: String,
    pub lease_expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerRun {
    pub id: String,
    pub job_key: String,
    pub job_kind: String,
    pub scheduled_for: String,
    pub status: String,
    pub attempt_count: i64,
    pub max_attempts: i64,
    pub idempotency_key: String,
    pub available_at: String,
    pub lease_owner: Option<String>,
    pub lease_expires_at: Option<String>,
    pub last_error: Option<String>,
    pub result: Option<Value>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerCycleReport {
    pub scheduled: usize,
    pub skipped: usize,
    pub lease_recoveries: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchedulerStatus {
    pub status: String,
    pub enabled_job_count: i64,
    pub queue_depth: i64,
    pub running_count: i64,
    pub retry_wait_count: i64,
    pub dead_letter_count: i64,
    pub effect_count: i64,
    pub succeeded_last_24_hours: i64,
    pub skipped_last_24_hours: i64,
    pub oldest_due_seconds: Option<i64>,
    pub last_succeeded_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub poll_interval_seconds: u64,
    pub lease_seconds: i64,
    pub timezone: &'static str,
    pub quiet_hours_start: &'static str,
    pub quiet_hours_end: &'static str,
    pub quiet_hours_user_facing_only: bool,
    pub openai_calls_enabled: bool,
    pub checked_at: String,
}

#[derive(Debug, Clone)]
struct SchedulerJob {
    id: String,
    job_key: String,
    schedule_type: String,
    interval_seconds: Option<i64>,
    local_time: Option<String>,
    timezone: String,
    max_attempts: i64,
    misfire_grace_seconds: i64,
    next_run_at: String,
}

#[derive(Debug, Default)]
pub struct SchedulerReconcileReport {
    pub scheduled: usize,
    pub skipped: usize,
}

impl TmCore {
    pub fn ensure_scheduler_defaults(&self, as_of: DateTime<Utc>) -> Result<()> {
        let now = timestamp(as_of);
        let memory_next = next_daily_after(Seoul, time(3, 10)?, as_of)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                transaction.execute(
                    "INSERT INTO scheduler_jobs(
                        id, job_key, kind, schedule_type, interval_seconds, local_time,
                        timezone, enabled, max_attempts, misfire_grace_seconds, coalesce,
                        next_run_at, last_scheduled_at, created_at, updated_at
                     ) VALUES (?1, 'scheduler.canary', 'scheduler.canary', 'interval', 3600,
                        NULL, 'Asia/Seoul', 1, ?2, ?3, 1, ?4, NULL, ?4, ?4)
                     ON CONFLICT(job_key) DO NOTHING",
                    params![
                        new_id(),
                        SCHEDULER_MAX_ATTEMPTS,
                        SCHEDULER_MISFIRE_GRACE_SECONDS,
                        now
                    ],
                )?;
                transaction.execute(
                    "INSERT INTO scheduler_jobs(
                        id, job_key, kind, schedule_type, interval_seconds, local_time,
                        timezone, enabled, max_attempts, misfire_grace_seconds, coalesce,
                        next_run_at, last_scheduled_at, created_at, updated_at
                     ) VALUES (?1, 'memory.maintenance', 'memory.maintenance', 'daily', NULL,
                        '03:10:00', 'Asia/Seoul', 1, ?2, ?3, 1, ?4, NULL, ?5, ?5)
                     ON CONFLICT(job_key) DO NOTHING",
                    params![
                        new_id(),
                        SCHEDULER_MAX_ATTEMPTS,
                        SCHEDULER_MISFIRE_GRACE_SECONDS,
                        timestamp(memory_next),
                        now,
                    ],
                )?;
                Ok(())
            })
    }

    pub fn run_scheduler_cycle(
        &self,
        worker_id: &str,
        as_of: DateTime<Utc>,
    ) -> Result<SchedulerCycleReport> {
        validate_worker_id(worker_id)?;
        self.ensure_scheduler_defaults(as_of)?;
        let reconcile = self.reconcile_scheduler(as_of)?;
        let mut report = SchedulerCycleReport {
            scheduled: reconcile.scheduled,
            skipped: reconcile.skipped,
            lease_recoveries: 0,
            succeeded: 0,
            failed: 0,
            checked_at: timestamp(as_of),
        };
        for _ in 0..MAX_CLAIMS_PER_CYCLE {
            let (claim, recovered) = self.claim_scheduler_run(worker_id, as_of)?;
            report.lease_recoveries += recovered;
            let Some(claim) = claim else {
                break;
            };
            match self.execute_scheduler_claim(&claim, as_of) {
                Ok(()) => report.succeeded += 1,
                Err(error) => {
                    report.failed += 1;
                    self.fail_scheduler_claim(&claim, &error.to_string(), true, as_of)?;
                }
            }
        }
        Ok(report)
    }

    pub fn reconcile_scheduler(&self, as_of: DateTime<Utc>) -> Result<SchedulerReconcileReport> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let jobs = {
                    let mut statement = transaction.prepare(
                        "SELECT id, job_key, schedule_type, interval_seconds, local_time,
                                timezone, max_attempts, misfire_grace_seconds, next_run_at
                         FROM scheduler_jobs
                         WHERE enabled = 1 AND next_run_at <= ?1
                         ORDER BY next_run_at, id",
                    )?;
                    let rows = statement.query_map([timestamp(as_of)], map_job)?;
                    rows.collect::<std::result::Result<Vec<_>, _>>()?
                };
                let mut report = SchedulerReconcileReport::default();
                for job in jobs {
                    reconcile_job(transaction, &job, as_of, &mut report)?;
                }
                Ok(report)
            })
    }

    pub fn claim_scheduler_run(
        &self,
        worker_id: &str,
        as_of: DateTime<Utc>,
    ) -> Result<(Option<SchedulerClaim>, usize)> {
        validate_worker_id(worker_id)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let recovered = recover_expired_leases(transaction, as_of)?;
                let now = timestamp(as_of);
                let candidate: Option<(String, String, String)> = transaction
                    .query_row(
                        "SELECT run.id, job.job_key, job.kind
                         FROM scheduler_runs AS run
                         JOIN scheduler_jobs AS job ON job.id = run.job_id
                         WHERE run.status IN ('pending', 'retry_wait')
                           AND run.available_at <= ?1
                         ORDER BY run.available_at, run.scheduled_for, run.id
                         LIMIT 1",
                        [&now],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let Some((run_id, job_key, job_kind)) = candidate else {
                    return Ok((None, recovered));
                };
                let lease_expires_at =
                    timestamp(as_of + Duration::seconds(SCHEDULER_LEASE_SECONDS));
                let changed = transaction.execute(
                    "UPDATE scheduler_runs
                     SET status = 'running', attempt_count = attempt_count + 1,
                         lease_owner = ?2, lease_acquired_at = ?3, lease_expires_at = ?4,
                         started_at = coalesce(started_at, ?3), updated_at = ?3
                     WHERE id = ?1 AND status IN ('pending', 'retry_wait')
                       AND available_at <= ?3",
                    params![run_id, worker_id, now, lease_expires_at],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "scheduler run was claimed concurrently".to_owned(),
                    ));
                }
                let claim = transaction.query_row(
                    "SELECT scheduled_for, idempotency_key, attempt_count, max_attempts
                     FROM scheduler_runs WHERE id = ?1",
                    [&run_id],
                    |row| {
                        Ok(SchedulerClaim {
                            run_id: run_id.clone(),
                            job_key: job_key.clone(),
                            job_kind: job_kind.clone(),
                            scheduled_for: row.get(0)?,
                            idempotency_key: row.get(1)?,
                            attempt_number: row.get(2)?,
                            max_attempts: row.get(3)?,
                            worker_id: worker_id.to_owned(),
                            lease_expires_at: lease_expires_at.clone(),
                        })
                    },
                )?;
                Ok((Some(claim), recovered))
            })
    }

    pub fn execute_scheduler_claim(
        &self,
        claim: &SchedulerClaim,
        as_of: DateTime<Utc>,
    ) -> Result<()> {
        let now = timestamp(as_of);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                validate_active_claim(transaction, claim)?;
                let existing: Option<String> = transaction
                    .query_row(
                        "SELECT result_json FROM scheduler_effects WHERE idempotency_key = ?1",
                        [&claim.idempotency_key],
                        |row| row.get(0),
                    )
                    .optional()?;
                let result_json = if let Some(existing) = existing {
                    existing
                } else {
                    let result = match claim.job_kind.as_str() {
                        "scheduler.canary" => json!({
                            "status": "ok",
                            "kind": "scheduler.canary",
                            "openAiCalls": 0,
                            "scheduledFor": claim.scheduled_for,
                        }),
                        "memory.maintenance" => serde_json::to_value(
                            run_memory_maintenance_in_transaction(transaction, as_of)?,
                        )?,
                        other => {
                            return Err(Error::Invariant(format!(
                                "unsupported scheduler job kind: {other}"
                            )));
                        }
                    };
                    let encoded = serde_json::to_string(&result)?;
                    transaction.execute(
                        "INSERT INTO scheduler_effects(
                            idempotency_key, run_id, job_kind, result_json, applied_at
                         ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            claim.idempotency_key,
                            claim.run_id,
                            claim.job_kind,
                            encoded,
                            now
                        ],
                    )?;
                    encoded
                };
                transaction.execute(
                    "INSERT INTO scheduler_attempts(
                        id, run_id, attempt_number, worker_id, started_at,
                        completed_at, outcome, error, created_at
                     ) SELECT ?1, id, attempt_count, lease_owner, lease_acquired_at,
                              ?2, 'succeeded', NULL, ?2
                       FROM scheduler_runs WHERE id = ?3",
                    params![new_id(), now, claim.run_id],
                )?;
                let changed = transaction.execute(
                    "UPDATE scheduler_runs
                     SET status = 'succeeded', result_json = ?2, completed_at = ?3,
                         updated_at = ?3, lease_owner = NULL, lease_acquired_at = NULL,
                         lease_expires_at = NULL, last_error = NULL
                     WHERE id = ?1 AND status = 'running' AND lease_owner = ?4
                       AND attempt_count = ?5",
                    params![
                        claim.run_id,
                        result_json,
                        now,
                        claim.worker_id,
                        claim.attempt_number,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "scheduler claim changed during execution".to_owned(),
                    ));
                }
                Ok(())
            })
    }

    pub fn fail_scheduler_claim(
        &self,
        claim: &SchedulerClaim,
        error: &str,
        retryable: bool,
        as_of: DateTime<Utc>,
    ) -> Result<()> {
        let now = timestamp(as_of);
        let error = bounded_error(error);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                validate_active_claim(transaction, claim)?;
                let terminal = !retryable || claim.attempt_number >= claim.max_attempts;
                let outcome = if terminal {
                    "dead_letter"
                } else {
                    "retry_scheduled"
                };
                transaction.execute(
                    "INSERT INTO scheduler_attempts(
                        id, run_id, attempt_number, worker_id, started_at,
                        completed_at, outcome, error, created_at
                     ) SELECT ?1, id, attempt_count, lease_owner, lease_acquired_at,
                              ?2, ?3, ?4, ?2
                       FROM scheduler_runs WHERE id = ?5",
                    params![new_id(), now, outcome, error, claim.run_id],
                )?;
                let (status, available_at, dead_letter_at) = if terminal {
                    ("dead_letter", now.clone(), Some(now.clone()))
                } else {
                    (
                        "retry_wait",
                        timestamp(
                            as_of + Duration::seconds(retry_delay_seconds(claim.attempt_number)),
                        ),
                        None,
                    )
                };
                let changed = transaction.execute(
                    "UPDATE scheduler_runs
                     SET status = ?2, available_at = ?3, last_error = ?4,
                         updated_at = ?5, completed_at = CASE WHEN ?6 IS NULL THEN NULL ELSE ?5 END,
                         dead_letter_at = ?6, lease_owner = NULL,
                         lease_acquired_at = NULL, lease_expires_at = NULL
                     WHERE id = ?1 AND status = 'running' AND lease_owner = ?7
                       AND attempt_count = ?8",
                    params![
                        claim.run_id,
                        status,
                        available_at,
                        error,
                        now,
                        dead_letter_at,
                        claim.worker_id,
                        claim.attempt_number,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(
                        "scheduler claim changed while failing".to_owned(),
                    ));
                }
                Ok(())
            })
    }

    pub fn scheduler_status(&self, as_of: DateTime<Utc>) -> Result<SchedulerStatus> {
        let connection = self.database.connect()?;
        let enabled_job_count = connection.query_row(
            "SELECT count(*) FROM scheduler_jobs WHERE enabled = 1",
            [],
            |row| row.get(0),
        )?;
        let count = |status: &str| -> Result<i64> {
            Ok(connection.query_row(
                "SELECT count(*) FROM scheduler_runs WHERE status = ?1",
                [status],
                |row| row.get(0),
            )?)
        };
        let pending = count("pending")?;
        let running_count = count("running")?;
        let retry_wait_count = count("retry_wait")?;
        let dead_letter_count = count("dead_letter")?;
        let effect_count =
            connection.query_row("SELECT count(*) FROM scheduler_effects", [], |row| {
                row.get(0)
            })?;
        let cutoff = timestamp(as_of - Duration::hours(24));
        let succeeded_last_24_hours = connection.query_row(
            "SELECT count(*) FROM scheduler_runs
             WHERE status = 'succeeded' AND completed_at >= ?1",
            [&cutoff],
            |row| row.get(0),
        )?;
        let skipped_last_24_hours = connection.query_row(
            "SELECT count(*) FROM scheduler_runs
             WHERE status = 'skipped' AND skipped_at >= ?1",
            [&cutoff],
            |row| row.get(0),
        )?;
        let last_succeeded_at = optional_scalar(
            &connection,
            "SELECT max(completed_at) FROM scheduler_runs WHERE status = 'succeeded'",
        )?;
        let last_failure_at = optional_scalar(
            &connection,
            "SELECT max(completed_at) FROM scheduler_attempts
             WHERE outcome IN ('retry_scheduled', 'dead_letter', 'lease_expired')",
        )?;
        let oldest_due: Option<String> = connection.query_row(
            "SELECT min(scheduled_for) FROM scheduler_runs
             WHERE status IN ('pending', 'running', 'retry_wait')",
            [],
            |row| row.get(0),
        )?;
        let oldest_due_seconds = oldest_due
            .map(|value| parse_utc(&value).map(|due| (as_of - due).num_seconds().max(0)))
            .transpose()?;
        let degraded = dead_letter_count > 0 || oldest_due_seconds.is_some_and(|age| age > 300);
        Ok(SchedulerStatus {
            status: if degraded { "degraded" } else { "healthy" }.to_owned(),
            enabled_job_count,
            queue_depth: pending + retry_wait_count,
            running_count,
            retry_wait_count,
            dead_letter_count,
            effect_count,
            succeeded_last_24_hours,
            skipped_last_24_hours,
            oldest_due_seconds,
            last_succeeded_at,
            last_failure_at,
            poll_interval_seconds: SCHEDULER_POLL_SECONDS,
            lease_seconds: SCHEDULER_LEASE_SECONDS,
            timezone: SCHEDULER_TIMEZONE,
            quiet_hours_start: SCHEDULER_QUIET_HOURS_START,
            quiet_hours_end: SCHEDULER_QUIET_HOURS_END,
            quiet_hours_user_facing_only: true,
            openai_calls_enabled: false,
            checked_at: timestamp(as_of),
        })
    }

    pub fn list_scheduler_runs(&self) -> Result<Vec<SchedulerRun>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT run.id, job.job_key, job.kind, run.scheduled_for, run.status,
                    run.attempt_count, run.max_attempts, run.idempotency_key,
                    run.available_at, run.lease_owner, run.lease_expires_at,
                    run.last_error, run.result_json, run.completed_at
             FROM scheduler_runs AS run
             JOIN scheduler_jobs AS job ON job.id = run.job_id
             ORDER BY run.scheduled_for, run.id",
        )?;
        let rows = statement.query_map([], map_run)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

fn reconcile_job(
    transaction: &Transaction<'_>,
    job: &SchedulerJob,
    as_of: DateTime<Utc>,
    report: &mut SchedulerReconcileReport,
) -> Result<()> {
    let first_due = parse_utc(&job.next_run_at)?;
    if first_due > as_of {
        return Ok(());
    }
    let (latest_due, next_run) = match job.schedule_type.as_str() {
        "interval" => {
            let seconds = job.interval_seconds.ok_or_else(|| {
                Error::Invariant(format!("interval is missing for job {}", job.job_key))
            })?;
            let intervals = (as_of - first_due).num_seconds() / seconds;
            let latest = first_due + Duration::seconds(intervals * seconds);
            (latest, Some(latest + Duration::seconds(seconds)))
        }
        "daily" => {
            let timezone = Tz::from_str(&job.timezone).map_err(|_| {
                Error::Invariant(format!("invalid scheduler timezone: {}", job.timezone))
            })?;
            let local_time = NaiveTime::parse_from_str(
                job.local_time.as_deref().unwrap_or_default(),
                "%H:%M:%S",
            )
            .map_err(|error| Error::Invariant(format!("invalid scheduler local time: {error}")))?;
            let latest = latest_daily_at_or_before(timezone, local_time, as_of)?;
            (
                latest,
                Some(next_daily_after(timezone, local_time, latest)?),
            )
        }
        "once" => (first_due, None),
        other => {
            return Err(Error::Invariant(format!(
                "unsupported schedule type: {other}"
            )));
        }
    };

    if first_due < latest_due && (as_of - first_due).num_seconds() > job.misfire_grace_seconds {
        insert_run(
            transaction,
            job,
            first_due,
            "skipped",
            as_of,
            Some("coalesced_misfire"),
        )?;
        report.skipped += 1;
    }

    let latest_is_expired = (as_of - latest_due).num_seconds() > job.misfire_grace_seconds;
    if latest_is_expired {
        insert_run(
            transaction,
            job,
            latest_due,
            "skipped",
            as_of,
            Some("misfire_expired"),
        )?;
        report.skipped += 1;
    } else {
        insert_run(transaction, job, latest_due, "pending", as_of, None)?;
        report.scheduled += 1;
    }

    if let Some(next_run) = next_run {
        transaction.execute(
            "UPDATE scheduler_jobs
             SET next_run_at = ?2, last_scheduled_at = ?3, updated_at = ?4
             WHERE id = ?1",
            params![
                job.id,
                timestamp(next_run),
                timestamp(latest_due),
                timestamp(as_of)
            ],
        )?;
    } else {
        transaction.execute(
            "UPDATE scheduler_jobs
             SET enabled = 0, last_scheduled_at = ?2, updated_at = ?3
             WHERE id = ?1",
            params![job.id, timestamp(latest_due), timestamp(as_of)],
        )?;
    }
    Ok(())
}

fn insert_run(
    transaction: &Transaction<'_>,
    job: &SchedulerJob,
    scheduled_for: DateTime<Utc>,
    status: &str,
    as_of: DateTime<Utc>,
    reason: Option<&str>,
) -> Result<()> {
    let scheduled = timestamp(scheduled_for);
    let now = timestamp(as_of);
    let idempotency_key = format!("{}:{scheduled}", job.job_key);
    let terminal = status == "skipped";
    transaction.execute(
        "INSERT INTO scheduler_runs(
            id, job_id, scheduled_for, status, attempt_count, max_attempts,
            idempotency_key, available_at, lease_owner, lease_acquired_at,
            lease_expires_at, last_error, result_json, created_at, updated_at,
            started_at, completed_at, dead_letter_at, skipped_at
         ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7, NULL, NULL, NULL,
            ?8, NULL, ?9, ?9, NULL, ?10, NULL, ?11)
         ON CONFLICT(job_id, scheduled_for) DO NOTHING",
        params![
            new_id(),
            job.id,
            scheduled,
            status,
            job.max_attempts,
            idempotency_key,
            now,
            reason,
            now,
            terminal.then_some(now.clone()),
            terminal.then_some(now.clone()),
        ],
    )?;
    Ok(())
}

fn recover_expired_leases(transaction: &Transaction<'_>, as_of: DateTime<Utc>) -> Result<usize> {
    let now = timestamp(as_of);
    let expired = {
        let mut statement = transaction.prepare(
            "SELECT id, attempt_count, max_attempts, lease_owner, lease_acquired_at
             FROM scheduler_runs
             WHERE status = 'running' AND lease_expires_at <= ?1
             ORDER BY lease_expires_at, id",
        )?;
        let rows = statement.query_map([&now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (run_id, attempt, max_attempts, worker, started_at) in &expired {
        let terminal = attempt >= max_attempts;
        let outcome = if terminal {
            "dead_letter"
        } else {
            "lease_expired"
        };
        transaction.execute(
            "INSERT INTO scheduler_attempts(
                id, run_id, attempt_number, worker_id, started_at,
                completed_at, outcome, error, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'lease expired', ?6)
             ON CONFLICT(run_id, attempt_number) DO NOTHING",
            params![new_id(), run_id, attempt, worker, started_at, now, outcome],
        )?;
        let (status, available_at, dead_letter_at) = if terminal {
            ("dead_letter", now.clone(), Some(now.clone()))
        } else {
            (
                "retry_wait",
                timestamp(as_of + Duration::seconds(retry_delay_seconds(*attempt))),
                None,
            )
        };
        transaction.execute(
            "UPDATE scheduler_runs
             SET status = ?2, available_at = ?3, last_error = 'lease expired',
                 updated_at = ?4, completed_at = CASE WHEN ?5 IS NULL THEN NULL ELSE ?4 END,
                 dead_letter_at = ?5, lease_owner = NULL,
                 lease_acquired_at = NULL, lease_expires_at = NULL
             WHERE id = ?1 AND status = 'running'",
            params![run_id, status, available_at, now, dead_letter_at],
        )?;
    }
    Ok(expired.len())
}

fn validate_active_claim(transaction: &Transaction<'_>, claim: &SchedulerClaim) -> Result<()> {
    let active: bool = transaction.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM scheduler_runs
            WHERE id = ?1 AND status = 'running' AND lease_owner = ?2
              AND attempt_count = ?3 AND idempotency_key = ?4
         )",
        params![
            claim.run_id,
            claim.worker_id,
            claim.attempt_number,
            claim.idempotency_key,
        ],
        |row| row.get(0),
    )?;
    if !active {
        return Err(Error::Conflict(
            "scheduler claim is no longer active".to_owned(),
        ));
    }
    Ok(())
}

fn retry_delay_seconds(attempt_number: i64) -> i64 {
    match attempt_number {
        i64::MIN..=1 => 30,
        2 => 120,
        3 => 600,
        _ => 3600,
    }
}

fn latest_daily_at_or_before(
    timezone: Tz,
    local_time: NaiveTime,
    as_of: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let local_date = as_of.with_timezone(&timezone).date_naive();
    let today = resolve_local(timezone, local_date, local_time)?;
    if today <= as_of {
        return Ok(today);
    }
    let previous = local_date
        .pred_opt()
        .ok_or_else(|| invalid("scheduler date has no predecessor"))?;
    resolve_local(timezone, previous, local_time)
}

fn next_daily_after(
    timezone: Tz,
    local_time: NaiveTime,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>> {
    let local_date = after.with_timezone(&timezone).date_naive();
    for offset in 0..=2_i64 {
        let date = local_date
            .checked_add_signed(Duration::days(offset))
            .ok_or_else(|| invalid("scheduler date is out of range"))?;
        let candidate = resolve_local(timezone, date, local_time)?;
        if candidate > after {
            return Ok(candidate);
        }
    }
    Err(Error::Invariant(
        "unable to resolve the next daily scheduler occurrence".to_owned(),
    ))
}

fn resolve_local(timezone: Tz, date: NaiveDate, local_time: NaiveTime) -> Result<DateTime<Utc>> {
    let mut local = date.and_time(local_time);
    for _ in 0..=180 {
        match timezone.from_local_datetime(&local) {
            LocalResult::Single(value) => return Ok(value.with_timezone(&Utc)),
            LocalResult::Ambiguous(first, second) => {
                return Ok(first.min(second).with_timezone(&Utc));
            }
            LocalResult::None => local += Duration::minutes(1),
        }
    }
    Err(Error::Invariant(format!(
        "unable to resolve scheduler local time {date} {local_time} in {timezone}"
    )))
}

fn map_job(row: &Row<'_>) -> rusqlite::Result<SchedulerJob> {
    Ok(SchedulerJob {
        id: row.get(0)?,
        job_key: row.get(1)?,
        schedule_type: row.get(2)?,
        interval_seconds: row.get(3)?,
        local_time: row.get(4)?,
        timezone: row.get(5)?,
        max_attempts: row.get(6)?,
        misfire_grace_seconds: row.get(7)?,
        next_run_at: row.get(8)?,
    })
}

fn map_run(row: &Row<'_>) -> rusqlite::Result<SchedulerRun> {
    let result_json: Option<String> = row.get(12)?;
    let result = result_json
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(SchedulerRun {
        id: row.get(0)?,
        job_key: row.get(1)?,
        job_kind: row.get(2)?,
        scheduled_for: row.get(3)?,
        status: row.get(4)?,
        attempt_count: row.get(5)?,
        max_attempts: row.get(6)?,
        idempotency_key: row.get(7)?,
        available_at: row.get(8)?,
        lease_owner: row.get(9)?,
        lease_expires_at: row.get(10)?,
        last_error: row.get(11)?,
        result,
        completed_at: row.get(13)?,
    })
}

fn optional_scalar(connection: &rusqlite::Connection, sql: &str) -> Result<Option<String>> {
    Ok(connection.query_row(sql, [], |row| row.get(0))?)
}

fn validate_worker_id(worker_id: &str) -> Result<()> {
    if worker_id.trim().is_empty() || worker_id.chars().count() > 200 {
        return Err(invalid("scheduler worker ID must contain 1-200 characters"));
    }
    Ok(())
}

fn bounded_error(value: &str) -> String {
    value.chars().take(1_000).collect()
}

fn parse_utc(value: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| Error::Invariant(format!("invalid scheduler UTC timestamp: {error}")))
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn time(hour: u32, minute: u32) -> Result<NaiveTime> {
    NaiveTime::from_hms_opt(hour, minute, 0)
        .ok_or_else(|| Error::Invariant("invalid scheduler local time".to_owned()))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Timelike};
    use chrono_tz::{America::New_York, Asia::Seoul};

    use super::{next_daily_after, time};

    #[test]
    fn seoul_daily_schedule_is_stable_in_utc() {
        let after = chrono::Utc
            .with_ymd_and_hms(2026, 7, 21, 18, 11, 0)
            .single()
            .expect("valid UTC time");
        let next = next_daily_after(Seoul, time(3, 10).expect("valid time"), after)
            .expect("resolve Seoul schedule");
        assert_eq!(
            next,
            chrono::Utc
                .with_ymd_and_hms(2026, 7, 22, 18, 10, 0)
                .single()
                .expect("valid UTC time")
        );
    }

    #[test]
    fn nonexistent_dst_time_advances_to_first_valid_minute() {
        let after = chrono::Utc
            .with_ymd_and_hms(2026, 3, 7, 12, 0, 0)
            .single()
            .expect("valid UTC time");
        let next = next_daily_after(New_York, time(2, 30).expect("valid time"), after)
            .expect("resolve DST gap");
        let local = next.with_timezone(&New_York);
        assert_eq!(local.date_naive().to_string(), "2026-03-08");
        assert_eq!(local.hour(), 3);
        assert_eq!(local.minute(), 0);
    }

    #[test]
    fn ambiguous_dst_time_uses_the_first_occurrence() {
        let after = chrono::Utc
            .with_ymd_and_hms(2026, 10, 31, 12, 0, 0)
            .single()
            .expect("valid UTC time");
        let next = next_daily_after(New_York, time(1, 30).expect("valid time"), after)
            .expect("resolve DST overlap");
        assert_eq!(
            next,
            chrono::Utc
                .with_ymd_and_hms(2026, 11, 1, 5, 30, 0)
                .single()
                .expect("valid UTC time")
        );
    }
}
