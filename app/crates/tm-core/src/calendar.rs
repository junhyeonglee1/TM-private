use std::{fmt, str::FromStr};

use chrono::{Datelike, NaiveDate, NaiveTime};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params, types::Type};
use serde::{Deserialize, Serialize};

use crate::{
    Error, RecurringExpenseOccurrence, Result, TmCore,
    database::{new_id, now_utc},
    error::{invalid, not_found},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarEventKind {
    Personal,
    Payment,
}

impl CalendarEventKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Personal => "personal",
            Self::Payment => "payment",
        }
    }
}

impl fmt::Display for CalendarEventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CalendarEventKind {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "personal" => Ok(Self::Personal),
            "payment" => Ok(Self::Payment),
            other => Err(Error::Invariant(format!(
                "unknown calendar event kind: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarRecurrence {
    None,
    MonthlyDay,
    MonthlyFirstDay,
    MonthlyLastDay,
}

impl CalendarRecurrence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::MonthlyDay => "monthly_day",
            Self::MonthlyFirstDay => "monthly_first_day",
            Self::MonthlyLastDay => "monthly_last_day",
        }
    }
}

impl fmt::Display for CalendarRecurrence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for CalendarRecurrence {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "none" => Ok(Self::None),
            "monthly_day" => Ok(Self::MonthlyDay),
            "monthly_first_day" => Ok(Self::MonthlyFirstDay),
            "monthly_last_day" => Ok(Self::MonthlyLastDay),
            other => Err(Error::Invariant(format!(
                "unknown calendar recurrence: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEvent {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: CalendarEventKind,
    pub start_date: NaiveDate,
    pub event_time: Option<NaiveTime>,
    pub recurrence: CalendarRecurrence,
    pub day_of_month: Option<u8>,
    pub ends_on: Option<NaiveDate>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateCalendarEventInput {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: CalendarEventKind,
    pub start_date: NaiveDate,
    pub event_time: Option<NaiveTime>,
    pub recurrence: CalendarRecurrence,
    pub day_of_month: Option<u8>,
    pub ends_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateCalendarEventInput {
    pub expected_version: u64,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub kind: CalendarEventKind,
    pub start_date: NaiveDate,
    pub event_time: Option<NaiveTime>,
    pub recurrence: CalendarRecurrence,
    pub day_of_month: Option<u8>,
    pub ends_on: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarOccurrence {
    pub occurrence_key: String,
    pub event_id: String,
    pub title: String,
    pub description: String,
    pub kind: CalendarEventKind,
    pub date: NaiveDate,
    pub event_time: Option<NaiveTime>,
    pub recurrence: CalendarRecurrence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarMonth {
    pub month: String,
    pub month_start: NaiveDate,
    pub month_end: NaiveDate,
    pub events: Vec<CalendarEvent>,
    pub occurrences: Vec<CalendarOccurrence>,
    /// Virtual payment occurrences; these are never copied into `calendar_events`.
    pub expense_occurrences: Vec<RecurringExpenseOccurrence>,
}

impl TmCore {
    pub fn create_calendar_event(&self, input: CreateCalendarEventInput) -> Result<CalendarEvent> {
        validate_calendar_fields(
            &input.title,
            &input.description,
            input.start_date,
            input.recurrence,
            input.day_of_month,
            input.ends_on,
        )?;
        let id = new_id();
        let now = now_utc();
        let connection = self.database.connect()?;
        connection.execute(
            "INSERT INTO calendar_events(
                id, title, description, event_kind, start_date, event_time,
                recurrence, day_of_month, ends_on, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                id,
                input.title.trim(),
                input.description.trim(),
                input.kind.as_str(),
                input.start_date,
                input.event_time,
                input.recurrence.as_str(),
                input.day_of_month,
                input.ends_on,
                now,
            ],
        )?;
        query_calendar_event(&connection, &id, false)
    }

    pub fn update_calendar_event(
        &self,
        event_id: &str,
        input: UpdateCalendarEventInput,
    ) -> Result<CalendarEvent> {
        if input.expected_version == 0 {
            return Err(invalid(
                "calendar expected version must be greater than zero",
            ));
        }
        validate_calendar_fields(
            &input.title,
            &input.description,
            input.start_date,
            input.recurrence,
            input.day_of_month,
            input.ends_on,
        )?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_calendar_event(transaction, event_id, true)?;
                if current.deleted_at.is_some() {
                    return Err(Error::Conflict(format!(
                        "calendar event {event_id} has been deleted"
                    )));
                }
                if current.version != input.expected_version {
                    return Err(Error::Conflict(format!(
                        "calendar event {event_id} changed from version {} to {}",
                        input.expected_version, current.version
                    )));
                }
                let changed = transaction.execute(
                    "UPDATE calendar_events
                     SET title = ?2, description = ?3, event_kind = ?4,
                         start_date = ?5, event_time = ?6, recurrence = ?7,
                         day_of_month = ?8, ends_on = ?9, updated_at = ?10,
                         version = version + 1
                     WHERE id = ?1 AND version = ?11 AND deleted_at IS NULL",
                    params![
                        event_id,
                        input.title.trim(),
                        input.description.trim(),
                        input.kind.as_str(),
                        input.start_date,
                        input.event_time,
                        input.recurrence.as_str(),
                        input.day_of_month,
                        input.ends_on,
                        now_utc(),
                        input.expected_version,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(format!(
                        "calendar event {event_id} changed before the update completed"
                    )));
                }
                query_calendar_event(transaction, event_id, false)
            })
    }

    pub fn delete_calendar_event(&self, event_id: &str, expected_version: u64) -> Result<()> {
        if expected_version == 0 {
            return Err(invalid(
                "calendar expected version must be greater than zero",
            ));
        }
        let connection = self.database.connect()?;
        let changed = connection.execute(
            "UPDATE calendar_events
             SET deleted_at = ?2, updated_at = ?2, version = version + 1
             WHERE id = ?1 AND version = ?3 AND deleted_at IS NULL",
            params![event_id, now_utc(), expected_version],
        )?;
        if changed == 1 {
            return Ok(());
        }
        let existing = connection
            .query_row(
                "SELECT version, deleted_at FROM calendar_events WHERE id = ?1",
                [event_id],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, Option<String>>(1)?)),
            )
            .optional()?;
        match existing {
            None => Err(not_found("calendar event", event_id)),
            Some((_, Some(_))) => Err(Error::Conflict(format!(
                "calendar event {event_id} has already been deleted"
            ))),
            Some((version, None)) => Err(Error::Conflict(format!(
                "calendar event {event_id} changed from version {expected_version} to {version}"
            ))),
        }
    }

    pub fn list_calendar_events(&self, include_deleted: bool) -> Result<Vec<CalendarEvent>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, title, description, event_kind, start_date, event_time,
                    recurrence, day_of_month, ends_on, created_at, updated_at,
                    deleted_at, version
             FROM calendar_events
             WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY start_date, event_time, title COLLATE NOCASE, id",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_calendar_event)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn calendar_month(&self, year: i32, month: u32) -> Result<CalendarMonth> {
        let month_start = NaiveDate::from_ymd_opt(year, month, 1)
            .ok_or_else(|| invalid("calendar month must be a valid YYYY-MM value"))?;
        let month_end = last_day_of_month(year, month)?;
        let events = self.list_calendar_events(false)?;
        let occurrences = occurrences_for_month(&events, month_start, month_end);
        let expense_occurrences = self.recurring_expense_occurrences(month_start)?;
        Ok(CalendarMonth {
            month: format!("{year:04}-{month:02}"),
            month_start,
            month_end,
            events,
            occurrences,
            expense_occurrences,
        })
    }

    pub fn calendar_occurrences_between(
        &self,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<CalendarOccurrence>> {
        if end_date < start_date {
            return Err(invalid("calendar end date cannot be before its start date"));
        }
        if end_date.signed_duration_since(start_date).num_days() > 366 {
            return Err(invalid("calendar occurrence window cannot exceed 367 days"));
        }

        let events = self.list_calendar_events(false)?;
        let mut month_start = NaiveDate::from_ymd_opt(start_date.year(), start_date.month(), 1)
            .ok_or_else(|| invalid("calendar occurrence window is outside the supported range"))?;
        let mut occurrences = Vec::new();
        while month_start <= end_date {
            let month_end = last_day_of_month(month_start.year(), month_start.month())?;
            occurrences.extend(
                occurrences_for_month(&events, month_start, month_end)
                    .into_iter()
                    .filter(|item| item.date >= start_date && item.date <= end_date),
            );
            month_start = next_month_start(month_start)?;
        }
        Ok(occurrences)
    }
}

fn occurrences_for_month(
    events: &[CalendarEvent],
    month_start: NaiveDate,
    month_end: NaiveDate,
) -> Vec<CalendarOccurrence> {
    let mut occurrences = events
        .iter()
        .filter_map(|event| {
            occurrence_date(event, month_start, month_end).map(|date| (event, date))
        })
        .map(|(event, date)| CalendarOccurrence {
            occurrence_key: format!("{}:{date}", event.id),
            event_id: event.id.clone(),
            title: event.title.clone(),
            description: event.description.clone(),
            kind: event.kind,
            date,
            event_time: event.event_time,
            recurrence: event.recurrence,
        })
        .collect::<Vec<_>>();
    occurrences.sort_by(|left, right| {
        left.date
            .cmp(&right.date)
            .then_with(|| left.event_time.cmp(&right.event_time))
            .then_with(|| left.title.to_lowercase().cmp(&right.title.to_lowercase()))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    occurrences
}

fn next_month_start(month_start: NaiveDate) -> Result<NaiveDate> {
    month_start
        .checked_add_months(chrono::Months::new(1))
        .ok_or_else(|| invalid("calendar occurrence window is outside the supported range"))
}

fn validate_calendar_fields(
    title: &str,
    description: &str,
    start_date: NaiveDate,
    recurrence: CalendarRecurrence,
    day_of_month: Option<u8>,
    ends_on: Option<NaiveDate>,
) -> Result<()> {
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid("calendar title cannot be empty"));
    }
    if title.chars().count() > 200 {
        return Err(invalid("calendar title cannot exceed 200 characters"));
    }
    if description.chars().count() > 4000 {
        return Err(invalid(
            "calendar description cannot exceed 4000 characters",
        ));
    }
    match (recurrence, day_of_month) {
        (CalendarRecurrence::MonthlyDay, Some(day)) if (1..=31).contains(&day) => {}
        (CalendarRecurrence::MonthlyDay, _) => {
            return Err(invalid(
                "monthly day recurrence requires a day between 1 and 31",
            ));
        }
        (_, Some(_)) => {
            return Err(invalid(
                "day of month is only allowed for monthly day recurrence",
            ));
        }
        _ => {}
    }
    if recurrence == CalendarRecurrence::None && ends_on.is_some() {
        return Err(invalid("one-time calendar events cannot have an end date"));
    }
    if ends_on.is_some_and(|value| value < start_date) {
        return Err(invalid(
            "calendar recurrence end date cannot be before its start date",
        ));
    }
    Ok(())
}

fn occurrence_date(
    event: &CalendarEvent,
    month_start: NaiveDate,
    month_end: NaiveDate,
) -> Option<NaiveDate> {
    let date = match event.recurrence {
        CalendarRecurrence::None => event.start_date,
        CalendarRecurrence::MonthlyDay => NaiveDate::from_ymd_opt(
            month_start.year(),
            month_start.month(),
            u32::from(event.day_of_month?),
        )?,
        CalendarRecurrence::MonthlyFirstDay => month_start,
        CalendarRecurrence::MonthlyLastDay => month_end,
    };
    (date >= month_start
        && date <= month_end
        && date >= event.start_date
        && event.ends_on.is_none_or(|end| date <= end))
    .then_some(date)
}

fn last_day_of_month(year: i32, month: u32) -> Result<NaiveDate> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_start = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .ok_or_else(|| invalid("calendar month is outside the supported date range"))?;
    next_start
        .pred_opt()
        .ok_or_else(|| invalid("calendar month is outside the supported date range"))
}

fn query_calendar_event(
    connection: &Connection,
    event_id: &str,
    include_deleted: bool,
) -> Result<CalendarEvent> {
    connection
        .query_row(
            "SELECT id, title, description, event_kind, start_date, event_time,
                    recurrence, day_of_month, ends_on, created_at, updated_at,
                    deleted_at, version
             FROM calendar_events
             WHERE id = ?1 AND (?2 = 1 OR deleted_at IS NULL)",
            params![event_id, i64::from(include_deleted)],
            map_calendar_event,
        )
        .optional()?
        .ok_or_else(|| not_found("calendar event", event_id))
}

fn map_calendar_event(row: &Row<'_>) -> rusqlite::Result<CalendarEvent> {
    let kind = row.get::<_, String>(3)?;
    let recurrence = row.get::<_, String>(6)?;
    Ok(CalendarEvent {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        kind: CalendarEventKind::from_str(&kind).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(3, Type::Text, Box::new(error))
        })?,
        start_date: row.get(4)?,
        event_time: row.get(5)?,
        recurrence: CalendarRecurrence::from_str(&recurrence).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Text, Box::new(error))
        })?,
        day_of_month: row.get(7)?,
        ends_on: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        deleted_at: row.get(11)?,
        version: row.get(12)?,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, NaiveTime};

    use super::{CalendarEvent, CalendarEventKind, CalendarRecurrence, occurrence_date};

    fn recurring(recurrence: CalendarRecurrence, day_of_month: Option<u8>) -> CalendarEvent {
        CalendarEvent {
            id: "event".to_owned(),
            title: "납부".to_owned(),
            description: String::new(),
            kind: CalendarEventKind::Payment,
            start_date: NaiveDate::from_ymd_opt(2026, 1, 15).expect("date"),
            event_time: NaiveTime::from_hms_opt(9, 30, 0),
            recurrence,
            day_of_month,
            ends_on: None,
            created_at: String::new(),
            updated_at: String::new(),
            deleted_at: None,
            version: 1,
        }
    }

    #[test]
    fn monthly_day_31_skips_short_months() {
        let event = recurring(CalendarRecurrence::MonthlyDay, Some(31));
        assert_eq!(
            occurrence_date(
                &event,
                NaiveDate::from_ymd_opt(2026, 2, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 2, 28).expect("end")
            ),
            None
        );
        assert_eq!(
            occurrence_date(
                &event,
                NaiveDate::from_ymd_opt(2026, 3, 1).expect("start"),
                NaiveDate::from_ymd_opt(2026, 3, 31).expect("end")
            ),
            NaiveDate::from_ymd_opt(2026, 3, 31)
        );
    }

    #[test]
    fn monthly_last_day_tracks_actual_month_end() {
        let event = recurring(CalendarRecurrence::MonthlyLastDay, None);
        assert_eq!(
            occurrence_date(
                &event,
                NaiveDate::from_ymd_opt(2028, 2, 1).expect("start"),
                NaiveDate::from_ymd_opt(2028, 2, 29).expect("end")
            ),
            NaiveDate::from_ymd_opt(2028, 2, 29)
        );
    }
}
