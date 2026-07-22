use chrono::{Datelike, NaiveDate, NaiveTime};
use tempfile::{Builder, TempDir};
use tm_core::{
    CalendarEventKind, CalendarRecurrence, CreateCalendarEventInput, DEFAULT_TM_HOME, Error,
    Result, TmCore, TmHome, UpdateCalendarEventInput,
};

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-calendar-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn input(
    title: &str,
    start_date: NaiveDate,
    recurrence: CalendarRecurrence,
    day_of_month: Option<u8>,
) -> CreateCalendarEventInput {
    CreateCalendarEventInput {
        title: title.to_owned(),
        description: String::new(),
        kind: CalendarEventKind::Payment,
        start_date,
        event_time: None,
        recurrence,
        day_of_month,
        ends_on: None,
    }
}

#[test]
fn expands_monthly_rules_with_start_end_and_short_month_boundaries() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.create_calendar_event(input(
        "31일 납부",
        NaiveDate::from_ymd_opt(2028, 1, 31).expect("date"),
        CalendarRecurrence::MonthlyDay,
        Some(31),
    ))?;
    core.create_calendar_event(input(
        "월말 납부",
        NaiveDate::from_ymd_opt(2028, 1, 31).expect("date"),
        CalendarRecurrence::MonthlyLastDay,
        None,
    ))?;
    core.create_calendar_event(input(
        "월초 확인",
        NaiveDate::from_ymd_opt(2028, 2, 15).expect("date"),
        CalendarRecurrence::MonthlyFirstDay,
        None,
    ))?;

    let february = core.calendar_month(2028, 2)?;
    assert_eq!(
        february.month_end,
        NaiveDate::from_ymd_opt(2028, 2, 29).expect("date")
    );
    assert_eq!(february.occurrences.len(), 1);
    assert_eq!(february.occurrences[0].title, "월말 납부");
    assert_eq!(february.occurrences[0].date, february.month_end);

    let march = core.calendar_month(2028, 3)?;
    assert_eq!(march.occurrences.len(), 3);
    assert!(
        march
            .occurrences
            .iter()
            .any(|item| item.title == "31일 납부" && item.date.day() == 31)
    );
    assert!(
        march
            .occurrences
            .iter()
            .any(|item| item.title == "월말 납부" && item.date.day() == 31)
    );
    assert!(
        march
            .occurrences
            .iter()
            .any(|item| item.title == "월초 확인" && item.date.day() == 1)
    );
    Ok(())
}

#[test]
fn stores_time_and_protects_updates_and_deletes_with_versions() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let created = core.create_calendar_event(CreateCalendarEventInput {
        title: "병원 예약".to_owned(),
        description: "10분 일찍 도착".to_owned(),
        kind: CalendarEventKind::Personal,
        start_date: NaiveDate::from_ymd_opt(2026, 7, 30).expect("date"),
        event_time: NaiveTime::from_hms_opt(9, 30, 0),
        recurrence: CalendarRecurrence::None,
        day_of_month: None,
        ends_on: None,
    })?;
    assert_eq!(created.version, 1);

    let updated = core.update_calendar_event(
        &created.id,
        UpdateCalendarEventInput {
            expected_version: created.version,
            title: "치과 예약".to_owned(),
            description: created.description.clone(),
            kind: created.kind,
            start_date: created.start_date,
            event_time: created.event_time,
            recurrence: created.recurrence,
            day_of_month: created.day_of_month,
            ends_on: created.ends_on,
        },
    )?;
    assert_eq!(updated.version, 2);
    assert!(matches!(
        core.delete_calendar_event(&created.id, created.version),
        Err(Error::Conflict(_))
    ));
    core.delete_calendar_event(&created.id, updated.version)?;
    assert!(core.calendar_month(2026, 7)?.occurrences.is_empty());
    assert_eq!(core.list_calendar_events(true)?.len(), 1);
    Ok(())
}

#[test]
fn rejects_invalid_recurrence_combinations() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let invalid = input(
        "잘못된 일정",
        NaiveDate::from_ymd_opt(2026, 7, 1).expect("date"),
        CalendarRecurrence::MonthlyDay,
        None,
    );
    assert!(matches!(
        core.create_calendar_event(invalid),
        Err(Error::InvalidInput(_))
    ));
    Ok(())
}
