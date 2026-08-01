use std::{
    collections::{BTreeSet, HashMap},
    str::FromStr,
};

use chrono::{DateTime, Datelike, NaiveDate, SecondsFormat, Utc};
use chrono_tz::Asia::Seoul;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::State;
use tm_core::{
    CalendarEvent, ChangeRequest, ChangeRequestKind, ChecklistMutationInput,
    CreateCalendarEventInput, CreateChangeRequestInput as CoreCreateChangeRequestInput,
    CreateNoteAggregateInput, CreateNoteInput, CreateProjectInput, CreateTaskAggregateInput,
    CreateTaskInput, CreateWorkLogInput, EndSessionInput, EntityLink, EntityType,
    LatestStockScreen, LinkTargetType, ListStockScreenResultsInput, Note, NoteLinksInput, NoteType,
    SearchHit, SessionStatus, StartSessionInput, StockScreenBandFilter, StockScreenDirection,
    StockScreenHorizon, StockScreenResultPage, StockWatchlistItem, Task, TaskDayEntry,
    TaskDayStatus, TaskPatch, TaskStatus, TmCore, TrashEntityType, UpdateCalendarEventInput,
    UpdateChangeRequestInput as CoreUpdateChangeRequestInput, UpdateTaskAggregateInput,
    UpsertStockWatchlistItemInput, WorkSession,
};

use crate::AppState;

type CommandResult<T> = std::result::Result<T, String>;
const DESKTOP_ACTOR: &str = "desktop-user";

fn command_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn parse_date(value: &str) -> CommandResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(command_error)
}

fn today_seoul() -> NaiveDate {
    Utc::now().with_timezone(&Seoul).date_naive()
}

fn priority_number(value: &str) -> CommandResult<u8> {
    match value {
        "none" => Ok(0),
        "low" => Ok(1),
        "medium" => Ok(2),
        "high" => Ok(3),
        other => Err(format!("알 수 없는 우선순위입니다: {other}")),
    }
}

fn priority_name(value: u8) -> &'static str {
    match value {
        0 => "none",
        1 => "low",
        2 => "medium",
        _ => "high",
    }
}

fn task_status(value: Option<&str>) -> CommandResult<TaskStatus> {
    value
        .map(TaskStatus::from_str)
        .transpose()
        .map_err(command_error)
        .map(|status| status.unwrap_or(TaskStatus::Inbox))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTaskRequest {
    title: String,
    #[serde(default)]
    description: String,
    project_id: Option<String>,
    status: Option<String>,
    priority: Option<String>,
    due_date: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChecklistRequest {
    id: String,
    label: String,
    checked: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateTaskRequest {
    task_id: String,
    title: String,
    description: String,
    status: String,
    priority: String,
    due_date: Option<String>,
    project_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    checklist: Vec<ChecklistRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StartSessionRequest {
    project_id: Option<String>,
    #[serde(default)]
    task_ids: Vec<String>,
    goal: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FinishSessionRequest {
    session_id: String,
    result: String,
    #[serde(default)]
    blockers: String,
    #[serde(default)]
    next_action: String,
    #[serde(default)]
    create_next_task: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateWorkLogRequest {
    title: String,
    content: String,
    #[serde(default)]
    outcome: String,
    #[serde(default)]
    blockers: String,
    project_id: Option<String>,
    #[serde(default)]
    task_ids: Vec<String>,
    session_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoteLinksRequest {
    #[serde(default)]
    task_ids: Vec<String>,
    #[serde(default)]
    session_ids: Vec<String>,
    #[serde(default)]
    work_log_ids: Vec<String>,
    #[serde(default)]
    file_paths: Vec<String>,
    #[serde(default)]
    urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateNoteRequest {
    #[serde(rename = "type")]
    note_type: String,
    title: String,
    content: String,
    #[serde(default)]
    links: NoteLinksRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateChangeRequestRequest {
    kind: String,
    project_id: Option<String>,
    task_id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    desired_outcome: String,
    #[serde(default)]
    reproduction_steps: String,
    priority: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateChangeRequestRequest {
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    kind: String,
    project_id: Option<String>,
    task_id: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    desired_outcome: String,
    #[serde(default)]
    reproduction_steps: String,
    priority: String,
}

#[tauri::command]
pub(crate) fn get_app_snapshot(state: State<'_, AppState>) -> CommandResult<Value> {
    let _ = state
        .core
        .create_daily_backup_if_due()
        .map_err(command_error)?;
    build_snapshot(&state.core).map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_project(
    name: String,
    description: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    state
        .core
        .create_project(CreateProjectInput {
            name,
            description: description.unwrap_or_default(),
            color: None,
        })
        .map(|project| project.id)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn get_calendar_month(
    month: String,
    state: State<'_, AppState>,
) -> CommandResult<Value> {
    let month = parse_month(&month)?;
    let calendar = state
        .core
        .calendar_month(month.year(), month.month())
        .map_err(command_error)?;
    crate::expense_commands::calendar_month_value(&state, calendar)
}

#[tauri::command]
pub(crate) fn create_calendar_event(
    input: CreateCalendarEventInput,
    state: State<'_, AppState>,
) -> CommandResult<CalendarEvent> {
    state
        .core
        .create_calendar_event(input)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn update_calendar_event(
    event_id: String,
    input: UpdateCalendarEventInput,
    state: State<'_, AppState>,
) -> CommandResult<CalendarEvent> {
    state
        .core
        .update_calendar_event(&event_id, input)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn delete_calendar_event(
    event_id: String,
    expected_version: u64,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .delete_calendar_event(&event_id, expected_version)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn get_stock_watchlist(
    state: State<'_, AppState>,
) -> CommandResult<Vec<StockWatchlistItem>> {
    state.core.stock_watchlist().map_err(command_error)
}

#[tauri::command]
pub(crate) fn upsert_stock_watchlist_item(
    input: UpsertStockWatchlistItemInput,
    state: State<'_, AppState>,
) -> CommandResult<StockWatchlistItem> {
    state
        .core
        .upsert_stock_watchlist_item(input)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn delete_stock_watchlist_item(
    symbol: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .delete_stock_watchlist_item(&symbol)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn get_latest_stock_screen(
    state: State<'_, AppState>,
) -> CommandResult<LatestStockScreen> {
    state.core.get_latest_stock_screen().map_err(command_error)
}

#[tauri::command]
pub(crate) fn list_stock_screen_results(
    run_id: String,
    horizon: StockScreenHorizon,
    direction: StockScreenDirection,
    band: Option<StockScreenBandFilter>,
    cursor: Option<String>,
    limit: Option<u32>,
    state: State<'_, AppState>,
) -> CommandResult<StockScreenResultPage> {
    state
        .core
        .list_stock_screen_results(ListStockScreenResultsInput {
            run_id,
            horizon,
            direction,
            band,
            cursor,
            limit,
        })
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_task(
    input: CreateTaskRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let priority = priority_number(input.priority.as_deref().unwrap_or("none"))?;
    let due_date = input.due_date.as_deref().map(parse_date).transpose()?;
    let status = task_status(input.status.as_deref())?;
    state
        .core
        .create_task_aggregate(CreateTaskAggregateInput {
            task: CreateTaskInput {
                project_id: input.project_id,
                title: input.title,
                description: input.description,
                status,
                priority,
                due_date,
            },
            tags: input.tags,
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn update_task(
    input: UpdateTaskRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let status = TaskStatus::from_str(&input.status).map_err(command_error)?;
    let priority = priority_number(&input.priority)?;
    let due_date = input.due_date.as_deref().map(parse_date).transpose()?;
    state
        .core
        .update_task_aggregate(UpdateTaskAggregateInput {
            task_id: input.task_id,
            patch: TaskPatch {
                title: Some(input.title),
                description: Some(input.description),
                status: Some(status),
                priority: Some(priority),
                project_id: input.project_id.clone(),
                clear_project: input.project_id.is_none(),
                due_date,
                clear_due_date: input.due_date.is_none(),
            },
            tags: input.tags,
            checklist: input
                .checklist
                .into_iter()
                .map(|item| ChecklistMutationInput {
                    id: (!item.id.is_empty()).then_some(item.id),
                    body: item.label,
                    is_done: item.checked,
                })
                .collect(),
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn plan_task(
    task_id: String,
    date: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .plan_task(&task_id, parse_date(&date)?, None)
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn resolve_day_entry(
    entry_id: String,
    status: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let status = TaskDayStatus::from_str(&status).map_err(command_error)?;
    if status == TaskDayStatus::Deferred {
        let entry = state
            .core
            .list_day_entries(None, None)
            .map_err(command_error)?
            .into_iter()
            .find(|entry| entry.id == entry_id)
            .ok_or_else(|| format!("오늘 기록을 찾지 못했습니다: {entry_id}"))?;
        let today = today_seoul();
        if entry.entry_date < today {
            state
                .core
                .carry_over_day_entry(&entry_id, today)
                .map(|_| ())
                .map_err(command_error)
        } else {
            Err("이월은 오늘보다 과거인 계획에만 적용할 수 있습니다.".to_owned())
        }
    } else {
        state
            .core
            .finalize_day_entry(&entry_id, status, None)
            .map(|_| ())
            .map_err(command_error)
    }
}

#[tauri::command]
pub(crate) fn start_session(
    input: StartSessionRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .start_session(StartSessionInput {
            project_id: input.project_id,
            goal: input.goal,
            task_ids: input.task_ids,
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn finish_session(
    input: FinishSessionRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let followup_title = (!input.next_action.trim().is_empty()).then(|| input.next_action.clone());
    state
        .core
        .end_session(
            &input.session_id,
            EndSessionInput {
                result: input.result,
                blockers: input.blockers,
                next_action: input.next_action,
                create_followup_task: input.create_next_task,
                followup_title,
                followup_project_id: None,
            },
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_work_log(
    input: CreateWorkLogRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let mut body = input.content.trim().to_owned();
    if !input.outcome.trim().is_empty() {
        body.push_str("\n\n## 결과\n");
        body.push_str(input.outcome.trim());
    }
    if !input.blockers.trim().is_empty() {
        body.push_str("\n\n## 막힌 점\n");
        body.push_str(input.blockers.trim());
    }
    state
        .core
        .create_worklog(CreateWorkLogInput {
            session_id: input.session_id,
            project_id: input.project_id,
            log_date: Some(today_seoul()),
            title: input.title,
            body,
            task_ids: input.task_ids,
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_note(
    input: CreateNoteRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let note_type = NoteType::from_str(&input.note_type).map_err(command_error)?;
    state
        .core
        .create_note_aggregate(CreateNoteAggregateInput {
            note: CreateNoteInput {
                note_type,
                title: input.title,
                body: input.content,
                note_date: (note_type == NoteType::Daily).then(today_seoul),
            },
            links: NoteLinksInput {
                task_ids: input.links.task_ids,
                session_ids: input.links.session_ids,
                worklog_ids: input.links.work_log_ids,
                file_paths: input.links.file_paths,
                urls: input.links.urls,
            },
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_change_request(
    input: CreateChangeRequestRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .create_change_request(CoreCreateChangeRequestInput {
            kind: ChangeRequestKind::from_str(&input.kind).map_err(command_error)?,
            project_id: input.project_id,
            task_id: input.task_id,
            title: input.title,
            description: input.description,
            desired_outcome: input.desired_outcome,
            reproduction_steps: input.reproduction_steps,
            priority: priority_number(&input.priority)?,
            requested_by: DESKTOP_ACTOR.to_owned(),
        })
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn update_change_request(
    input: UpdateChangeRequestRequest,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .update_change_request(
            &input.change_request_id,
            CoreUpdateChangeRequestInput {
                expected_revision: input.expected_revision,
                expected_attempt_count: input.expected_attempt_count,
                kind: ChangeRequestKind::from_str(&input.kind).map_err(command_error)?,
                project_id: input.project_id,
                task_id: input.task_id,
                title: input.title,
                description: input.description,
                desired_outcome: input.desired_outcome,
                reproduction_steps: input.reproduction_steps,
                priority: priority_number(&input.priority)?,
                updated_by: DESKTOP_ACTOR.to_owned(),
            },
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn approve_change_request(
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .approve_change_request(
            &change_request_id,
            expected_revision,
            expected_attempt_count,
            DESKTOP_ACTOR,
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn return_change_request_to_draft(
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .return_change_request_to_draft(
            &change_request_id,
            expected_revision,
            expected_attempt_count,
            DESKTOP_ACTOR,
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn cancel_change_request(
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    reason: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .cancel_change_request(
            &change_request_id,
            expected_revision,
            expected_attempt_count,
            &reason,
            DESKTOP_ACTOR,
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn abandon_change_request(
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    reason: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .abandon_change_request(
            &change_request_id,
            expected_revision,
            expected_attempt_count,
            &reason,
            DESKTOP_ACTOR,
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn promote_work_log(
    work_log_id: String,
    note_type: String,
    title: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .promote_worklog(
            &work_log_id,
            NoteType::from_str(&note_type).map_err(command_error)?,
            Some(&title),
        )
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn search(query: String, state: State<'_, AppState>) -> CommandResult<Vec<Value>> {
    state
        .core
        .search(&query, 100)
        .map(|hits| hits.into_iter().map(search_value).collect())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn move_to_trash(
    item_id: String,
    entity_type: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    state
        .core
        .move_to_trash(parse_trash_type(&entity_type)?, &item_id)
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn restore_trash_item(item_id: String, state: State<'_, AppState>) -> CommandResult<()> {
    let item = state
        .core
        .list_trash()
        .map_err(command_error)?
        .into_iter()
        .find(|item| item.id == item_id)
        .ok_or_else(|| format!("휴지통 항목을 찾지 못했습니다: {item_id}"))?;
    state
        .core
        .restore_from_trash(item.entity_type, &item_id)
        .map_err(command_error)
}

fn parse_trash_type(value: &str) -> CommandResult<TrashEntityType> {
    match value {
        "task" => Ok(TrashEntityType::Task),
        "note" => Ok(TrashEntityType::Note),
        "project" => Ok(TrashEntityType::Project),
        "work_log" | "worklog" => Ok(TrashEntityType::Worklog),
        "session" => Ok(TrashEntityType::Session),
        other => Err(format!("알 수 없는 휴지통 유형입니다: {other}")),
    }
}

fn parse_month(value: &str) -> CommandResult<NaiveDate> {
    if value.len() != 7 {
        return Err(format!("월 형식은 YYYY-MM이어야 합니다: {value}"));
    }
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d").map_err(command_error)
}

#[tauri::command]
pub(crate) fn create_backup(state: State<'_, AppState>) -> CommandResult<()> {
    state
        .core
        .create_backup()
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn restore_backup(backup_id: String, state: State<'_, AppState>) -> CommandResult<()> {
    let backup = state
        .core
        .list_backups()
        .map_err(command_error)?
        .into_iter()
        .find(|backup| backup.file_name == backup_id)
        .ok_or_else(|| format!("백업 파일을 찾지 못했습니다: {backup_id}"))?;
    state
        .core
        .restore_backup(backup.path)
        .map(|_| ())
        .map_err(command_error)
}

#[tauri::command]
pub(crate) fn export_all(state: State<'_, AppState>) -> CommandResult<Value> {
    state
        .core
        .create_export()
        .map(|artifact| {
            json!({
                "jsonPath": artifact.json_path,
                "markdownPath": artifact.markdown_path,
                "exportedAt": artifact.created_at,
            })
        })
        .map_err(command_error)
}

fn build_snapshot(core: &TmCore) -> tm_core::Result<Value> {
    let projects = core.list_projects(false)?;
    let tasks = core.list_tasks(false)?;
    let sessions = core.list_sessions(false)?;
    let worklogs = core.list_worklogs(false)?;
    let notes = core.list_notes(false)?;
    let change_requests = core.list_change_requests()?;
    let links = core.list_links(None, None)?;
    let day_entries = core.list_day_entries(None, None)?;
    let trash = core.list_trash()?;
    let backups = core.list_backups()?;

    let project_names: HashMap<&str, &str> = projects
        .iter()
        .map(|project| (project.id.as_str(), project.name.as_str()))
        .collect();
    let task_by_id: HashMap<&str, &Task> =
        tasks.iter().map(|task| (task.id.as_str(), task)).collect();
    let session_by_id: HashMap<&str, &WorkSession> = sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect();
    let promoted_notes: HashMap<&str, &str> = notes
        .iter()
        .filter_map(|note| {
            note.source_worklog_id
                .as_deref()
                .map(|worklog_id| (worklog_id, note.id.as_str()))
        })
        .collect();

    let worklog_values: HashMap<String, Value> = worklogs
        .iter()
        .map(|worklog| {
            let task_ids = linked_ids(
                &links,
                EntityType::Worklog,
                &worklog.id,
                LinkTargetType::Task,
            );
            let session = worklog
                .session_id
                .as_deref()
                .and_then(|id| session_by_id.get(id).copied());
            (
                worklog.id.clone(),
                json!({
                    "id": worklog.id,
                    "taskIds": task_ids,
                    "sessionId": worklog.session_id,
                    "projectId": worklog.project_id,
                    "title": worklog.title,
                    "content": worklog.body,
                    "outcome": session.map(|item| item.result.as_str()).filter(|value| !value.is_empty()),
                    "blockers": session.map(|item| item.blockers.as_str()).filter(|value| !value.is_empty()),
                    "createdAt": worklog.created_at,
                    "promotedNoteId": promoted_notes.get(worklog.id.as_str()).copied(),
                }),
            )
        })
        .collect();

    let mut task_values = HashMap::new();
    for task in &tasks {
        let checklist = core.list_checklist_items(&task.id)?;
        let tags = core.list_tags(Some(&task.id))?;
        let events = core.list_task_events(&task.id)?;
        let related_logs: Vec<Value> = worklogs
            .iter()
            .filter(|log| {
                linked_ids(&links, EntityType::Worklog, &log.id, LinkTargetType::Task)
                    .iter()
                    .any(|task_id| task_id == &task.id)
            })
            .filter_map(|log| worklog_values.get(&log.id).cloned())
            .collect();
        let event_values: Vec<Value> = events
            .into_iter()
            .map(|event| {
                json!({
                    "id": event.id,
                    "taskId": event.task_id,
                    "eventType": event.event_type,
                    "summary": event_summary(&event.event_type),
                    "before": event.before,
                    "after": event.after,
                    "createdAt": event.created_at,
                })
            })
            .collect();
        let checklist_values: Vec<Value> = checklist
            .into_iter()
            .map(|item| json!({"id": item.id, "label": item.body, "checked": item.is_done}))
            .collect();
        task_values.insert(
            task.id.clone(),
            json!({
                "id": task.id,
                "title": task.title,
                "description": task.description,
                "status": task.status.as_str(),
                "priority": priority_name(task.priority),
                "dueDate": task.due_date.map(|date| date.to_string()),
                "projectId": task.project_id,
                "projectName": task.project_id.as_deref().and_then(|id| project_names.get(id).copied()),
                "tags": tags.into_iter().map(|tag| tag.name).collect::<Vec<_>>(),
                "checklist": checklist_values,
                "events": event_values,
                "workLogs": related_logs,
                "createdAt": task.created_at,
                "updatedAt": task.updated_at,
                "deletedAt": task.deleted_at,
            }),
        );
    }

    let session_values: Vec<Value> = sessions
        .iter()
        .map(|session| {
            let task_ids = core.session_task_ids(&session.id)?;
            let task_titles: Vec<&str> = task_ids
                .iter()
                .filter_map(|id| task_by_id.get(id.as_str()).map(|task| task.title.as_str()))
                .collect();
            Ok(json!({
                "id": session.id,
                "projectId": session.project_id,
                "projectName": session.project_id.as_deref().and_then(|id| project_names.get(id).copied()),
                "taskIds": task_ids,
                "taskTitles": task_titles,
                "goal": session.goal,
                "startedAt": session.started_at,
                "endedAt": session.ended_at,
                "result": optional_text(&session.result),
                "blockers": optional_text(&session.blockers),
                "nextAction": optional_text(&session.next_action),
            }))
        })
        .collect::<tm_core::Result<Vec<_>>>()?;

    let note_values: Vec<Value> = notes.iter().map(|note| note_value(note, &links)).collect();
    let change_request_values: Vec<Value> =
        change_requests.iter().map(change_request_value).collect();

    let entry_values: Vec<(NaiveDate, TaskDayStatus, Value)> = day_entries
        .iter()
        .filter_map(|entry| {
            task_values
                .get(&entry.task_id)
                .cloned()
                .map(|task| (entry.entry_date, entry.status, day_entry_value(entry, task)))
        })
        .collect();
    let today = today_seoul();
    let yesterday = today.pred_opt();
    let yesterday_incomplete: Vec<Value> = entry_values
        .iter()
        .filter(|(date, status, _)| Some(*date) == yesterday && *status == TaskDayStatus::Planned)
        .map(|(_, _, value)| value.clone())
        .collect();
    let planned: Vec<Value> = entry_values
        .iter()
        .filter(|(date, status, _)| *date == today && *status == TaskDayStatus::Planned)
        .map(|(_, _, value)| value.clone())
        .collect();
    let completed: Vec<Value> = entry_values
        .iter()
        .filter(|(date, status, _)| *date == today && *status == TaskDayStatus::Done)
        .map(|(_, _, value)| value.clone())
        .collect();
    let in_progress: Vec<Value> = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::InProgress)
        .filter_map(|task| task_values.get(&task.id).cloned())
        .collect();

    let mut history_dates = BTreeSet::new();
    history_dates.extend(day_entries.iter().map(|entry| entry.entry_date));
    history_dates.extend(worklogs.iter().map(|worklog| worklog.log_date));
    history_dates.extend(sessions.iter().filter_map(session_seoul_date));
    let history: Vec<Value> = history_dates
        .into_iter()
        .rev()
        .map(|date| {
            let entries: Vec<Value> = entry_values
                .iter()
                .filter(|(entry_date, _, _)| *entry_date == date)
                .map(|(_, _, value)| value.clone())
                .collect();
            let worklog_count = worklogs.iter().filter(|log| log.log_date == date).count();
            let session_minutes: i64 = sessions
                .iter()
                .filter(|session| session_seoul_date(session) == Some(date))
                .filter_map(session_minutes)
                .sum();
            json!({
                "date": date.to_string(),
                "entries": entries,
                "workLogCount": worklog_count,
                "sessionMinutes": session_minutes,
            })
        })
        .collect();

    let project_values: Vec<Value> = projects
        .iter()
        .map(|project| {
            let project_tasks: Vec<&Task> = tasks
                .iter()
                .filter(|task| task.project_id.as_deref() == Some(project.id.as_str()))
                .collect();
            json!({
                "id": project.id,
                "systemKey": project.system_key,
                "name": project.name,
                "description": project.description,
                "color": project.color.as_deref().unwrap_or("#7386ff"),
                "openTaskCount": project_tasks.iter().filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled)).count(),
                "completedTaskCount": project_tasks.iter().filter(|task| task.status == TaskStatus::Done).count(),
                "archived": project.archived_at.is_some(),
            })
        })
        .collect();
    let backup_values: Vec<Value> = backups
        .iter()
        .map(|backup| {
            json!({
                "id": backup.file_name,
                "fileName": backup.file_name,
                "createdAt": backup.created_at,
                "sizeBytes": backup.byte_size,
                "trigger": backup.trigger,
            })
        })
        .collect();
    let trash_values: Vec<Value> = trash
        .into_iter()
        .map(|item| {
            json!({
                "id": item.id,
                "entityType": match item.entity_type {
                    TrashEntityType::Worklog => "work_log",
                    other => other.as_str(),
                },
                "title": item.title,
                "deletedAt": item.deleted_at,
            })
        })
        .collect();
    let active_session = sessions
        .iter()
        .position(|session| session.status == SessionStatus::Running)
        .and_then(|index| session_values.get(index).cloned());

    Ok(json!({
        "generatedAt": Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        "today": today.to_string(),
        "projects": project_values,
        "tasks": tasks.iter().filter_map(|task| task_values.get(&task.id).cloned()).collect::<Vec<_>>(),
        "todayView": {
            "yesterdayIncomplete": yesterday_incomplete,
            "planned": planned,
            "inProgress": in_progress,
            "completed": completed,
        },
        "history": history,
        "activeSession": active_session,
        "recentSessions": session_values,
        "workLogs": worklogs.iter().filter_map(|log| worklog_values.get(&log.id).cloned()).collect::<Vec<_>>(),
        "notes": note_values,
        "changeRequests": change_request_values,
        "trash": trash_values,
        "backups": backup_values,
        "databasePath": core.home().database_path().to_string_lossy(),
        "lastBackupAt": backups.first().map(|backup| backup.created_at.as_str()),
    }))
}

fn change_request_value(request: &ChangeRequest) -> Value {
    json!({
        "id": request.id,
        "kind": request.kind.as_str(),
        "projectId": request.project_id,
        "taskId": request.task_id,
        "title": request.title,
        "description": request.description,
        "desiredOutcome": request.desired_outcome,
        "reproductionSteps": request.reproduction_steps,
        "priority": priority_name(request.priority),
        "status": request.status.as_str(),
        "revision": request.revision,
        "approvedRevision": request.approved_revision,
        "attemptCount": request.attempt_count,
        "requestedBy": request.requested_by,
        "updatedBy": request.updated_by,
        "createdAt": request.created_at,
        "updatedAt": request.updated_at,
        "approvedAt": request.approved_at,
        "approvedBy": request.approved_by,
        "claimedAt": request.claimed_at,
        "claimedBy": request.claimed_by,
        "completedAt": request.completed_at,
        "completedBy": request.completed_by,
        "failedAt": request.failed_at,
        "failedBy": request.failed_by,
        "cancelledAt": request.cancelled_at,
        "cancelledBy": request.cancelled_by,
        "resultSummary": request.result_summary,
        "patchRef": request.patch_ref,
        "failureReason": request.failure_reason,
        "cancellationReason": request.cancellation_reason,
    })
}

fn linked_ids(
    links: &[EntityLink],
    source_type: EntityType,
    source_id: &str,
    target_type: LinkTargetType,
) -> Vec<String> {
    links
        .iter()
        .filter(|link| {
            link.source_type == source_type
                && link.source_id == source_id
                && link.target_type == target_type
        })
        .filter_map(|link| link.target_id.clone())
        .collect()
}

fn note_value(note: &Note, links: &[EntityLink]) -> Value {
    let note_links: Vec<&EntityLink> = links
        .iter()
        .filter(|link| link.source_type == EntityType::Note && link.source_id == note.id)
        .collect();
    json!({
        "id": note.id,
        "type": note.note_type.as_str(),
        "title": note.title,
        "content": note.body,
        "links": {
            "taskIds": link_target_ids(&note_links, LinkTargetType::Task),
            "sessionIds": link_target_ids(&note_links, LinkTargetType::Session),
            "workLogIds": link_target_ids(&note_links, LinkTargetType::Worklog),
            "filePaths": link_target_values(&note_links, LinkTargetType::File),
            "urls": link_target_values(&note_links, LinkTargetType::Url),
        },
        "createdAt": note.created_at,
        "updatedAt": note.updated_at,
        "deletedAt": note.deleted_at,
    })
}

fn link_target_ids(links: &[&EntityLink], target_type: LinkTargetType) -> Vec<String> {
    links
        .iter()
        .filter(|link| link.target_type == target_type)
        .filter_map(|link| link.target_id.clone())
        .collect()
}

fn link_target_values(links: &[&EntityLink], target_type: LinkTargetType) -> Vec<String> {
    links
        .iter()
        .filter(|link| link.target_type == target_type)
        .filter_map(|link| link.target_value.clone())
        .collect()
}

fn day_entry_value(entry: &TaskDayEntry, task: Value) -> Value {
    json!({
        "id": entry.id,
        "taskId": entry.task_id,
        "date": entry.entry_date.to_string(),
        "status": entry.status.as_str(),
        "task": task,
        "resolvedAt": entry.finalized_at,
    })
}

fn search_value(hit: SearchHit) -> Value {
    let entity_type = match hit.entity_type {
        EntityType::Worklog => "work_log",
        other => other.as_str(),
    };
    json!({
        "id": hit.entity_id,
        "entityType": entity_type,
        "title": hit.title,
        "excerpt": hit.excerpt,
        "meta": entity_type,
        "taskId": (hit.entity_type == EntityType::Task).then_some(hit.entity_id),
    })
}

fn event_summary(event_type: &str) -> &'static str {
    match event_type {
        "task.created" => "Task를 만들었습니다.",
        "task.changed" => "Task 속성을 변경했습니다.",
        "checklist.changed" => "체크리스트를 변경했습니다.",
        "tags.changed" => "태그를 변경했습니다.",
        _ => "Task 활동을 기록했습니다.",
    }
}

fn optional_text(value: &str) -> Option<&str> {
    (!value.trim().is_empty()).then_some(value)
}

fn session_seoul_date(session: &WorkSession) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(&session.started_at)
        .ok()
        .map(|value| value.with_timezone(&Seoul).date_naive())
}

fn session_minutes(session: &WorkSession) -> Option<i64> {
    let started = DateTime::parse_from_rfc3339(&session.started_at).ok()?;
    let ended = DateTime::parse_from_rfc3339(session.ended_at.as_deref()?).ok()?;
    Some((ended - started).num_minutes().max(0))
}

#[cfg(test)]
mod tests {
    use tempfile::Builder;
    use tm_core::{
        ChangeRequestKind, CreateChangeRequestInput, CreateProjectInput, CreateTaskInput,
        DEFAULT_TM_HOME, TaskStatus, TmHome,
    };

    use super::{build_snapshot, priority_name, priority_number, task_status};

    #[test]
    fn snapshot_adapts_core_models_for_the_react_contract() -> tm_core::Result<()> {
        let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
            .join("dist")
            .join("test-runs");
        std::fs::create_dir_all(&test_runs)?;
        let temporary = Builder::new().prefix("tm-tauri-").tempdir_in(test_runs)?;
        let core = tm_core::TmCore::open(TmHome::new(temporary.path()))?;
        let project = core.create_project(CreateProjectInput {
            name: "TM 구현".to_owned(),
            description: "로컬 우선 앱".to_owned(),
            color: None,
        })?;
        let task = core.create_task(CreateTaskInput {
            project_id: Some(project.id),
            title: "IPC snapshot 검증".to_owned(),
            description: "React는 SQL을 실행하지 않는다".to_owned(),
            status: TaskStatus::InProgress,
            priority: 3,
            due_date: None,
        })?;
        core.plan_task(
            &task.id,
            chrono::Utc::now()
                .with_timezone(&chrono_tz::Asia::Seoul)
                .date_naive(),
            None,
        )?;
        let change_request = core.create_change_request(CreateChangeRequestInput {
            kind: ChangeRequestKind::Ui,
            project_id: None,
            task_id: Some(task.id.clone()),
            title: "오늘 화면 구분 개선".to_owned(),
            description: "구역 경계가 흐립니다.".to_owned(),
            desired_outcome: "구역을 빠르게 구분합니다.".to_owned(),
            reproduction_steps: String::new(),
            priority: 2,
            requested_by: "desktop-user".to_owned(),
        })?;

        let snapshot = build_snapshot(&core)?;

        assert_eq!(snapshot["projects"][0]["name"], "TM 구현");
        assert!(
            snapshot["projects"]
                .as_array()
                .is_some_and(|projects| projects.iter().any(|project| {
                    project["systemKey"] == "uncategorized" && project["name"] == "기타"
                }))
        );
        assert_eq!(snapshot["tasks"][0]["priority"], "high");
        assert_eq!(snapshot["todayView"]["inProgress"][0]["id"], task.id);
        assert_eq!(snapshot["todayView"]["planned"][0]["taskId"], task.id);
        assert_eq!(snapshot["changeRequests"][0]["id"], change_request.id);
        assert_eq!(
            snapshot["changeRequests"][0]["desiredOutcome"],
            "구역을 빠르게 구분합니다."
        );
        assert_eq!(snapshot["changeRequests"][0]["priority"], "medium");
        assert_eq!(snapshot["changeRequests"][0]["status"], "draft");
        assert_eq!(snapshot["changeRequests"][0]["revision"], 1);
        Ok(())
    }

    #[test]
    fn priority_mapping_is_stable() {
        assert_eq!(priority_number("none"), Ok(0));
        assert_eq!(priority_number("high"), Ok(3));
        assert_eq!(priority_name(4), "high");
        assert!(priority_number("urgent").is_err());
    }

    #[test]
    fn create_task_status_defaults_to_inbox_and_accepts_todo() {
        assert_eq!(task_status(None), Ok(TaskStatus::Inbox));
        assert_eq!(task_status(Some("inbox")), Ok(TaskStatus::Inbox));
        assert_eq!(task_status(Some("todo")), Ok(TaskStatus::Todo));
    }

    #[test]
    fn create_task_status_rejects_unknown_values() {
        let error = task_status(Some("ready")).expect_err("unknown status must fail");
        assert!(error.contains("ready"));
    }
}
