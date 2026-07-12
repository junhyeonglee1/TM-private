CREATE TABLE schema_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL
) STRICT;

CREATE TABLE projects (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    description TEXT NOT NULL DEFAULT '',
    color TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    archived_at TEXT,
    deleted_at TEXT
) STRICT;

CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL CHECK (status IN ('inbox', 'todo', 'in_progress', 'blocked', 'done', 'cancelled')),
    priority INTEGER NOT NULL DEFAULT 0 CHECK (priority BETWEEN 0 AND 4),
    due_date TEXT,
    completed_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    CHECK ((status = 'done' AND completed_at IS NOT NULL) OR status <> 'done')
) STRICT;

CREATE TABLE checklist_items (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    body TEXT NOT NULL CHECK (length(trim(body)) > 0),
    is_done INTEGER NOT NULL DEFAULT 0 CHECK (is_done IN (0, 1)),
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    completed_at TEXT
) STRICT;

CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL COLLATE NOCASE UNIQUE CHECK (length(trim(name)) > 0),
    color TEXT,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE task_tags (
    task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (task_id, tag_id)
) WITHOUT ROWID, STRICT;

CREATE TABLE task_day_entries (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    entry_date TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('planned', 'done', 'deferred', 'skipped')),
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    finalized_at TEXT,
    UNIQUE (task_id, entry_date),
    CHECK (
        (status = 'planned' AND finalized_at IS NULL)
        OR
        (status IN ('done', 'deferred', 'skipped') AND finalized_at IS NOT NULL)
    )
) STRICT;

CREATE TABLE task_events (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    event_type TEXT NOT NULL,
    before_json TEXT,
    after_json TEXT,
    actor TEXT NOT NULL DEFAULT 'system',
    created_at TEXT NOT NULL,
    CHECK (before_json IS NULL OR json_valid(before_json)),
    CHECK (after_json IS NULL OR json_valid(after_json))
) STRICT;

CREATE TABLE work_sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id),
    goal TEXT NOT NULL CHECK (length(trim(goal)) > 0),
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'cancelled')),
    started_at TEXT NOT NULL,
    ended_at TEXT,
    result TEXT NOT NULL DEFAULT '',
    blockers TEXT NOT NULL DEFAULT '',
    next_action TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    CHECK ((status = 'running' AND ended_at IS NULL) OR (status <> 'running' AND ended_at IS NOT NULL))
) STRICT;

CREATE TABLE session_tasks (
    session_id TEXT NOT NULL REFERENCES work_sessions(id) ON DELETE CASCADE,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    created_at TEXT NOT NULL,
    PRIMARY KEY (session_id, task_id)
) WITHOUT ROWID, STRICT;

CREATE TABLE worklogs (
    id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES work_sessions(id),
    project_id TEXT REFERENCES projects(id),
    log_date TEXT NOT NULL,
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    body TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
) STRICT;

CREATE TABLE notes (
    id TEXT PRIMARY KEY,
    note_type TEXT NOT NULL CHECK (note_type IN ('concept', 'howto', 'decision', 'reference', 'daily')),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    body TEXT NOT NULL DEFAULT '',
    source_worklog_id TEXT REFERENCES worklogs(id),
    note_date TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
) STRICT;

CREATE TABLE entity_links (
    id TEXT PRIMARY KEY,
    source_type TEXT NOT NULL CHECK (source_type IN ('task', 'session', 'worklog', 'note')),
    source_id TEXT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('task', 'session', 'worklog', 'note', 'file', 'url')),
    target_id TEXT,
    target_value TEXT,
    relation TEXT NOT NULL DEFAULT 'related',
    created_at TEXT NOT NULL,
    CHECK (
        (target_type IN ('task', 'session', 'worklog', 'note') AND target_id IS NOT NULL AND target_value IS NULL)
        OR
        (target_type IN ('file', 'url') AND target_id IS NULL AND target_value IS NOT NULL AND length(trim(target_value)) > 0)
    ),
    UNIQUE (id)
) STRICT;

CREATE TABLE attachments (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL CHECK (owner_type IN ('task', 'session', 'worklog', 'note')),
    owner_id TEXT NOT NULL,
    relative_path TEXT NOT NULL UNIQUE CHECK (length(trim(relative_path)) > 0),
    original_name TEXT NOT NULL,
    media_type TEXT,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64),
    created_at TEXT NOT NULL,
    deleted_at TEXT
) STRICT;

CREATE TABLE digest_deliveries (
    delivery_key TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('morning', 'evening')),
    digest_date TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('claimed', 'sent', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 1 CHECK (attempt_count > 0),
    claimed_at TEXT NOT NULL,
    claim_expires_at TEXT NOT NULL,
    sent_at TEXT,
    slack_ref TEXT,
    failed_at TEXT,
    failure_reason TEXT,
    updated_at TEXT NOT NULL,
    CHECK ((status = 'sent' AND sent_at IS NOT NULL AND slack_ref IS NOT NULL) OR status <> 'sent'),
    UNIQUE (kind, digest_date)
) STRICT;

CREATE TABLE app_state (
    key TEXT PRIMARY KEY,
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    updated_at TEXT NOT NULL
) STRICT;

CREATE INDEX idx_tasks_project_status ON tasks(project_id, status) WHERE deleted_at IS NULL;
CREATE INDEX idx_tasks_due_date ON tasks(due_date) WHERE deleted_at IS NULL AND due_date IS NOT NULL;
CREATE INDEX idx_tasks_deleted_at ON tasks(deleted_at) WHERE deleted_at IS NOT NULL;
CREATE INDEX idx_day_entries_date_status ON task_day_entries(entry_date, status);
CREATE INDEX idx_task_events_task_created ON task_events(task_id, created_at);
CREATE INDEX idx_sessions_status_started ON work_sessions(status, started_at) WHERE deleted_at IS NULL;
CREATE INDEX idx_worklogs_date ON worklogs(log_date) WHERE deleted_at IS NULL;
CREATE INDEX idx_notes_type_date ON notes(note_type, note_date) WHERE deleted_at IS NULL;
CREATE INDEX idx_entity_links_source ON entity_links(source_type, source_id);
CREATE INDEX idx_entity_links_target ON entity_links(target_type, target_id);
CREATE UNIQUE INDEX idx_entity_links_unique
    ON entity_links(
        source_type,
        source_id,
        target_type,
        ifnull(target_id, ''),
        ifnull(target_value, ''),
        relation
    );
CREATE INDEX idx_digest_status_date ON digest_deliveries(status, digest_date);
CREATE UNIQUE INDEX idx_one_running_session
    ON work_sessions((1))
    WHERE status = 'running' AND deleted_at IS NULL;

CREATE VIRTUAL TABLE search_index USING fts5(
    entity_type UNINDEXED,
    entity_id UNINDEXED,
    title,
    body,
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER task_events_no_update
BEFORE UPDATE ON task_events
BEGIN
    SELECT RAISE(ABORT, 'task events are append-only');
END;

CREATE TRIGGER task_events_no_delete
BEFORE DELETE ON task_events
BEGIN
    SELECT RAISE(ABORT, 'task events are append-only');
END;

CREATE TRIGGER finalized_day_entries_no_update
BEFORE UPDATE ON task_day_entries
WHEN OLD.status IN ('done', 'deferred', 'skipped')
BEGIN
    SELECT RAISE(ABORT, 'finalized task day entries are immutable');
END;

CREATE TRIGGER finalized_day_entries_no_delete
BEFORE DELETE ON task_day_entries
WHEN OLD.status IN ('done', 'deferred', 'skipped')
BEGIN
    SELECT RAISE(ABORT, 'finalized task day entries are immutable');
END;

CREATE TRIGGER task_day_entries_identity_immutable
BEFORE UPDATE ON task_day_entries
WHEN OLD.id IS NOT NEW.id
  OR OLD.task_id IS NOT NEW.task_id
  OR OLD.entry_date IS NOT NEW.entry_date
  OR OLD.created_at IS NOT NEW.created_at
BEGIN
    SELECT RAISE(ABORT, 'task day entry identity and date are immutable');
END;

CREATE TRIGGER digest_delivery_identity_immutable
BEFORE UPDATE ON digest_deliveries
WHEN OLD.delivery_key IS NOT NEW.delivery_key
  OR OLD.kind IS NOT NEW.kind
  OR OLD.digest_date IS NOT NEW.digest_date
BEGIN
    SELECT RAISE(ABORT, 'digest delivery identity is immutable');
END;

CREATE TRIGGER sent_digest_deliveries_immutable
BEFORE UPDATE ON digest_deliveries
WHEN OLD.status = 'sent'
BEGIN
    SELECT RAISE(ABORT, 'sent digest deliveries are immutable');
END;

CREATE TRIGGER sent_digest_deliveries_no_delete
BEFORE DELETE ON digest_deliveries
WHEN OLD.status = 'sent'
BEGIN
    SELECT RAISE(ABORT, 'sent digest deliveries cannot be deleted');
END;

CREATE TRIGGER tasks_event_after_insert
AFTER INSERT ON tasks
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.id, 'task.created', NULL,
        json_object(
            'projectId', NEW.project_id,
            'title', NEW.title,
            'description', NEW.description,
            'status', NEW.status,
            'priority', NEW.priority,
            'dueDate', NEW.due_date,
            'completedAt', NEW.completed_at,
            'deletedAt', NEW.deleted_at
        ),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER tasks_event_after_update
AFTER UPDATE ON tasks
WHEN OLD.project_id IS NOT NEW.project_id
  OR OLD.title IS NOT NEW.title
  OR OLD.description IS NOT NEW.description
  OR OLD.status IS NOT NEW.status
  OR OLD.priority IS NOT NEW.priority
  OR OLD.due_date IS NOT NEW.due_date
  OR OLD.completed_at IS NOT NEW.completed_at
  OR OLD.deleted_at IS NOT NEW.deleted_at
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.id, 'task.changed',
        json_object(
            'projectId', OLD.project_id,
            'title', OLD.title,
            'description', OLD.description,
            'status', OLD.status,
            'priority', OLD.priority,
            'dueDate', OLD.due_date,
            'completedAt', OLD.completed_at,
            'deletedAt', OLD.deleted_at
        ),
        json_object(
            'projectId', NEW.project_id,
            'title', NEW.title,
            'description', NEW.description,
            'status', NEW.status,
            'priority', NEW.priority,
            'dueDate', NEW.due_date,
            'completedAt', NEW.completed_at,
            'deletedAt', NEW.deleted_at
        ),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER checklist_event_after_insert
AFTER INSERT ON checklist_items
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.task_id, 'checklist.created', NULL,
        json_object('id', NEW.id, 'body', NEW.body, 'isDone', NEW.is_done, 'sortOrder', NEW.sort_order),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER checklist_event_after_update
AFTER UPDATE ON checklist_items
WHEN OLD.body IS NOT NEW.body
  OR OLD.is_done IS NOT NEW.is_done
  OR OLD.sort_order IS NOT NEW.sort_order
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.task_id, 'checklist.changed',
        json_object('id', OLD.id, 'body', OLD.body, 'isDone', OLD.is_done, 'sortOrder', OLD.sort_order),
        json_object('id', NEW.id, 'body', NEW.body, 'isDone', NEW.is_done, 'sortOrder', NEW.sort_order),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER checklist_event_after_delete
AFTER DELETE ON checklist_items
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), OLD.task_id, 'checklist.deleted',
        json_object('id', OLD.id, 'body', OLD.body, 'isDone', OLD.is_done, 'sortOrder', OLD.sort_order),
        NULL, 'system', tm_now_utc()
    );
END;

CREATE TRIGGER task_tags_event_after_insert
AFTER INSERT ON task_tags
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    SELECT tm_uuid_v7(), NEW.task_id, 'tag.added', NULL,
           json_object('tagId', NEW.tag_id, 'name', tags.name), 'system', tm_now_utc()
    FROM tags WHERE tags.id = NEW.tag_id;
END;

CREATE TRIGGER task_tags_event_after_delete
AFTER DELETE ON task_tags
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    SELECT tm_uuid_v7(), OLD.task_id, 'tag.removed',
           json_object('tagId', OLD.tag_id, 'name', tags.name), NULL, 'system', tm_now_utc()
    FROM tags WHERE tags.id = OLD.tag_id;
END;

CREATE TRIGGER task_day_entries_event_after_insert
AFTER INSERT ON task_day_entries
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.task_id, 'day_entry.created', NULL,
        json_object(
            'id', NEW.id,
            'date', NEW.entry_date,
            'status', NEW.status,
            'note', NEW.note,
            'finalizedAt', NEW.finalized_at
        ),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER task_day_entries_event_after_update
AFTER UPDATE ON task_day_entries
WHEN OLD.status IS NOT NEW.status OR OLD.note IS NOT NEW.note
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.task_id, 'day_entry.changed',
        json_object(
            'id', OLD.id,
            'date', OLD.entry_date,
            'status', OLD.status,
            'note', OLD.note,
            'finalizedAt', OLD.finalized_at
        ),
        json_object(
            'id', NEW.id,
            'date', NEW.entry_date,
            'status', NEW.status,
            'note', NEW.note,
            'finalizedAt', NEW.finalized_at
        ),
        'system', tm_now_utc()
    );
END;

CREATE TRIGGER task_day_entries_event_after_delete
AFTER DELETE ON task_day_entries
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), OLD.task_id, 'day_entry.deleted',
        json_object(
            'id', OLD.id,
            'date', OLD.entry_date,
            'status', OLD.status,
            'note', OLD.note,
            'finalizedAt', OLD.finalized_at
        ),
        NULL, 'system', tm_now_utc()
    );
END;

CREATE TRIGGER session_tasks_event_after_insert
AFTER INSERT ON session_tasks
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), NEW.task_id, 'session.linked', NULL,
        json_object('sessionId', NEW.session_id), 'system', tm_now_utc()
    );
END;

CREATE TRIGGER session_tasks_event_after_delete
AFTER DELETE ON session_tasks
BEGIN
    INSERT INTO task_events(id, task_id, event_type, before_json, after_json, actor, created_at)
    VALUES (
        tm_uuid_v7(), OLD.task_id, 'session.unlinked',
        json_object('sessionId', OLD.session_id), NULL, 'system', tm_now_utc()
    );
END;

CREATE TRIGGER entity_links_validate_insert
BEFORE INSERT ON entity_links
BEGIN
    SELECT CASE
        WHEN NEW.source_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source task does not exist')
        WHEN NEW.source_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source session does not exist')
        WHEN NEW.source_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source worklog does not exist')
        WHEN NEW.source_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source note does not exist')
        WHEN NEW.target_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target task does not exist')
        WHEN NEW.target_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target session does not exist')
        WHEN NEW.target_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target worklog does not exist')
        WHEN NEW.target_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target note does not exist')
    END;
END;

CREATE TRIGGER entity_links_validate_update
BEFORE UPDATE ON entity_links
BEGIN
    SELECT CASE
        WHEN NEW.source_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source task does not exist')
        WHEN NEW.source_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source session does not exist')
        WHEN NEW.source_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source worklog does not exist')
        WHEN NEW.source_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.source_id) THEN RAISE(ABORT, 'source note does not exist')
        WHEN NEW.target_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target task does not exist')
        WHEN NEW.target_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target session does not exist')
        WHEN NEW.target_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target worklog does not exist')
        WHEN NEW.target_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.target_id) THEN RAISE(ABORT, 'target note does not exist')
    END;
END;

CREATE TRIGGER attachments_validate_owner_insert
BEFORE INSERT ON attachments
BEGIN
    SELECT CASE
        WHEN NEW.owner_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment task does not exist')
        WHEN NEW.owner_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment session does not exist')
        WHEN NEW.owner_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment worklog does not exist')
        WHEN NEW.owner_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment note does not exist')
    END;
END;

CREATE TRIGGER attachments_validate_owner_update
BEFORE UPDATE ON attachments
BEGIN
    SELECT CASE
        WHEN NEW.owner_type = 'task' AND NOT EXISTS (SELECT 1 FROM tasks WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment task does not exist')
        WHEN NEW.owner_type = 'session' AND NOT EXISTS (SELECT 1 FROM work_sessions WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment session does not exist')
        WHEN NEW.owner_type = 'worklog' AND NOT EXISTS (SELECT 1 FROM worklogs WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment worklog does not exist')
        WHEN NEW.owner_type = 'note' AND NOT EXISTS (SELECT 1 FROM notes WHERE id = NEW.owner_id) THEN RAISE(ABORT, 'attachment note does not exist')
    END;
END;

CREATE TRIGGER tasks_search_insert AFTER INSERT ON tasks
WHEN NEW.deleted_at IS NULL
BEGIN
    INSERT INTO search_index(entity_type, entity_id, title, body)
    VALUES ('task', NEW.id, NEW.title, NEW.description);
END;

CREATE TRIGGER tasks_search_update AFTER UPDATE ON tasks
BEGIN
    DELETE FROM search_index WHERE entity_type = 'task' AND entity_id = OLD.id;
    INSERT INTO search_index(entity_type, entity_id, title, body)
    SELECT 'task', NEW.id, NEW.title, NEW.description WHERE NEW.deleted_at IS NULL;
END;

CREATE TRIGGER tasks_search_delete AFTER DELETE ON tasks
BEGIN
    DELETE FROM search_index WHERE entity_type = 'task' AND entity_id = OLD.id;
END;

CREATE TRIGGER sessions_search_insert AFTER INSERT ON work_sessions
WHEN NEW.deleted_at IS NULL
BEGIN
    INSERT INTO search_index(entity_type, entity_id, title, body)
    VALUES ('session', NEW.id, NEW.goal, trim(NEW.result || ' ' || NEW.blockers || ' ' || NEW.next_action));
END;

CREATE TRIGGER sessions_search_update AFTER UPDATE ON work_sessions
BEGIN
    DELETE FROM search_index WHERE entity_type = 'session' AND entity_id = OLD.id;
    INSERT INTO search_index(entity_type, entity_id, title, body)
    SELECT 'session', NEW.id, NEW.goal, trim(NEW.result || ' ' || NEW.blockers || ' ' || NEW.next_action)
    WHERE NEW.deleted_at IS NULL;
END;

CREATE TRIGGER sessions_search_delete AFTER DELETE ON work_sessions
BEGIN
    DELETE FROM search_index WHERE entity_type = 'session' AND entity_id = OLD.id;
END;

CREATE TRIGGER worklogs_search_insert AFTER INSERT ON worklogs
WHEN NEW.deleted_at IS NULL
BEGIN
    INSERT INTO search_index(entity_type, entity_id, title, body)
    VALUES ('worklog', NEW.id, NEW.title, NEW.body);
END;

CREATE TRIGGER worklogs_search_update AFTER UPDATE ON worklogs
BEGIN
    DELETE FROM search_index WHERE entity_type = 'worklog' AND entity_id = OLD.id;
    INSERT INTO search_index(entity_type, entity_id, title, body)
    SELECT 'worklog', NEW.id, NEW.title, NEW.body WHERE NEW.deleted_at IS NULL;
END;

CREATE TRIGGER worklogs_search_delete AFTER DELETE ON worklogs
BEGIN
    DELETE FROM search_index WHERE entity_type = 'worklog' AND entity_id = OLD.id;
END;

CREATE TRIGGER notes_search_insert AFTER INSERT ON notes
WHEN NEW.deleted_at IS NULL
BEGIN
    INSERT INTO search_index(entity_type, entity_id, title, body)
    VALUES ('note', NEW.id, NEW.title, NEW.body);
END;

CREATE TRIGGER notes_search_update AFTER UPDATE ON notes
BEGIN
    DELETE FROM search_index WHERE entity_type = 'note' AND entity_id = OLD.id;
    INSERT INTO search_index(entity_type, entity_id, title, body)
    SELECT 'note', NEW.id, NEW.title, NEW.body WHERE NEW.deleted_at IS NULL;
END;

CREATE TRIGGER notes_search_delete AFTER DELETE ON notes
BEGIN
    DELETE FROM search_index WHERE entity_type = 'note' AND entity_id = OLD.id;
END;
