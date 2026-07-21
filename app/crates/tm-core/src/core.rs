use std::{collections::HashSet, path::Path, str::FromStr};

use chrono::{DateTime, NaiveDate, Utc};
use rusqlite::{
    Connection, OptionalExtension, Transaction, TransactionBehavior, params, types::Type,
};
use serde_json::{Value, json};

use crate::{
    AiBudgetPolicy, AiBudgetReservation, AiBudgetStatus, AiTokenUsage, Attachment, BackupArtifact,
    BackupInfo, ChangeRequest, ChangeRequestClaim, ChangeRequestEvent, ChecklistItem,
    ChecklistMutationInput, CreateAttachmentInput, CreateChangeRequestInput, CreateLinkInput,
    CreateNoteAggregateInput, CreateNoteInput, CreateProjectInput, CreateTaskAggregateInput,
    CreateTaskInput, CreateWorkLogInput, DigestDelivery, DigestKind, DigestPreparation,
    EndSessionInput, EntityLink, EntityType, Error, ExportArtifact, HealthReport, LinkTargetType,
    MigrationDryRun, MigrationManifest, Note, NoteAggregate, NotePatch, NoteType, Project, Result,
    SearchHit, SessionCompletion, SessionStatus, StartSessionInput, Tag, Task, TaskAggregate,
    TaskDayEntry, TaskDayStatus, TaskEvent, TaskPatch, TaskStatus, TmHome, TrashEntityType,
    TrashItem, UpdateChangeRequestInput, UpdateTaskAggregateInput, WorkLog, WorkSession, ai_budget,
    backup, change_request,
    database::{Database, SCHEMA_VERSION, new_id, now_utc, today_seoul},
    digest,
    error::{invalid, not_found},
    export, migration,
};

#[derive(Debug, Clone)]
pub struct TmCore {
    pub(crate) database: Database,
}

impl TmCore {
    pub fn open_from_env() -> Result<Self> {
        Self::open(TmHome::from_env())
    }

    pub fn open(home: TmHome) -> Result<Self> {
        Ok(Self {
            database: Database::open(home)?,
        })
    }

    #[must_use]
    pub fn home(&self) -> &TmHome {
        self.database.home()
    }

    pub fn create_project(&self, input: CreateProjectInput) -> Result<Project> {
        let name = required_text("project name", &input.name)?;
        let now = now_utc();
        let id = new_id();
        let connection = self.database.connect()?;
        connection.execute(
            "INSERT INTO projects(
                id, name, description, color, sort_order, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
            params![id, name, input.description.trim(), input.color, now],
        )?;
        query_project(&connection, &id)
    }

    pub fn list_projects(&self, include_deleted: bool) -> Result<Vec<Project>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, name, description, color, sort_order, created_at, updated_at,
                    archived_at, deleted_at
             FROM projects
             WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY sort_order ASC, name COLLATE NOCASE ASC",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_project)?;
        collect_rows(rows)
    }

    pub fn create_task(&self, input: CreateTaskInput) -> Result<Task> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                create_task_in_transaction(transaction, &input)
            })
    }

    pub fn create_task_aggregate(&self, input: CreateTaskAggregateInput) -> Result<TaskAggregate> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let task = create_task_in_transaction(transaction, &input.task)?;
                let tags = set_task_tags_in_transaction(transaction, &task.id, &input.tags)?;
                Ok(TaskAggregate {
                    task,
                    tags,
                    checklist: Vec::new(),
                })
            })
    }

    pub fn get_task(&self, task_id: &str) -> Result<Task> {
        let connection = self.database.connect()?;
        query_task(&connection, task_id, true)
    }

    pub fn list_tasks(&self, include_deleted: bool) -> Result<Vec<Task>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, title, description, status, priority, due_date,
                    completed_at, created_at, updated_at, deleted_at, version
             FROM tasks
             WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY
                CASE status
                    WHEN 'in_progress' THEN 0
                    WHEN 'blocked' THEN 1
                    WHEN 'todo' THEN 2
                    WHEN 'inbox' THEN 3
                    ELSE 4
                END,
                priority DESC,
                CASE WHEN due_date IS NULL THEN 1 ELSE 0 END,
                due_date ASC,
                created_at DESC",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_task)?;
        collect_rows(rows)
    }

    pub fn update_task(&self, task_id: &str, patch: TaskPatch) -> Result<Task> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                update_task_in_transaction(transaction, task_id, patch)
            })
    }

    pub fn update_task_aggregate(&self, input: UpdateTaskAggregateInput) -> Result<TaskAggregate> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let task = update_task_in_transaction(transaction, &input.task_id, input.patch)?;
                let tags = set_task_tags_in_transaction(transaction, &input.task_id, &input.tags)?;
                let checklist =
                    sync_checklist_in_transaction(transaction, &input.task_id, &input.checklist)?;
                Ok(TaskAggregate {
                    task,
                    tags,
                    checklist,
                })
            })
    }

    pub fn list_task_events(&self, task_id: &str) -> Result<Vec<TaskEvent>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, task_id, event_type, before_json, after_json, actor, created_at
             FROM task_events WHERE task_id = ?1 ORDER BY created_at ASC, id ASC",
        )?;
        let rows = statement.query_map([task_id], map_task_event)?;
        collect_rows(rows)
    }

    pub fn add_checklist_item(&self, task_id: &str, body: &str) -> Result<ChecklistItem> {
        let body = required_text("checklist item", body)?;
        let connection = self.database.connect()?;
        let sort_order: i64 = connection.query_row(
            "SELECT coalesce(max(sort_order), -1) + 1 FROM checklist_items WHERE task_id = ?1",
            [task_id],
            |row| row.get(0),
        )?;
        let id = new_id();
        let now = now_utc();
        connection.execute(
            "INSERT INTO checklist_items(
                id, task_id, body, is_done, sort_order, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?5)",
            params![id, task_id, body, sort_order, now],
        )?;
        query_checklist_item(&connection, &id)
    }

    pub fn update_checklist_item(
        &self,
        item_id: &str,
        body: Option<&str>,
        is_done: Option<bool>,
        sort_order: Option<i64>,
    ) -> Result<ChecklistItem> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                update_checklist_item_in_transaction(
                    transaction,
                    item_id,
                    body,
                    is_done,
                    sort_order,
                )
            })
    }

    pub fn delete_checklist_item(&self, item_id: &str) -> Result<()> {
        let connection = self.database.connect()?;
        let changed = connection.execute("DELETE FROM checklist_items WHERE id = ?1", [item_id])?;
        if changed == 0 {
            return Err(not_found("checklist item", item_id));
        }
        Ok(())
    }

    pub fn list_checklist_items(&self, task_id: &str) -> Result<Vec<ChecklistItem>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, task_id, body, is_done, sort_order, created_at, updated_at, completed_at, version
             FROM checklist_items WHERE task_id = ?1 ORDER BY sort_order ASC, created_at ASC",
        )?;
        let rows = statement.query_map([task_id], map_checklist_item)?;
        collect_rows(rows)
    }

    pub fn create_tag(&self, name: &str, color: Option<&str>) -> Result<Tag> {
        let name = required_text("tag name", name)?;
        let connection = self.database.connect()?;
        let id = new_id();
        connection.execute(
            "INSERT INTO tags(id, name, color, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(name) DO UPDATE SET color = coalesce(excluded.color, tags.color)",
            params![id, name, color, now_utc()],
        )?;
        connection
            .query_row(
                "SELECT id, name, color, created_at FROM tags WHERE name = ?1 COLLATE NOCASE",
                [name],
                map_tag,
            )
            .map_err(Into::into)
    }

    pub fn set_task_tags(&self, task_id: &str, names: &[String]) -> Result<Vec<Tag>> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                set_task_tags_in_transaction(transaction, task_id, names)
            })
    }

    pub fn list_tags(&self, task_id: Option<&str>) -> Result<Vec<Tag>> {
        let connection = self.database.connect()?;
        list_tags_for_connection(&connection, task_id)
    }

    pub fn plan_task(
        &self,
        task_id: &str,
        date: NaiveDate,
        note: Option<&str>,
    ) -> Result<TaskDayEntry> {
        self.database.transaction(TransactionBehavior::Immediate, |transaction| {
            let task = query_task(transaction, task_id, true)?;
            if task.deleted_at.is_some() {
                return Err(Error::Conflict(format!(
                    "task {task_id} is in the trash and cannot be planned"
                )));
            }
            let existing = transaction
                .query_row(
                    "SELECT id, task_id, entry_date, status, note, created_at, updated_at, finalized_at
                     FROM task_day_entries WHERE task_id = ?1 AND entry_date = ?2",
                    params![task_id, date],
                    map_day_entry,
                )
                .optional()?;
            if let Some(existing) = existing {
                if existing.status != TaskDayStatus::Planned {
                    return Err(Error::Conflict(format!(
                        "day entry {} is finalized as {} and cannot become planned",
                        existing.id, existing.status
                    )));
                }
                return Ok(existing);
            }
            let id = new_id();
            let now = now_utc();
            transaction.execute(
                "INSERT INTO task_day_entries(
                    id, task_id, entry_date, status, note, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'planned', ?4, ?5, ?5)",
                params![id, task_id, date, note.unwrap_or_default().trim(), now],
            )?;
            query_day_entry(transaction, &id)
        })
    }

    pub fn finalize_day_entry(
        &self,
        entry_id: &str,
        status: TaskDayStatus,
        note: Option<&str>,
    ) -> Result<TaskDayEntry> {
        if status == TaskDayStatus::Planned {
            return Err(invalid("final status cannot be planned"));
        }
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_day_entry(transaction, entry_id)?;
                if current.status != TaskDayStatus::Planned {
                    return Err(Error::Conflict(format!(
                        "day entry {entry_id} is already finalized as {}",
                        current.status
                    )));
                }
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE task_day_entries
                 SET status = ?2, note = ?3, finalized_at = ?4, updated_at = ?4
                 WHERE id = ?1 AND status = 'planned'",
                    params![
                        entry_id,
                        status.as_str(),
                        note.unwrap_or(&current.note).trim(),
                        now,
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(format!(
                        "day entry {entry_id} changed concurrently"
                    )));
                }
                if status == TaskDayStatus::Done {
                    transaction.execute(
                        "UPDATE tasks
                     SET status = 'done', completed_at = coalesce(completed_at, ?2),
                         updated_at = ?2, version = version + 1
                     WHERE id = ?1 AND status NOT IN ('done', 'cancelled')",
                        params![current.task_id, now],
                    )?;
                }
                query_day_entry(transaction, entry_id)
            })
    }

    pub fn carry_over_day_entry(
        &self,
        entry_id: &str,
        target_date: NaiveDate,
    ) -> Result<(TaskDayEntry, TaskDayEntry)> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_day_entry(transaction, entry_id)?;
                if current.status != TaskDayStatus::Planned {
                    return Err(Error::Conflict(format!(
                        "only planned entries can be deferred; {entry_id} is {}",
                        current.status
                    )));
                }
                if target_date <= current.entry_date {
                    return Err(invalid(
                        "carry-over target date must be later than the source date",
                    ));
                }
                let now = now_utc();
                let changed = transaction.execute(
                    "UPDATE task_day_entries
                 SET status = 'deferred', finalized_at = ?2, updated_at = ?2
                 WHERE id = ?1 AND status = 'planned'",
                    params![entry_id, now],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(format!(
                        "day entry {entry_id} changed concurrently"
                    )));
                }
                let new_id = new_id();
                transaction.execute(
                    "INSERT INTO task_day_entries(
                    id, task_id, entry_date, status, note, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'planned', '', ?4, ?4)",
                    params![new_id, current.task_id, target_date, now],
                )?;
                Ok((
                    query_day_entry(transaction, entry_id)?,
                    query_day_entry(transaction, &new_id)?,
                ))
            })
    }

    pub fn list_day_entries(
        &self,
        from: Option<NaiveDate>,
        through: Option<NaiveDate>,
    ) -> Result<Vec<TaskDayEntry>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, task_id, entry_date, status, note, created_at, updated_at, finalized_at
             FROM task_day_entries
             WHERE (?1 IS NULL OR entry_date >= ?1)
               AND (?2 IS NULL OR entry_date <= ?2)
             ORDER BY entry_date DESC, created_at ASC",
        )?;
        let rows = statement.query_map(params![from, through], map_day_entry)?;
        collect_rows(rows)
    }

    pub fn start_session(&self, input: StartSessionInput) -> Result<WorkSession> {
        let goal = required_text("session goal", &input.goal)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let id = new_id();
                let now = now_utc();
                transaction.execute(
                    "INSERT INTO work_sessions(
                    id, project_id, goal, status, started_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'running', ?4, ?4, ?4)",
                    params![id, input.project_id, goal, now],
                )?;
                for task_id in input.task_ids.iter().collect::<HashSet<_>>() {
                    query_task(transaction, task_id, false)?;
                    transaction.execute(
                        "INSERT INTO session_tasks(session_id, task_id, created_at)
                     VALUES (?1, ?2, ?3)",
                        params![id, task_id, now],
                    )?;
                }
                query_session(transaction, &id)
            })
    }

    pub fn end_session(
        &self,
        session_id: &str,
        input: EndSessionInput,
    ) -> Result<SessionCompletion> {
        let result = required_text("session result", &input.result)?;
        let followup_title = if input.create_followup_task {
            Some(required_text(
                "follow-up task title",
                input
                    .followup_title
                    .as_deref()
                    .unwrap_or(input.next_action.as_str()),
            )?)
        } else {
            None
        };

        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let current = query_session(transaction, session_id)?;
                if current.status != SessionStatus::Running {
                    return Err(Error::Conflict(format!(
                        "session {session_id} is {}, not running",
                        current.status
                    )));
                }
                let ended_at = now_utc();
                let changed = transaction.execute(
                    "UPDATE work_sessions
                 SET status = 'completed', ended_at = ?2, result = ?3,
                     blockers = ?4, next_action = ?5, updated_at = ?2
                 WHERE id = ?1 AND status = 'running'",
                    params![
                        session_id,
                        ended_at,
                        result,
                        input.blockers.trim(),
                        input.next_action.trim(),
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::Conflict(format!(
                        "session {session_id} changed concurrently"
                    )));
                }

                let worklog_id = new_id();
                let worklog_title = format!("세션 종료: {}", current.goal);
                let worklog_body = format!(
                    "## 결과\n{}\n\n## 막힌 점\n{}\n\n## 다음 행동\n{}",
                    result,
                    input.blockers.trim(),
                    input.next_action.trim()
                );
                let log_date = today_seoul();
                transaction.execute(
                    "INSERT INTO worklogs(
                    id, session_id, project_id, log_date, title, body, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        worklog_id,
                        session_id,
                        current.project_id,
                        log_date,
                        worklog_title,
                        worklog_body,
                        ended_at,
                    ],
                )?;
                let task_ids = list_session_task_ids(transaction, session_id)?;
                for task_id in &task_ids {
                    insert_entity_link(
                        transaction,
                        EntityType::Worklog,
                        &worklog_id,
                        LinkTargetType::Task,
                        Some(task_id),
                        None,
                        "documents",
                    )?;
                }

                let followup_task = if let Some(title) = followup_title {
                    let task = create_task_in_transaction(
                        transaction,
                        &CreateTaskInput {
                            project_id: input
                                .followup_project_id
                                .clone()
                                .or_else(|| current.project_id.clone()),
                            title,
                            description: format!("세션 후속 행동: {}", input.next_action.trim()),
                            status: TaskStatus::Todo,
                            priority: 0,
                            due_date: None,
                        },
                    )?;
                    insert_entity_link(
                        transaction,
                        EntityType::Session,
                        session_id,
                        LinkTargetType::Task,
                        Some(&task.id),
                        None,
                        "created_followup",
                    )?;
                    Some(task)
                } else {
                    None
                };

                Ok(SessionCompletion {
                    session: query_session(transaction, session_id)?,
                    worklog: query_worklog(transaction, &worklog_id)?,
                    followup_task,
                })
            })
    }

    pub fn list_sessions(&self, include_deleted: bool) -> Result<Vec<WorkSession>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, project_id, goal, status, started_at, ended_at, result,
                    blockers, next_action, created_at, updated_at, deleted_at
             FROM work_sessions
             WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY started_at DESC",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_session)?;
        collect_rows(rows)
    }

    pub fn session_task_ids(&self, session_id: &str) -> Result<Vec<String>> {
        let connection = self.database.connect()?;
        query_session(&connection, session_id)?;
        list_session_task_ids(&connection, session_id)
    }

    pub fn create_worklog(&self, input: CreateWorkLogInput) -> Result<WorkLog> {
        let title = required_text("work log title", &input.title)?;
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                if let Some(session_id) = input.session_id.as_deref() {
                    query_session(transaction, session_id)?;
                }
                let id = new_id();
                let now = now_utc();
                transaction.execute(
                    "INSERT INTO worklogs(
                    id, session_id, project_id, log_date, title, body, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        id,
                        input.session_id,
                        input.project_id,
                        input.log_date.unwrap_or_else(today_seoul),
                        title,
                        input.body.trim(),
                        now,
                    ],
                )?;
                for task_id in input.task_ids.iter().collect::<HashSet<_>>() {
                    insert_entity_link(
                        transaction,
                        EntityType::Worklog,
                        &id,
                        LinkTargetType::Task,
                        Some(task_id),
                        None,
                        "documents",
                    )?;
                }
                query_worklog(transaction, &id)
            })
    }

    pub fn list_worklogs(&self, include_deleted: bool) -> Result<Vec<WorkLog>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, session_id, project_id, log_date, title, body,
                    created_at, updated_at, deleted_at
             FROM worklogs WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY log_date DESC, created_at DESC",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_worklog)?;
        collect_rows(rows)
    }

    pub fn create_note(&self, input: CreateNoteInput) -> Result<Note> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                create_note_in_transaction(transaction, &input)
            })
    }

    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        let connection = self.database.connect()?;
        query_note(&connection, note_id)
    }

    pub fn update_note(&self, note_id: &str, patch: NotePatch) -> Result<Note> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                update_note_in_transaction(transaction, note_id, patch)
            })
    }

    pub fn create_note_aggregate(&self, input: CreateNoteAggregateInput) -> Result<NoteAggregate> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let note = create_note_in_transaction(transaction, &input.note)?;
                let mut links = Vec::new();
                for task_id in &input.links.task_ids {
                    links.push(create_note_link_in_transaction(
                        transaction,
                        &note.id,
                        LinkTargetType::Task,
                        Some(task_id),
                        None,
                    )?);
                }
                for session_id in &input.links.session_ids {
                    links.push(create_note_link_in_transaction(
                        transaction,
                        &note.id,
                        LinkTargetType::Session,
                        Some(session_id),
                        None,
                    )?);
                }
                for worklog_id in &input.links.worklog_ids {
                    links.push(create_note_link_in_transaction(
                        transaction,
                        &note.id,
                        LinkTargetType::Worklog,
                        Some(worklog_id),
                        None,
                    )?);
                }
                for file_path in &input.links.file_paths {
                    links.push(create_note_link_in_transaction(
                        transaction,
                        &note.id,
                        LinkTargetType::File,
                        None,
                        Some(file_path),
                    )?);
                }
                for url in &input.links.urls {
                    links.push(create_note_link_in_transaction(
                        transaction,
                        &note.id,
                        LinkTargetType::Url,
                        None,
                        Some(url),
                    )?);
                }
                Ok(NoteAggregate { note, links })
            })
    }

    pub fn promote_worklog(
        &self,
        worklog_id: &str,
        note_type: NoteType,
        title: Option<&str>,
    ) -> Result<Note> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let worklog = query_worklog(transaction, worklog_id)?;
                let note_id = new_id();
                let now = now_utc();
                let note_title = title
                    .map(|value| required_text("note title", value))
                    .transpose()?
                    .unwrap_or_else(|| worklog.title.clone());
                transaction.execute(
                    "INSERT INTO notes(
                    id, note_type, title, body, source_worklog_id, note_date,
                    created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![
                        note_id,
                        note_type.as_str(),
                        note_title,
                        worklog.body,
                        worklog_id,
                        worklog.log_date,
                        now,
                    ],
                )?;
                insert_entity_link(
                    transaction,
                    EntityType::Note,
                    &note_id,
                    LinkTargetType::Worklog,
                    Some(worklog_id),
                    None,
                    "promoted_from",
                )?;
                let mut statement = transaction.prepare(
                    "SELECT target_id FROM entity_links
                 WHERE source_type = 'worklog' AND source_id = ?1 AND target_type = 'task'",
                )?;
                let task_ids = statement
                    .query_map([worklog_id], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                drop(statement);
                for task_id in task_ids {
                    insert_entity_link(
                        transaction,
                        EntityType::Note,
                        &note_id,
                        LinkTargetType::Task,
                        Some(&task_id),
                        None,
                        "documents",
                    )?;
                }
                query_note(transaction, &note_id)
            })
    }

    pub fn list_notes(&self, include_deleted: bool) -> Result<Vec<Note>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, note_type, title, body, source_worklog_id, note_date,
                    created_at, updated_at, deleted_at, version
             FROM notes WHERE (?1 = 1 OR deleted_at IS NULL)
             ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([i64::from(include_deleted)], map_note)?;
        collect_rows(rows)
    }

    pub fn create_link(&self, input: CreateLinkInput) -> Result<EntityLink> {
        validate_link_target(
            input.target_type,
            input.target_id.as_deref(),
            input.target_value.as_deref(),
        )?;
        let relation = required_text("link relation", &input.relation)?;
        let connection = self.database.connect()?;
        let id = insert_entity_link(
            &connection,
            input.source_type,
            &input.source_id,
            input.target_type,
            input.target_id.as_deref(),
            input.target_value.as_deref(),
            &relation,
        )?;
        query_link(&connection, &id)
    }

    pub fn list_links(
        &self,
        source_type: Option<EntityType>,
        source_id: Option<&str>,
    ) -> Result<Vec<EntityLink>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, source_type, source_id, target_type, target_id,
                    target_value, relation, created_at
             FROM entity_links
             WHERE (?1 IS NULL OR source_type = ?1)
               AND (?2 IS NULL OR source_id = ?2)
             ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map(
            params![source_type.map(EntityType::as_str), source_id],
            map_link,
        )?;
        collect_rows(rows)
    }

    pub fn register_attachment(&self, input: CreateAttachmentInput) -> Result<Attachment> {
        let relative = Path::new(&input.relative_path);
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            return Err(invalid(
                "attachment path must be relative and remain inside data/attachments",
            ));
        }
        let relative_path = required_text("attachment relative path", &input.relative_path)?;
        let original_name = required_text("attachment original name", &input.original_name)?;
        if input.sha256.len() != 64 || !input.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(invalid(
                "attachment SHA-256 must be 64 hexadecimal characters",
            ));
        }
        let byte_size = i64::try_from(input.byte_size)
            .map_err(|_| invalid("attachment is too large for SQLite metadata"))?;
        let connection = self.database.connect()?;
        let id = new_id();
        connection.execute(
            "INSERT INTO attachments(
                id, owner_type, owner_id, relative_path, original_name,
                media_type, byte_size, sha256, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, lower(?8), ?9)",
            params![
                id,
                input.owner_type.as_str(),
                input.owner_id,
                relative_path,
                original_name,
                input.media_type,
                byte_size,
                input.sha256,
                now_utc(),
            ],
        )?;
        query_attachment(&connection, &id)
    }

    pub fn list_attachments(
        &self,
        owner_type: Option<EntityType>,
        owner_id: Option<&str>,
        include_deleted: bool,
    ) -> Result<Vec<Attachment>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, owner_type, owner_id, relative_path, original_name,
                    media_type, byte_size, sha256, created_at, deleted_at
             FROM attachments
             WHERE (?1 IS NULL OR owner_type = ?1)
               AND (?2 IS NULL OR owner_id = ?2)
               AND (?3 = 1 OR deleted_at IS NULL)
             ORDER BY created_at ASC",
        )?;
        let rows = statement.query_map(
            params![
                owner_type.map(EntityType::as_str),
                owner_id,
                i64::from(include_deleted)
            ],
            map_attachment,
        )?;
        collect_rows(rows)
    }

    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let match_query = fts_query(query)?;
        let capped_limit =
            i64::try_from(limit.clamp(1, 200)).map_err(|_| invalid("search limit is too large"))?;
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT entity_type, entity_id, title,
                    snippet(search_index, 3, '<mark>', '</mark>', ' … ', 24),
                    CAST(round(bm25(search_index) * 1000.0) AS INTEGER)
             FROM search_index
             WHERE search_index MATCH ?1
             ORDER BY bm25(search_index), entity_type, entity_id
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![match_query, capped_limit], |row| {
            let entity: String = row.get(0)?;
            Ok(SearchHit {
                entity_type: enum_from_sql(0, &entity)?,
                entity_id: row.get(1)?,
                title: row.get(2)?,
                excerpt: row.get(3)?,
                rank_millis: row.get(4)?,
            })
        })?;
        collect_rows(rows)
    }

    pub fn move_to_trash(&self, entity_type: TrashEntityType, id: &str) -> Result<()> {
        self.set_deleted_at(entity_type, id, Some(now_utc()))
    }

    pub fn restore_from_trash(&self, entity_type: TrashEntityType, id: &str) -> Result<()> {
        self.set_deleted_at(entity_type, id, None)
    }

    pub fn list_trash(&self) -> Result<Vec<TrashItem>> {
        let connection = self.database.connect()?;
        let mut statement = connection.prepare(
            "SELECT id, entity_type, title, deleted_at FROM (
                SELECT id, 'task' AS entity_type, title, deleted_at FROM tasks WHERE deleted_at IS NOT NULL
                UNION ALL
                SELECT id, 'note', title, deleted_at FROM notes WHERE deleted_at IS NOT NULL
                UNION ALL
                SELECT id, 'project', name, deleted_at FROM projects WHERE deleted_at IS NOT NULL
                UNION ALL
                SELECT id, 'worklog', title, deleted_at FROM worklogs WHERE deleted_at IS NOT NULL
                UNION ALL
                SELECT id, 'session', goal, deleted_at FROM work_sessions WHERE deleted_at IS NOT NULL
             ) ORDER BY deleted_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            let kind: String = row.get(1)?;
            let entity_type = match kind.as_str() {
                "task" => TrashEntityType::Task,
                "note" => TrashEntityType::Note,
                "project" => TrashEntityType::Project,
                "worklog" => TrashEntityType::Worklog,
                "session" => TrashEntityType::Session,
                _ => return Err(enum_conversion_error(1, kind)),
            };
            Ok(TrashItem {
                id: row.get(0)?,
                entity_type,
                title: row.get(2)?,
                deleted_at: row.get(3)?,
            })
        })?;
        collect_rows(rows)
    }

    fn set_deleted_at(
        &self,
        entity_type: TrashEntityType,
        id: &str,
        deleted_at: Option<String>,
    ) -> Result<()> {
        let (table, title_column, version_update) = match entity_type {
            TrashEntityType::Task => ("tasks", "title", ", version = version + 1"),
            TrashEntityType::Note => ("notes", "title", ", version = version + 1"),
            TrashEntityType::Project => ("projects", "name", ""),
            TrashEntityType::Worklog => ("worklogs", "title", ""),
            TrashEntityType::Session => ("work_sessions", "goal", ""),
        };
        let connection = self.database.connect()?;
        let sql = format!(
            "UPDATE {table} SET deleted_at = ?2, updated_at = ?3{version_update} WHERE id = ?1 AND {title_column} IS NOT NULL"
        );
        let changed = connection.execute(&sql, params![id, deleted_at, now_utc()])?;
        if changed == 0 {
            return Err(not_found(entity_type.as_str(), id));
        }
        Ok(())
    }

    pub fn create_change_request(&self, input: CreateChangeRequestInput) -> Result<ChangeRequest> {
        change_request::create(&self.database, input)
    }

    pub fn update_change_request(
        &self,
        request_id: &str,
        input: UpdateChangeRequestInput,
    ) -> Result<ChangeRequest> {
        change_request::update(&self.database, request_id, input)
    }

    pub fn approve_change_request(
        &self,
        request_id: &str,
        expected_revision: u32,
        expected_attempt_count: u32,
        approved_by: &str,
    ) -> Result<ChangeRequest> {
        change_request::approve(
            &self.database,
            request_id,
            expected_revision,
            expected_attempt_count,
            approved_by,
        )
    }

    pub fn return_change_request_to_draft(
        &self,
        request_id: &str,
        expected_revision: u32,
        expected_attempt_count: u32,
        returned_by: &str,
    ) -> Result<ChangeRequest> {
        change_request::return_to_draft(
            &self.database,
            request_id,
            expected_revision,
            expected_attempt_count,
            returned_by,
        )
    }

    pub fn cancel_change_request(
        &self,
        request_id: &str,
        expected_revision: u32,
        expected_attempt_count: u32,
        cancellation_reason: &str,
        cancelled_by: &str,
    ) -> Result<ChangeRequest> {
        change_request::cancel(
            &self.database,
            request_id,
            expected_revision,
            expected_attempt_count,
            cancellation_reason,
            cancelled_by,
        )
    }

    pub fn claim_next_change_request(&self, worker_id: &str) -> Result<ChangeRequestClaim> {
        change_request::claim_next(&self.database, worker_id)
    }

    pub fn complete_change_request(
        &self,
        request_id: &str,
        claim_key: &str,
        result_summary: &str,
        patch_ref: Option<&str>,
        worker_id: &str,
    ) -> Result<ChangeRequest> {
        change_request::complete(
            &self.database,
            request_id,
            claim_key,
            result_summary,
            patch_ref,
            worker_id,
        )
    }

    pub fn fail_change_request(
        &self,
        request_id: &str,
        claim_key: &str,
        failure_reason: &str,
        worker_id: &str,
    ) -> Result<ChangeRequest> {
        change_request::fail(
            &self.database,
            request_id,
            claim_key,
            failure_reason,
            worker_id,
        )
    }

    pub fn abandon_change_request(
        &self,
        request_id: &str,
        expected_revision: u32,
        expected_attempt_count: u32,
        failure_reason: &str,
        abandoned_by: &str,
    ) -> Result<ChangeRequest> {
        change_request::abandon(
            &self.database,
            request_id,
            expected_revision,
            expected_attempt_count,
            failure_reason,
            abandoned_by,
        )
    }

    pub fn list_change_requests(&self) -> Result<Vec<ChangeRequest>> {
        change_request::list(&self.database)
    }

    pub fn list_change_request_events(&self, request_id: &str) -> Result<Vec<ChangeRequestEvent>> {
        change_request::list_events(&self.database, request_id)
    }

    pub fn prepare_digest(&self, kind: DigestKind, date: NaiveDate) -> Result<DigestPreparation> {
        digest::prepare(&self.database, kind, date)
    }

    pub fn complete_digest(&self, delivery_key: &str, slack_ref: &str) -> Result<DigestDelivery> {
        digest::complete(&self.database, delivery_key, slack_ref)
    }

    pub fn fail_digest(&self, delivery_key: &str, reason: &str) -> Result<DigestDelivery> {
        digest::fail(&self.database, delivery_key, reason)
    }

    pub fn digest_status(&self) -> Result<Vec<DigestDelivery>> {
        digest::status(&self.database)
    }

    pub fn create_backup(&self) -> Result<BackupArtifact> {
        backup::create_database_backup(
            &self.home().database_path(),
            &self.home().database_backups_dir(),
            "manual",
        )
    }

    pub fn ai_budget_status(&self, policy: AiBudgetPolicy) -> Result<AiBudgetStatus> {
        ai_budget::status(&self.database, policy)
    }

    pub fn reserve_ai_budget(
        &self,
        request_id: &str,
        provider: &str,
        model: &str,
        operation: &str,
        maximum_cost_microusd: u64,
        policy: AiBudgetPolicy,
    ) -> Result<AiBudgetReservation> {
        ai_budget::reserve(
            &self.database,
            request_id,
            provider,
            model,
            operation,
            maximum_cost_microusd,
            policy,
        )
    }

    pub fn settle_ai_budget(
        &self,
        reservation: &AiBudgetReservation,
        actual_cost_microusd: u64,
        usage: Option<AiTokenUsage>,
        outcome: &str,
        policy: AiBudgetPolicy,
    ) -> Result<AiBudgetStatus> {
        ai_budget::settle(
            &self.database,
            reservation,
            actual_cost_microusd,
            usage,
            outcome,
            policy,
        )
    }

    pub fn create_startup_backup(&self) -> Result<BackupArtifact> {
        backup::create_database_backup(
            &self.home().database_path(),
            &self.home().database_backups_dir(),
            "startup",
        )
    }

    pub fn create_shutdown_backup(&self) -> Result<BackupArtifact> {
        backup::create_database_backup(
            &self.home().database_path(),
            &self.home().database_backups_dir(),
            "shutdown",
        )
    }

    pub fn create_daily_backup_if_due(&self) -> Result<Option<BackupArtifact>> {
        let date = today_seoul();
        let claim_id = new_id();
        let claimed = self.database.transaction(TransactionBehavior::Immediate, |transaction| {
            let value = json!({
                "date": date,
                "status": "claimed",
                "claimId": claim_id,
            });
            let changed = transaction.execute(
                "INSERT INTO app_state(key, value_json, updated_at)
                 VALUES ('last_daily_backup', ?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                 WHERE json_extract(app_state.value_json, '$.date') <> json_extract(excluded.value_json, '$.date')",
                params![value.to_string(), now_utc()],
            )?;
            Ok(changed == 1)
        })?;
        if !claimed {
            return Ok(None);
        }
        let result = backup::create_database_backup(
            &self.home().database_path(),
            &self.home().database_backups_dir(),
            "daily",
        );
        match result {
            Ok(artifact) => {
                self.database
                    .transaction(TransactionBehavior::Immediate, |transaction| {
                        let value = json!({
                            "date": date,
                            "status": "completed",
                            "claimId": claim_id,
                            "path": artifact.path,
                            "sha256": artifact.sha256,
                        });
                        transaction.execute(
                            "UPDATE app_state SET value_json = ?1, updated_at = ?2
                         WHERE key = 'last_daily_backup'
                           AND json_extract(value_json, '$.claimId') = ?3",
                            params![value.to_string(), now_utc(), claim_id],
                        )?;
                        Ok(())
                    })?;
                Ok(Some(artifact))
            }
            Err(error) => {
                self.database
                    .transaction(TransactionBehavior::Immediate, |transaction| {
                        transaction.execute(
                            "DELETE FROM app_state
                         WHERE key = 'last_daily_backup'
                           AND json_extract(value_json, '$.claimId') = ?1",
                            [claim_id],
                        )?;
                        Ok(())
                    })?;
                Err(error)
            }
        }
    }

    pub fn create_source_snapshot(&self) -> Result<BackupArtifact> {
        backup::create_source_snapshot(&self.home().app_dir(), &self.home().source_backups_dir())
    }

    pub fn restore_backup(&self, backup_path: impl AsRef<Path>) -> Result<BackupArtifact> {
        self.database.checkpoint()?;
        backup::restore_database(
            &self.home().database_path(),
            &self.home().database_backups_dir(),
            backup_path.as_ref(),
        )
    }

    pub fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        let mut backups = Vec::new();
        for entry in std::fs::read_dir(self.home().database_backups_dir())? {
            let entry = entry?;
            let path = entry.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "sqlite3")
            {
                continue;
            }
            let metadata = entry.metadata()?;
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let trigger = backup_trigger(&file_name).to_owned();
            let created: DateTime<Utc> = metadata.modified()?.into();
            backups.push(BackupInfo {
                path: path.to_string_lossy().into_owned(),
                file_name,
                created_at: created.to_rfc3339(),
                byte_size: metadata.len(),
                trigger,
            });
        }
        backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(backups)
    }

    pub fn export_json(&self) -> Result<Value> {
        export::json_snapshot(&self.database)
    }

    pub fn export_markdown(&self) -> Result<String> {
        export::markdown_snapshot(&self.database)
    }

    pub fn create_export(&self) -> Result<ExportArtifact> {
        export::create_artifacts(&self.database)
    }

    pub fn migration_manifest(&self) -> Result<MigrationManifest> {
        migration::manifest(&self.database)
    }

    pub fn migration_dry_run(&self) -> Result<MigrationDryRun> {
        migration::dry_run(&self.database)
    }

    pub fn inspect_migration_database(path: impl AsRef<Path>) -> Result<MigrationManifest> {
        migration::inspect_database(path.as_ref())
    }

    pub fn database_initialized_at(&self) -> Result<String> {
        let connection = self.database.connect()?;
        connection
            .query_row("SELECT MIN(applied_at) FROM schema_migrations", [], |row| {
                row.get::<_, Option<String>>(0)
            })?
            .ok_or_else(|| {
                Error::Invariant("database initialization timestamp is missing".to_owned())
            })
    }

    pub fn health(&self) -> Result<HealthReport> {
        let connection = self.database.connect()?;
        let schema_version: i64 =
            connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let journal_mode: String =
            connection.pragma_query_value(None, "journal_mode", |row| row.get(0))?;
        let foreign_keys: i64 =
            connection.pragma_query_value(None, "foreign_keys", |row| row.get(0))?;
        let integrity_check: String =
            connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
        let sqlite_version: String =
            connection.query_row("SELECT sqlite_version()", [], |row| row.get(0))?;
        Ok(HealthReport {
            ok: schema_version == SCHEMA_VERSION
                && foreign_keys == 1
                && integrity_check == "ok"
                && journal_mode.eq_ignore_ascii_case("wal"),
            database_path: self.home().database_path().to_string_lossy().into_owned(),
            schema_version,
            sqlite_version,
            journal_mode,
            foreign_keys: foreign_keys == 1,
            integrity_check,
            checked_at: now_utc(),
        })
    }
}

fn required_text(field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(invalid(format!("{field} cannot be empty")));
    }
    Ok(value.to_owned())
}

pub(crate) fn update_task_in_transaction(
    transaction: &Transaction<'_>,
    task_id: &str,
    patch: TaskPatch,
) -> Result<Task> {
    if patch.clear_project && patch.project_id.is_some() {
        return Err(invalid("project cannot be both set and cleared"));
    }
    if patch.clear_due_date && patch.due_date.is_some() {
        return Err(invalid("due date cannot be both set and cleared"));
    }
    if patch.priority.is_some_and(|priority| priority > 4) {
        return Err(invalid("task priority must be between 0 and 4"));
    }

    let current = query_task(transaction, task_id, true)?;
    if current.deleted_at.is_some() {
        return Err(Error::Conflict(format!(
            "task {task_id} is in the trash; restore it before editing"
        )));
    }
    let title = patch
        .title
        .as_deref()
        .map(|value| required_text("task title", value))
        .transpose()?
        .unwrap_or(current.title);
    let description = patch.description.unwrap_or(current.description);
    let project_id = if patch.clear_project {
        None
    } else {
        patch.project_id.or(current.project_id)
    };
    let due_date = if patch.clear_due_date {
        None
    } else {
        patch.due_date.or(current.due_date)
    };
    let status = patch.status.unwrap_or(current.status);
    let priority = patch.priority.unwrap_or(current.priority);
    let completed_at = if status == TaskStatus::Done {
        current.completed_at.or_else(|| Some(now_utc()))
    } else {
        None
    };
    transaction.execute(
        "UPDATE tasks
         SET project_id = ?2, title = ?3, description = ?4, status = ?5,
             priority = ?6, due_date = ?7, completed_at = ?8, updated_at = ?9,
             version = version + 1
         WHERE id = ?1",
        params![
            task_id,
            project_id,
            title,
            description.trim(),
            status.as_str(),
            priority,
            due_date,
            completed_at,
            now_utc(),
        ],
    )?;
    query_task(transaction, task_id, true)
}

fn set_task_tags_in_transaction(
    transaction: &Transaction<'_>,
    task_id: &str,
    names: &[String],
) -> Result<Vec<Tag>> {
    query_task(transaction, task_id, true)?;
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();
    for raw_name in names {
        let name = required_text("tag name", raw_name)?;
        if name.chars().count() > 100 {
            return Err(invalid("tag name cannot exceed 100 characters"));
        }
        if seen.insert(name.to_lowercase()) {
            normalized.push(name);
        }
    }

    let mut tag_ids = Vec::with_capacity(normalized.len());
    for name in &normalized {
        let existing: Option<String> = transaction
            .query_row(
                "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        let id = existing.unwrap_or_else(new_id);
        transaction.execute(
            "INSERT INTO tags(id, name, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(name) DO NOTHING",
            params![id, name, now_utc()],
        )?;
        let actual_id: String = transaction.query_row(
            "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
            [name],
            |row| row.get(0),
        )?;
        tag_ids.push(actual_id);
    }
    let keep = tag_ids.iter().cloned().collect::<HashSet<_>>();
    let mut current_statement =
        transaction.prepare("SELECT tag_id FROM task_tags WHERE task_id = ?1")?;
    let current = current_statement
        .query_map([task_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(current_statement);
    for old_id in current {
        if !keep.contains(&old_id) {
            transaction.execute(
                "DELETE FROM task_tags WHERE task_id = ?1 AND tag_id = ?2",
                params![task_id, old_id],
            )?;
        }
    }
    for tag_id in &tag_ids {
        transaction.execute(
            "INSERT OR IGNORE INTO task_tags(task_id, tag_id, created_at)
             VALUES (?1, ?2, ?3)",
            params![task_id, tag_id, now_utc()],
        )?;
    }
    list_tags_for_connection(transaction, Some(task_id))
}

fn sync_checklist_in_transaction(
    transaction: &Transaction<'_>,
    task_id: &str,
    requested: &[ChecklistMutationInput],
) -> Result<Vec<ChecklistItem>> {
    query_task(transaction, task_id, false)?;
    let mut requested_ids = HashSet::new();
    for item in requested {
        required_text("checklist item", &item.body)?;
        if let Some(item_id) = item.id.as_deref() {
            let item_id = required_text("checklist item ID", item_id)?;
            if !requested_ids.insert(item_id.clone()) {
                return Err(invalid(format!(
                    "checklist item appears more than once: {item_id}"
                )));
            }
            let existing = query_checklist_item(transaction, &item_id)?;
            if existing.task_id != task_id {
                return Err(Error::Conflict(format!(
                    "checklist item {item_id} belongs to task {}, not {task_id}",
                    existing.task_id
                )));
            }
        }
    }

    let mut existing_statement = transaction.prepare(
        "SELECT id FROM checklist_items WHERE task_id = ?1 ORDER BY sort_order, created_at",
    )?;
    let existing_ids = existing_statement
        .query_map([task_id], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(existing_statement);
    for existing_id in existing_ids {
        if !requested_ids.contains(&existing_id) {
            transaction.execute(
                "DELETE FROM checklist_items WHERE id = ?1 AND task_id = ?2",
                params![existing_id, task_id],
            )?;
        }
    }

    for (position, item) in requested.iter().enumerate() {
        let position =
            i64::try_from(position).map_err(|_| invalid("too many checklist items to persist"))?;
        let body = required_text("checklist item", &item.body)?;
        if let Some(item_id) = item.id.as_deref() {
            let current = query_checklist_item(transaction, item_id)?;
            let completed_at = if item.is_done {
                current.completed_at.or_else(|| Some(now_utc()))
            } else {
                None
            };
            let changed = transaction.execute(
                "UPDATE checklist_items
                 SET body = ?3, is_done = ?4, sort_order = ?5,
                     completed_at = ?6, updated_at = ?7, version = version + 1
                 WHERE id = ?1 AND task_id = ?2",
                params![
                    item_id,
                    task_id,
                    body,
                    i64::from(item.is_done),
                    position,
                    completed_at,
                    now_utc(),
                ],
            )?;
            if changed != 1 {
                return Err(Error::Conflict(format!(
                    "checklist item {item_id} changed ownership concurrently"
                )));
            }
        } else {
            let id = new_id();
            let now = now_utc();
            transaction.execute(
                "INSERT INTO checklist_items(
                    id, task_id, body, is_done, sort_order,
                    created_at, updated_at, completed_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
                params![
                    id,
                    task_id,
                    body,
                    i64::from(item.is_done),
                    position,
                    now,
                    item.is_done.then(now_utc),
                ],
            )?;
        }
    }
    list_checklist_for_connection(transaction, task_id)
}

pub(crate) fn create_note_in_transaction(
    transaction: &Transaction<'_>,
    input: &CreateNoteInput,
) -> Result<Note> {
    let title = required_text("note title", &input.title)?;
    let id = new_id();
    let now = now_utc();
    transaction.execute(
        "INSERT INTO notes(
            id, note_type, title, body, note_date, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
        params![
            id,
            input.note_type.as_str(),
            title,
            input.body.trim(),
            input.note_date,
            now,
        ],
    )?;
    query_note(transaction, &id)
}

pub(crate) fn update_note_in_transaction(
    transaction: &Transaction<'_>,
    note_id: &str,
    patch: NotePatch,
) -> Result<Note> {
    if patch.clear_note_date && patch.note_date.is_some() {
        return Err(invalid("note date cannot be both set and cleared"));
    }
    let current = query_note(transaction, note_id)?;
    if current.deleted_at.is_some() {
        return Err(Error::Conflict(format!(
            "note {note_id} is in the trash; restore it before editing"
        )));
    }
    let title = patch
        .title
        .as_deref()
        .map(|value| required_text("note title", value))
        .transpose()?
        .unwrap_or(current.title);
    let note_date = if patch.clear_note_date {
        None
    } else {
        patch.note_date.or(current.note_date)
    };
    transaction.execute(
        "UPDATE notes
         SET note_type = ?2, title = ?3, body = ?4, note_date = ?5,
             updated_at = ?6, version = version + 1
         WHERE id = ?1",
        params![
            note_id,
            patch.note_type.unwrap_or(current.note_type).as_str(),
            title,
            patch.body.unwrap_or(current.body).trim(),
            note_date,
            now_utc(),
        ],
    )?;
    query_note(transaction, note_id)
}

fn create_note_link_in_transaction(
    transaction: &Transaction<'_>,
    note_id: &str,
    target_type: LinkTargetType,
    target_id: Option<&str>,
    target_value: Option<&str>,
) -> Result<EntityLink> {
    validate_link_target(target_type, target_id, target_value)?;
    let id = insert_entity_link(
        transaction,
        EntityType::Note,
        note_id,
        target_type,
        target_id,
        target_value,
        "documents",
    )?;
    query_link(transaction, &id)
}

fn validate_link_target(
    target_type: LinkTargetType,
    target_id: Option<&str>,
    target_value: Option<&str>,
) -> Result<()> {
    match target_type {
        LinkTargetType::Task
        | LinkTargetType::Session
        | LinkTargetType::Worklog
        | LinkTargetType::Note => {
            let id = target_id.ok_or_else(|| invalid("entity link requires targetId"))?;
            required_text("link target ID", id)?;
            if target_value.is_some() {
                return Err(invalid("entity link cannot contain targetValue"));
            }
        }
        LinkTargetType::File | LinkTargetType::Url => {
            if target_id.is_some() {
                return Err(invalid("file and URL links cannot contain targetId"));
            }
            let value = target_value.ok_or_else(|| invalid("link requires targetValue"))?;
            let value = required_text("link target value", value)?;
            if target_type == LinkTargetType::Url {
                url::Url::parse(&value)
                    .map_err(|error| invalid(format!("invalid URL target: {error}")))?;
            }
        }
    }
    Ok(())
}

pub(crate) fn create_task_in_transaction(
    transaction: &Transaction<'_>,
    input: &CreateTaskInput,
) -> Result<Task> {
    let title = required_text("task title", &input.title)?;
    if input.priority > 4 {
        return Err(invalid("task priority must be between 0 and 4"));
    }
    let id = new_id();
    let now = now_utc();
    let completed_at = (input.status == TaskStatus::Done).then(|| now.clone());
    transaction.execute(
        "INSERT INTO tasks(
            id, project_id, title, description, status, priority, due_date,
            completed_at, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
        params![
            id,
            input.project_id,
            title,
            input.description.trim(),
            input.status.as_str(),
            input.priority,
            input.due_date,
            completed_at,
            now,
        ],
    )?;
    query_task(transaction, &id, true)
}

pub(crate) fn query_project(connection: &Connection, id: &str) -> Result<Project> {
    connection
        .query_row(
            "SELECT id, name, description, color, sort_order, created_at, updated_at,
                    archived_at, deleted_at
             FROM projects WHERE id = ?1",
            [id],
            map_project,
        )
        .optional()?
        .ok_or_else(|| not_found("project", id))
}

fn map_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        color: row.get(3)?,
        sort_order: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        archived_at: row.get(7)?,
        deleted_at: row.get(8)?,
    })
}

pub(crate) fn query_task(connection: &Connection, id: &str, include_deleted: bool) -> Result<Task> {
    connection
        .query_row(
            "SELECT id, project_id, title, description, status, priority, due_date,
                    completed_at, created_at, updated_at, deleted_at, version
             FROM tasks WHERE id = ?1 AND (?2 = 1 OR deleted_at IS NULL)",
            params![id, i64::from(include_deleted)],
            map_task,
        )
        .optional()?
        .ok_or_else(|| not_found("task", id))
}

fn map_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let status: String = row.get(4)?;
    let priority: i64 = row.get(5)?;
    Ok(Task {
        id: row.get(0)?,
        project_id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: enum_from_sql(4, &status)?,
        priority: u8::try_from(priority).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(5, Type::Integer, Box::new(error))
        })?,
        due_date: row.get(6)?,
        completed_at: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
        deleted_at: row.get(10)?,
        version: integer_version(row, 11)?,
    })
}

fn map_task_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskEvent> {
    let before_text: Option<String> = row.get(3)?;
    let after_text: Option<String> = row.get(4)?;
    Ok(TaskEvent {
        id: row.get(0)?,
        task_id: row.get(1)?,
        event_type: row.get(2)?,
        before: parse_json_column(3, before_text)?,
        after: parse_json_column(4, after_text)?,
        actor: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub(crate) fn query_checklist_item(connection: &Connection, id: &str) -> Result<ChecklistItem> {
    connection
        .query_row(
            "SELECT id, task_id, body, is_done, sort_order, created_at, updated_at, completed_at, version
             FROM checklist_items WHERE id = ?1",
            [id],
            map_checklist_item,
        )
        .optional()?
        .ok_or_else(|| not_found("checklist item", id))
}

pub(crate) fn update_checklist_item_in_transaction(
    transaction: &Transaction<'_>,
    item_id: &str,
    body: Option<&str>,
    is_done: Option<bool>,
    sort_order: Option<i64>,
) -> Result<ChecklistItem> {
    let current = query_checklist_item(transaction, item_id)?;
    let next_body = body
        .map(|value| required_text("checklist item", value))
        .transpose()?
        .unwrap_or(current.body);
    let next_done = is_done.unwrap_or(current.is_done);
    let completed_at = if next_done {
        current.completed_at.or_else(|| Some(now_utc()))
    } else {
        None
    };
    transaction.execute(
        "UPDATE checklist_items
         SET body = ?2, is_done = ?3, sort_order = ?4,
             completed_at = ?5, updated_at = ?6, version = version + 1
         WHERE id = ?1",
        params![
            item_id,
            next_body,
            i64::from(next_done),
            sort_order.unwrap_or(current.sort_order),
            completed_at,
            now_utc(),
        ],
    )?;
    query_checklist_item(transaction, item_id)
}

fn list_checklist_for_connection(
    connection: &Connection,
    task_id: &str,
) -> Result<Vec<ChecklistItem>> {
    let mut statement = connection.prepare(
        "SELECT id, task_id, body, is_done, sort_order, created_at, updated_at, completed_at, version
         FROM checklist_items WHERE task_id = ?1 ORDER BY sort_order ASC, created_at ASC",
    )?;
    let rows = statement.query_map([task_id], map_checklist_item)?;
    collect_rows(rows)
}

fn map_checklist_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChecklistItem> {
    let is_done: i64 = row.get(3)?;
    Ok(ChecklistItem {
        id: row.get(0)?,
        task_id: row.get(1)?,
        body: row.get(2)?,
        is_done: is_done == 1,
        sort_order: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        completed_at: row.get(7)?,
        version: integer_version(row, 8)?,
    })
}

fn map_tag(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tag> {
    Ok(Tag {
        id: row.get(0)?,
        name: row.get(1)?,
        color: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn list_tags_for_connection(connection: &Connection, task_id: Option<&str>) -> Result<Vec<Tag>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT t.id, t.name, t.color, t.created_at
         FROM tags t
         LEFT JOIN task_tags tt ON tt.tag_id = t.id
         WHERE (?1 IS NULL OR tt.task_id = ?1)
         ORDER BY t.name COLLATE NOCASE ASC",
    )?;
    let rows = statement.query_map([task_id], map_tag)?;
    collect_rows(rows)
}

fn query_day_entry(connection: &Connection, id: &str) -> Result<TaskDayEntry> {
    connection
        .query_row(
            "SELECT id, task_id, entry_date, status, note, created_at, updated_at, finalized_at
             FROM task_day_entries WHERE id = ?1",
            [id],
            map_day_entry,
        )
        .optional()?
        .ok_or_else(|| not_found("task day entry", id))
}

fn map_day_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskDayEntry> {
    let status: String = row.get(3)?;
    Ok(TaskDayEntry {
        id: row.get(0)?,
        task_id: row.get(1)?,
        entry_date: row.get(2)?,
        status: enum_from_sql(3, &status)?,
        note: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        finalized_at: row.get(7)?,
    })
}

fn query_session(connection: &Connection, id: &str) -> Result<WorkSession> {
    connection
        .query_row(
            "SELECT id, project_id, goal, status, started_at, ended_at, result,
                    blockers, next_action, created_at, updated_at, deleted_at
             FROM work_sessions WHERE id = ?1",
            [id],
            map_session,
        )
        .optional()?
        .ok_or_else(|| not_found("work session", id))
}

fn map_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkSession> {
    let status: String = row.get(3)?;
    Ok(WorkSession {
        id: row.get(0)?,
        project_id: row.get(1)?,
        goal: row.get(2)?,
        status: enum_from_sql(3, &status)?,
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        result: row.get(6)?,
        blockers: row.get(7)?,
        next_action: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        deleted_at: row.get(11)?,
    })
}

fn list_session_task_ids(connection: &Connection, session_id: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT task_id FROM session_tasks WHERE session_id = ?1 ORDER BY created_at, task_id",
    )?;
    let rows = statement.query_map([session_id], |row| row.get(0))?;
    collect_rows(rows)
}

fn query_worklog(connection: &Connection, id: &str) -> Result<WorkLog> {
    connection
        .query_row(
            "SELECT id, session_id, project_id, log_date, title, body,
                    created_at, updated_at, deleted_at
             FROM worklogs WHERE id = ?1",
            [id],
            map_worklog,
        )
        .optional()?
        .ok_or_else(|| not_found("work log", id))
}

fn map_worklog(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkLog> {
    Ok(WorkLog {
        id: row.get(0)?,
        session_id: row.get(1)?,
        project_id: row.get(2)?,
        log_date: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        deleted_at: row.get(8)?,
    })
}

pub(crate) fn query_note(connection: &Connection, id: &str) -> Result<Note> {
    connection
        .query_row(
            "SELECT id, note_type, title, body, source_worklog_id, note_date,
                    created_at, updated_at, deleted_at, version
             FROM notes WHERE id = ?1",
            [id],
            map_note,
        )
        .optional()?
        .ok_or_else(|| not_found("note", id))
}

fn map_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let note_type: String = row.get(1)?;
    Ok(Note {
        id: row.get(0)?,
        note_type: enum_from_sql(1, &note_type)?,
        title: row.get(2)?,
        body: row.get(3)?,
        source_worklog_id: row.get(4)?,
        note_date: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        deleted_at: row.get(8)?,
        version: integer_version(row, 9)?,
    })
}

fn insert_entity_link(
    connection: &Connection,
    source_type: EntityType,
    source_id: &str,
    target_type: LinkTargetType,
    target_id: Option<&str>,
    target_value: Option<&str>,
    relation: &str,
) -> Result<String> {
    let id = new_id();
    let changed = connection.execute(
        "INSERT OR IGNORE INTO entity_links(
            id, source_type, source_id, target_type, target_id,
            target_value, relation, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            source_type.as_str(),
            source_id,
            target_type.as_str(),
            target_id,
            target_value,
            relation,
            now_utc(),
        ],
    )?;
    if changed == 1 {
        return Ok(id);
    }
    connection
        .query_row(
            "SELECT id FROM entity_links
             WHERE source_type = ?1 AND source_id = ?2 AND target_type = ?3
               AND ifnull(target_id, '') = ifnull(?4, '')
               AND ifnull(target_value, '') = ifnull(?5, '')
               AND relation = ?6",
            params![
                source_type.as_str(),
                source_id,
                target_type.as_str(),
                target_id,
                target_value,
                relation,
            ],
            |row| row.get(0),
        )
        .map_err(Into::into)
}

fn query_link(connection: &Connection, id: &str) -> Result<EntityLink> {
    connection
        .query_row(
            "SELECT id, source_type, source_id, target_type, target_id,
                    target_value, relation, created_at
             FROM entity_links WHERE id = ?1",
            [id],
            map_link,
        )
        .optional()?
        .ok_or_else(|| not_found("entity link", id))
}

fn map_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<EntityLink> {
    let source: String = row.get(1)?;
    let target: String = row.get(3)?;
    Ok(EntityLink {
        id: row.get(0)?,
        source_type: enum_from_sql(1, &source)?,
        source_id: row.get(2)?,
        target_type: enum_from_sql(3, &target)?,
        target_id: row.get(4)?,
        target_value: row.get(5)?,
        relation: row.get(6)?,
        created_at: row.get(7)?,
    })
}

fn query_attachment(connection: &Connection, id: &str) -> Result<Attachment> {
    connection
        .query_row(
            "SELECT id, owner_type, owner_id, relative_path, original_name,
                    media_type, byte_size, sha256, created_at, deleted_at
             FROM attachments WHERE id = ?1",
            [id],
            map_attachment,
        )
        .optional()?
        .ok_or_else(|| not_found("attachment", id))
}

fn map_attachment(row: &rusqlite::Row<'_>) -> rusqlite::Result<Attachment> {
    let owner_type: String = row.get(1)?;
    let byte_size: i64 = row.get(6)?;
    Ok(Attachment {
        id: row.get(0)?,
        owner_type: enum_from_sql(1, &owner_type)?,
        owner_id: row.get(2)?,
        relative_path: row.get(3)?,
        original_name: row.get(4)?,
        media_type: row.get(5)?,
        byte_size: u64::try_from(byte_size).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(6, Type::Integer, Box::new(error))
        })?,
        sha256: row.get(7)?,
        created_at: row.get(8)?,
        deleted_at: row.get(9)?,
    })
}

fn parse_json_column(index: usize, value: Option<String>) -> rusqlite::Result<Option<Value>> {
    value
        .map(|value| {
            serde_json::from_str(&value).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
            })
        })
        .transpose()
}

fn enum_from_sql<T>(index: usize, value: &str) -> rusqlite::Result<T>
where
    T: FromStr<Err = Error>,
{
    T::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
    })
}

fn enum_conversion_error(index: usize, value: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        Type::Text,
        Box::new(Error::Invariant(format!("unknown enum value: {value}"))),
    )
}

fn integer_version(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(index, Type::Integer, Box::new(error))
    })
}

fn collect_rows<T>(rows: impl Iterator<Item = rusqlite::Result<T>>) -> Result<Vec<T>> {
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn fts_query(query: &str) -> Result<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        return Err(invalid("search query cannot be empty"));
    }
    Ok(terms.join(" "))
}

fn backup_trigger(file_name: &str) -> &'static str {
    if file_name.contains("-pre-migration-") {
        "pre_migration"
    } else if file_name.contains("-pre-restore-") {
        "pre_restore"
    } else if file_name.contains("-startup-") {
        "startup"
    } else if file_name.contains("-shutdown-") {
        "shutdown"
    } else if file_name.contains("-daily-") {
        "daily"
    } else {
        "manual"
    }
}
