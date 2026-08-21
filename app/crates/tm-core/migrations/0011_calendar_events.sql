CREATE TABLE calendar_events (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL CHECK (length(trim(title)) BETWEEN 1 AND 200),
    description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 4000),
    event_kind TEXT NOT NULL CHECK (event_kind IN ('personal', 'payment')),
    start_date TEXT NOT NULL CHECK (date(start_date) = start_date),
    event_time TEXT CHECK (
        event_time IS NULL OR (
            length(event_time) = 8
            AND event_time GLOB '[0-2][0-9]:[0-5][0-9]:[0-5][0-9]'
            AND substr(event_time, 1, 2) <= '23'
        )
    ),
    recurrence TEXT NOT NULL CHECK (
        recurrence IN ('none', 'monthly_day', 'monthly_first_day', 'monthly_last_day')
    ),
    day_of_month INTEGER CHECK (day_of_month BETWEEN 1 AND 31),
    ends_on TEXT CHECK (ends_on IS NULL OR (date(ends_on) = ends_on AND ends_on >= start_date)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    version INTEGER NOT NULL DEFAULT 1 CHECK (version > 0),
    CHECK (
        (recurrence = 'monthly_day' AND day_of_month IS NOT NULL)
        OR
        (recurrence <> 'monthly_day' AND day_of_month IS NULL)
    ),
    CHECK (recurrence <> 'none' OR ends_on IS NULL)
) STRICT;

CREATE INDEX idx_calendar_events_active_start
    ON calendar_events(start_date, recurrence)
    WHERE deleted_at IS NULL;

CREATE INDEX idx_calendar_events_deleted_at
    ON calendar_events(deleted_at)
    WHERE deleted_at IS NOT NULL;

CREATE TRIGGER calendar_events_identity_immutable
BEFORE UPDATE ON calendar_events
WHEN OLD.id IS NOT NEW.id OR OLD.created_at IS NOT NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'calendar event identity is immutable');
END;
