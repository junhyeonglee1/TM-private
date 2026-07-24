use std::{
    collections::{BTreeSet, HashMap},
    fs,
    str::FromStr,
};

use chrono::{DateTime, Datelike, NaiveDate, SecondsFormat, Utc};
use chrono_tz::Asia::Seoul;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    ChangeRequest, ChangeRequestKind, ChecklistMutationInput, CreateCalendarEventInput,
    CreateChangeRequestInput, CreateNoteAggregateInput, CreateNoteInput, CreateProjectInput,
    CreateTaskAggregateInput, CreateTaskInput, CreateWorkLogInput, EndSessionInput, EntityLink,
    EntityType, Error, LinkTargetType, Note, NoteLinksInput, NoteType, Result, SearchHit,
    SessionStatus, StartSessionInput, Task, TaskDayEntry, TaskDayStatus, TaskPatch, TaskStatus,
    TmCore, TrashEntityType, UpdateCalendarEventInput, UpdateChangeRequestInput,
    UpdateTaskAggregateInput, UpsertStockWatchlistItemInput, WorkSession,
};

const DESKTOP_ACTOR: &str = "desktop-user";

/// Explicit allowlist for the first-party desktop command bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCommand {
    GetAppSnapshot,
    GetCalendarMonth,
    CreateCalendarEvent,
    UpdateCalendarEvent,
    DeleteCalendarEvent,
    GetStockWatchlist,
    UpsertStockWatchlistItem,
    DeleteStockWatchlistItem,
    CreateProject,
    CreateTask,
    UpdateTask,
    PlanTask,
    ResolveDayEntry,
    StartSession,
    FinishSession,
    CreateWorkLog,
    CreateNote,
    CreateChangeRequest,
    UpdateChangeRequest,
    ApproveChangeRequest,
    ReturnChangeRequestToDraft,
    CancelChangeRequest,
    AbandonChangeRequest,
    ListChangeRequests,
    ClaimNextChangeRequest,
    CompleteChangeRequest,
    FailChangeRequest,
    PromoteWorkLog,
    Search,
    MoveToTrash,
    RestoreTrashItem,
    CreateBackup,
    RestoreBackup,
    ExportAll,
}

impl DesktopCommand {
    pub const fn is_read_only(self) -> bool {
        matches!(
            self,
            Self::GetAppSnapshot
                | Self::GetCalendarMonth
                | Self::GetStockWatchlist
                | Self::Search
                | Self::ListChangeRequests
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GetAppSnapshot => "get_app_snapshot",
            Self::GetCalendarMonth => "get_calendar_month",
            Self::CreateCalendarEvent => "create_calendar_event",
            Self::UpdateCalendarEvent => "update_calendar_event",
            Self::DeleteCalendarEvent => "delete_calendar_event",
            Self::GetStockWatchlist => "get_stock_watchlist",
            Self::UpsertStockWatchlistItem => "upsert_stock_watchlist_item",
            Self::DeleteStockWatchlistItem => "delete_stock_watchlist_item",
            Self::CreateProject => "create_project",
            Self::CreateTask => "create_task",
            Self::UpdateTask => "update_task",
            Self::PlanTask => "plan_task",
            Self::ResolveDayEntry => "resolve_day_entry",
            Self::StartSession => "start_session",
            Self::FinishSession => "finish_session",
            Self::CreateWorkLog => "create_work_log",
            Self::CreateNote => "create_note",
            Self::CreateChangeRequest => "create_change_request",
            Self::UpdateChangeRequest => "update_change_request",
            Self::ApproveChangeRequest => "approve_change_request",
            Self::ReturnChangeRequestToDraft => "return_change_request_to_draft",
            Self::CancelChangeRequest => "cancel_change_request",
            Self::AbandonChangeRequest => "abandon_change_request",
            Self::ListChangeRequests => "list_change_requests",
            Self::ClaimNextChangeRequest => "claim_next_change_request",
            Self::CompleteChangeRequest => "complete_change_request",
            Self::FailChangeRequest => "fail_change_request",
            Self::PromoteWorkLog => "promote_work_log",
            Self::Search => "search",
            Self::MoveToTrash => "move_to_trash",
            Self::RestoreTrashItem => "restore_trash_item",
            Self::CreateBackup => "create_backup",
            Self::RestoreBackup => "restore_backup",
            Self::ExportAll => "export_all",
        }
    }
}

impl FromStr for DesktopCommand {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        serde_json::from_value(Value::String(value.to_owned()))
            .map_err(|_| Error::InvalidInput(format!("unsupported desktop command: {value}")))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateTaskRequest {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChecklistRequest {
    id: String,
    label: String,
    checked: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTaskRequest {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartSessionRequest {
    project_id: Option<String>,
    #[serde(default)]
    task_ids: Vec<String>,
    goal: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FinishSessionRequest {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWorkLogRequest {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NoteLinksRequest {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateNoteRequest {
    #[serde(rename = "type")]
    note_type: String,
    title: String,
    content: String,
    #[serde(default)]
    links: NoteLinksRequest,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ChangeRequestFields {
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateChangeRequestRequest {
    change_request_id: String,
    expected_revision: u32,
    expected_attempt_count: u32,
    #[serde(flatten)]
    fields: ChangeRequestFields,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputArg<T> {
    input: T,
}

fn parse_args<T: for<'de> Deserialize<'de>>(args: Value) -> Result<T> {
    serde_json::from_value(args).map_err(|error| {
        Error::InvalidInput(format!("desktop command arguments are invalid: {error}"))
    })
}

fn parse_date(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| Error::InvalidInput(format!("invalid date: {value}")))
}

fn today_seoul() -> NaiveDate {
    Utc::now().with_timezone(&Seoul).date_naive()
}

fn priority_number(value: &str) -> Result<u8> {
    match value {
        "none" => Ok(0),
        "low" => Ok(1),
        "medium" => Ok(2),
        "high" => Ok(3),
        other => Err(Error::InvalidInput(format!("invalid priority: {other}"))),
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

fn task_status(value: Option<&str>) -> Result<TaskStatus> {
    value
        .map(TaskStatus::from_str)
        .transpose()
        .map_err(|error| Error::InvalidInput(error.to_string()))
        .map(|status| status.unwrap_or(TaskStatus::Inbox))
}

pub fn execute_desktop_command(
    core: &TmCore,
    command: DesktopCommand,
    args: Value,
) -> Result<Value> {
    match command {
        DesktopCommand::GetAppSnapshot => build_snapshot(core),
        DesktopCommand::GetCalendarMonth => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                month: String,
            }
            let args: Args = parse_args(args)?;
            let month = parse_month(&args.month)?;
            serde_json::to_value(core.calendar_month(month.year(), month.month())?)
                .map_err(Into::into)
        }
        DesktopCommand::CreateCalendarEvent => {
            let args: InputArg<CreateCalendarEventInput> = parse_args(args)?;
            serde_json::to_value(core.create_calendar_event(args.input)?).map_err(Into::into)
        }
        DesktopCommand::UpdateCalendarEvent => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                event_id: String,
                input: UpdateCalendarEventInput,
            }
            let args: Args = parse_args(args)?;
            serde_json::to_value(core.update_calendar_event(&args.event_id, args.input)?)
                .map_err(Into::into)
        }
        DesktopCommand::DeleteCalendarEvent => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                event_id: String,
                expected_version: u64,
            }
            let args: Args = parse_args(args)?;
            core.delete_calendar_event(&args.event_id, args.expected_version)?;
            Ok(Value::Null)
        }
        DesktopCommand::GetStockWatchlist => {
            serde_json::to_value(core.stock_watchlist()?).map_err(Into::into)
        }
        DesktopCommand::UpsertStockWatchlistItem => {
            let args: InputArg<UpsertStockWatchlistItemInput> = parse_args(args)?;
            serde_json::to_value(core.upsert_stock_watchlist_item(args.input)?).map_err(Into::into)
        }
        DesktopCommand::DeleteStockWatchlistItem => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                symbol: String,
            }
            let args: Args = parse_args(args)?;
            core.delete_stock_watchlist_item(&args.symbol)?;
            Ok(Value::Null)
        }
        DesktopCommand::CreateProject => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                name: String,
                #[serde(default)]
                description: String,
            }
            let args: Args = parse_args(args)?;
            let project = core.create_project(CreateProjectInput {
                name: args.name,
                description: args.description,
                color: None,
            })?;
            Ok(Value::String(project.id))
        }
        DesktopCommand::CreateTask => {
            let args: InputArg<CreateTaskRequest> = parse_args(args)?;
            let input = args.input;
            core.create_task_aggregate(CreateTaskAggregateInput {
                task: CreateTaskInput {
                    project_id: input.project_id,
                    title: input.title,
                    description: input.description,
                    status: task_status(input.status.as_deref())?,
                    priority: priority_number(input.priority.as_deref().unwrap_or("none"))?,
                    due_date: input.due_date.as_deref().map(parse_date).transpose()?,
                },
                tags: input.tags,
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::UpdateTask => {
            let args: InputArg<UpdateTaskRequest> = parse_args(args)?;
            let input = args.input;
            let due_date = input.due_date.as_deref().map(parse_date).transpose()?;
            core.update_task_aggregate(UpdateTaskAggregateInput {
                task_id: input.task_id,
                patch: TaskPatch {
                    title: Some(input.title),
                    description: Some(input.description),
                    status: Some(
                        TaskStatus::from_str(&input.status)
                            .map_err(|error| Error::InvalidInput(error.to_string()))?,
                    ),
                    priority: Some(priority_number(&input.priority)?),
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
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::PlanTask => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                task_id: String,
                date: String,
            }
            let args: Args = parse_args(args)?;
            core.plan_task(&args.task_id, parse_date(&args.date)?, None)?;
            Ok(Value::Null)
        }
        DesktopCommand::ResolveDayEntry => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                entry_id: String,
                status: String,
            }
            let args: Args = parse_args(args)?;
            let status = TaskDayStatus::from_str(&args.status)
                .map_err(|error| Error::InvalidInput(error.to_string()))?;
            if status == TaskDayStatus::Deferred {
                let entry = core
                    .list_day_entries(None, None)?
                    .into_iter()
                    .find(|entry| entry.id == args.entry_id)
                    .ok_or_else(|| Error::NotFound {
                        entity: "day entry",
                        id: args.entry_id.clone(),
                    })?;
                let today = today_seoul();
                if entry.entry_date >= today {
                    return Err(Error::InvalidInput(
                        "only a past planned entry can be deferred".to_owned(),
                    ));
                }
                core.carry_over_day_entry(&args.entry_id, today)?;
            } else {
                core.finalize_day_entry(&args.entry_id, status, None)?;
            }
            Ok(Value::Null)
        }
        DesktopCommand::StartSession => {
            let args: InputArg<StartSessionRequest> = parse_args(args)?;
            let input = args.input;
            core.start_session(StartSessionInput {
                project_id: input.project_id,
                goal: input.goal,
                task_ids: input.task_ids,
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::FinishSession => {
            let args: InputArg<FinishSessionRequest> = parse_args(args)?;
            let input = args.input;
            let followup_title =
                (!input.next_action.trim().is_empty()).then(|| input.next_action.clone());
            core.end_session(
                &input.session_id,
                EndSessionInput {
                    result: input.result,
                    blockers: input.blockers,
                    next_action: input.next_action,
                    create_followup_task: input.create_next_task,
                    followup_title,
                    followup_project_id: None,
                },
            )?;
            Ok(Value::Null)
        }
        DesktopCommand::CreateWorkLog => {
            let args: InputArg<CreateWorkLogRequest> = parse_args(args)?;
            let input = args.input;
            let mut body = input.content.trim().to_owned();
            if !input.outcome.trim().is_empty() {
                body.push_str("\n\n## Result\n");
                body.push_str(input.outcome.trim());
            }
            if !input.blockers.trim().is_empty() {
                body.push_str("\n\n## Blockers\n");
                body.push_str(input.blockers.trim());
            }
            core.create_worklog(CreateWorkLogInput {
                session_id: input.session_id,
                project_id: input.project_id,
                log_date: Some(today_seoul()),
                title: input.title,
                body,
                task_ids: input.task_ids,
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::CreateNote => {
            let args: InputArg<CreateNoteRequest> = parse_args(args)?;
            let input = args.input;
            let note_type = NoteType::from_str(&input.note_type)
                .map_err(|error| Error::InvalidInput(error.to_string()))?;
            core.create_note_aggregate(CreateNoteAggregateInput {
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
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::CreateChangeRequest => {
            let args: InputArg<ChangeRequestFields> = parse_args(args)?;
            let input = args.input;
            core.create_change_request(CreateChangeRequestInput {
                kind: ChangeRequestKind::from_str(&input.kind)
                    .map_err(|error| Error::InvalidInput(error.to_string()))?,
                project_id: input.project_id,
                task_id: input.task_id,
                title: input.title,
                description: input.description,
                desired_outcome: input.desired_outcome,
                reproduction_steps: input.reproduction_steps,
                priority: priority_number(&input.priority)?,
                requested_by: DESKTOP_ACTOR.to_owned(),
            })?;
            Ok(Value::Null)
        }
        DesktopCommand::UpdateChangeRequest => {
            let args: InputArg<UpdateChangeRequestRequest> = parse_args(args)?;
            let input = args.input;
            let fields = input.fields;
            core.update_change_request(
                &input.change_request_id,
                UpdateChangeRequestInput {
                    expected_revision: input.expected_revision,
                    expected_attempt_count: input.expected_attempt_count,
                    kind: ChangeRequestKind::from_str(&fields.kind)
                        .map_err(|error| Error::InvalidInput(error.to_string()))?,
                    project_id: fields.project_id,
                    task_id: fields.task_id,
                    title: fields.title,
                    description: fields.description,
                    desired_outcome: fields.desired_outcome,
                    reproduction_steps: fields.reproduction_steps,
                    priority: priority_number(&fields.priority)?,
                    updated_by: DESKTOP_ACTOR.to_owned(),
                },
            )?;
            Ok(Value::Null)
        }
        DesktopCommand::ApproveChangeRequest
        | DesktopCommand::ReturnChangeRequestToDraft
        | DesktopCommand::CancelChangeRequest
        | DesktopCommand::AbandonChangeRequest => {
            execute_change_request_transition(core, command, args)
        }
        DesktopCommand::ListChangeRequests => {
            serde_json::to_value(core.list_change_requests()?).map_err(Into::into)
        }
        DesktopCommand::ClaimNextChangeRequest => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                worker_id: String,
            }
            let args: Args = parse_args(args)?;
            serde_json::to_value(core.claim_next_change_request(&args.worker_id)?)
                .map_err(Into::into)
        }
        DesktopCommand::CompleteChangeRequest => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                request_id: String,
                claim_key: String,
                result_summary: String,
                patch_ref: Option<String>,
                worker_id: String,
            }
            let args: Args = parse_args(args)?;
            serde_json::to_value(core.complete_change_request(
                &args.request_id,
                &args.claim_key,
                &args.result_summary,
                args.patch_ref.as_deref(),
                &args.worker_id,
            )?)
            .map_err(Into::into)
        }
        DesktopCommand::FailChangeRequest => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                request_id: String,
                claim_key: String,
                failure_reason: String,
                worker_id: String,
            }
            let args: Args = parse_args(args)?;
            serde_json::to_value(core.fail_change_request(
                &args.request_id,
                &args.claim_key,
                &args.failure_reason,
                &args.worker_id,
            )?)
            .map_err(Into::into)
        }
        DesktopCommand::PromoteWorkLog => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                work_log_id: String,
                note_type: String,
                title: String,
            }
            let args: Args = parse_args(args)?;
            core.promote_worklog(
                &args.work_log_id,
                NoteType::from_str(&args.note_type)
                    .map_err(|error| Error::InvalidInput(error.to_string()))?,
                Some(&args.title),
            )?;
            Ok(Value::Null)
        }
        DesktopCommand::Search => {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Args {
                query: String,
            }
            let args: Args = parse_args(args)?;
            Ok(Value::Array(
                core.search(&args.query, 100)?
                    .into_iter()
                    .map(search_value)
                    .collect(),
            ))
        }
        DesktopCommand::MoveToTrash => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                item_id: String,
                entity_type: String,
            }
            let args: Args = parse_args(args)?;
            core.move_to_trash(parse_trash_type(&args.entity_type)?, &args.item_id)?;
            Ok(Value::Null)
        }
        DesktopCommand::RestoreTrashItem => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                item_id: String,
            }
            let args: Args = parse_args(args)?;
            let item = core
                .list_trash()?
                .into_iter()
                .find(|item| item.id == args.item_id)
                .ok_or_else(|| Error::NotFound {
                    entity: "trash item",
                    id: args.item_id.clone(),
                })?;
            core.restore_from_trash(item.entity_type, &args.item_id)?;
            Ok(Value::Null)
        }
        DesktopCommand::CreateBackup => {
            core.create_backup()?;
            Ok(Value::Null)
        }
        DesktopCommand::RestoreBackup => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct Args {
                backup_id: String,
            }
            let args: Args = parse_args(args)?;
            let backup = core
                .list_backups()?
                .into_iter()
                .find(|backup| backup.file_name == args.backup_id)
                .ok_or_else(|| Error::NotFound {
                    entity: "backup",
                    id: args.backup_id.clone(),
                })?;
            core.restore_backup(backup.path)?;
            Ok(Value::Null)
        }
        DesktopCommand::ExportAll => {
            let artifact = core.create_export()?;
            let json_content = fs::read_to_string(&artifact.json_path)?;
            let markdown_content = fs::read_to_string(&artifact.markdown_path)?;
            let json_file_name = std::path::Path::new(&artifact.json_path)
                .file_name()
                .and_then(|value| value.to_str());
            let markdown_file_name = std::path::Path::new(&artifact.markdown_path)
                .file_name()
                .and_then(|value| value.to_str());
            Ok(json!({
                "jsonFileName": json_file_name,
                "jsonContent": json_content,
                "markdownFileName": markdown_file_name,
                "markdownContent": markdown_content,
                "exportedAt": artifact.created_at,
            }))
        }
    }
}

fn execute_change_request_transition(
    core: &TmCore,
    command: DesktopCommand,
    args: Value,
) -> Result<Value> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    struct Args {
        change_request_id: String,
        expected_revision: u32,
        expected_attempt_count: u32,
        #[serde(default)]
        reason: String,
    }
    let args: Args = parse_args(args)?;
    match command {
        DesktopCommand::ApproveChangeRequest => core.approve_change_request(
            &args.change_request_id,
            args.expected_revision,
            args.expected_attempt_count,
            DESKTOP_ACTOR,
        )?,
        DesktopCommand::ReturnChangeRequestToDraft => core.return_change_request_to_draft(
            &args.change_request_id,
            args.expected_revision,
            args.expected_attempt_count,
            DESKTOP_ACTOR,
        )?,
        DesktopCommand::CancelChangeRequest => core.cancel_change_request(
            &args.change_request_id,
            args.expected_revision,
            args.expected_attempt_count,
            &args.reason,
            DESKTOP_ACTOR,
        )?,
        DesktopCommand::AbandonChangeRequest => core.abandon_change_request(
            &args.change_request_id,
            args.expected_revision,
            args.expected_attempt_count,
            &args.reason,
            DESKTOP_ACTOR,
        )?,
        _ => {
            return Err(Error::Invariant(
                "non-transition command reached transition handler".to_owned(),
            ));
        }
    };
    Ok(Value::Null)
}

fn parse_month(value: &str) -> Result<NaiveDate> {
    if value.len() != 7 {
        return Err(Error::InvalidInput(format!(
            "calendar month must use YYYY-MM: {value}"
        )));
    }
    NaiveDate::parse_from_str(&format!("{value}-01"), "%Y-%m-%d")
        .map_err(|_| Error::InvalidInput(format!("invalid calendar month: {value}")))
}

fn parse_trash_type(value: &str) -> Result<TrashEntityType> {
    match value {
        "task" => Ok(TrashEntityType::Task),
        "note" => Ok(TrashEntityType::Note),
        "project" => Ok(TrashEntityType::Project),
        "work_log" | "worklog" => Ok(TrashEntityType::Worklog),
        "session" => Ok(TrashEntityType::Session),
        other => Err(Error::InvalidInput(format!(
            "invalid trash entity type: {other}"
        ))),
    }
}

fn build_snapshot(core: &TmCore) -> Result<Value> {
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
        .collect::<Result<Vec<_>>>()?;

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
        "databasePath": "TM Cloud",
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
        "task.created" => "Task created.",
        "task.changed" => "Task updated.",
        "checklist.changed" => "Checklist updated.",
        "tags.changed" => "Tags updated.",
        _ => "Task activity recorded.",
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
    use serde_json::json;
    use tempfile::tempdir;

    use crate::{
        ChangeRequestKind, CreateChangeRequestInput, CreateProjectInput, CreateTaskInput,
        TaskStatus, TmHome,
    };

    use super::{DesktopCommand, execute_desktop_command};

    #[test]
    fn desktop_command_allowlist_round_trips() -> crate::Result<()> {
        let commands = [
            DesktopCommand::GetAppSnapshot,
            DesktopCommand::GetCalendarMonth,
            DesktopCommand::CreateCalendarEvent,
            DesktopCommand::UpdateCalendarEvent,
            DesktopCommand::DeleteCalendarEvent,
            DesktopCommand::GetStockWatchlist,
            DesktopCommand::UpsertStockWatchlistItem,
            DesktopCommand::DeleteStockWatchlistItem,
            DesktopCommand::CreateProject,
            DesktopCommand::CreateTask,
            DesktopCommand::UpdateTask,
            DesktopCommand::PlanTask,
            DesktopCommand::ResolveDayEntry,
            DesktopCommand::StartSession,
            DesktopCommand::FinishSession,
            DesktopCommand::CreateWorkLog,
            DesktopCommand::CreateNote,
            DesktopCommand::CreateChangeRequest,
            DesktopCommand::UpdateChangeRequest,
            DesktopCommand::ApproveChangeRequest,
            DesktopCommand::ReturnChangeRequestToDraft,
            DesktopCommand::CancelChangeRequest,
            DesktopCommand::AbandonChangeRequest,
            DesktopCommand::ListChangeRequests,
            DesktopCommand::ClaimNextChangeRequest,
            DesktopCommand::CompleteChangeRequest,
            DesktopCommand::FailChangeRequest,
            DesktopCommand::PromoteWorkLog,
            DesktopCommand::Search,
            DesktopCommand::MoveToTrash,
            DesktopCommand::RestoreTrashItem,
            DesktopCommand::CreateBackup,
            DesktopCommand::RestoreBackup,
            DesktopCommand::ExportAll,
        ];
        for command in commands {
            assert_eq!(command.as_str().parse::<DesktopCommand>()?, command);
        }
        assert!("unknown".parse::<DesktopCommand>().is_err());
        Ok(())
    }

    #[test]
    fn snapshot_matches_the_desktop_contract_without_exposing_server_path() -> crate::Result<()> {
        let temporary = tempdir()?;
        let core = crate::TmCore::open(TmHome::new(temporary.path()))?;
        let project = core.create_project(CreateProjectInput {
            name: "Cloud project".to_owned(),
            description: "Remote".to_owned(),
            color: None,
        })?;
        let task = core.create_task(CreateTaskInput {
            project_id: Some(project.id),
            title: "Cloud task".to_owned(),
            description: String::new(),
            status: TaskStatus::InProgress,
            priority: 3,
            due_date: None,
        })?;

        let snapshot = execute_desktop_command(&core, DesktopCommand::GetAppSnapshot, json!({}))?;

        assert_eq!(snapshot["databasePath"], "TM Cloud");
        assert_eq!(snapshot["projects"][0]["name"], "Cloud project");
        assert_eq!(snapshot["tasks"][0]["id"], task.id);
        assert_eq!(snapshot["tasks"][0]["priority"], "high");
        Ok(())
    }

    #[test]
    fn command_arguments_are_strict_and_create_task_matches_desktop_shape() -> crate::Result<()> {
        let temporary = tempdir()?;
        let core = crate::TmCore::open(TmHome::new(temporary.path()))?;

        execute_desktop_command(
            &core,
            DesktopCommand::CreateTask,
            json!({
                "input": {
                    "title": "Created remotely",
                    "description": "",
                    "projectId": null,
                    "status": "todo",
                    "priority": "medium",
                    "dueDate": null,
                    "tags": []
                }
            }),
        )?;

        let tasks = core.list_tasks(false)?;
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Created remotely");
        assert_eq!(tasks[0].priority, 2);
        assert!(
            execute_desktop_command(
                &core,
                DesktopCommand::CreateTask,
                json!({"input": {"title": "bad", "unexpected": true}}),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn cloud_change_request_commands_claim_once_and_require_the_claim_identity() -> crate::Result<()>
    {
        let temporary = tempdir()?;
        let core = crate::TmCore::open(TmHome::new(temporary.path()))?;
        let request = core.create_change_request(CreateChangeRequestInput {
            kind: ChangeRequestKind::Ui,
            project_id: None,
            task_id: None,
            title: "One-touch completion".to_owned(),
            description: "The current flow opens the detail drawer.".to_owned(),
            desired_outcome: "Complete from the list after confirmation.".to_owned(),
            reproduction_steps: String::new(),
            priority: 2,
            requested_by: "desktop-user".to_owned(),
        })?;
        core.approve_change_request(
            &request.id,
            request.revision,
            request.attempt_count,
            "desktop-user",
        )?;

        let listed = execute_desktop_command(&core, DesktopCommand::ListChangeRequests, json!({}))?;
        assert_eq!(listed.as_array().map(Vec::len), Some(1));

        let claim = execute_desktop_command(
            &core,
            DesktopCommand::ClaimNextChangeRequest,
            json!({"workerId": "codex-cloud"}),
        )?;
        assert_eq!(claim["shouldProcess"], true);
        assert_eq!(claim["request"]["id"], request.id);
        let claim_key = claim["claimKey"].as_str().expect("claim key");

        let empty_claim = execute_desktop_command(
            &core,
            DesktopCommand::ClaimNextChangeRequest,
            json!({"workerId": "other-worker"}),
        )?;
        assert_eq!(empty_claim["shouldProcess"], false);

        assert!(
            execute_desktop_command(
                &core,
                DesktopCommand::CompleteChangeRequest,
                json!({
                    "requestId": request.id,
                    "claimKey": claim_key,
                    "resultSummary": "Implemented",
                    "patchRef": "0005",
                    "workerId": "other-worker"
                }),
            )
            .is_err()
        );

        let completed = execute_desktop_command(
            &core,
            DesktopCommand::CompleteChangeRequest,
            json!({
                "requestId": request.id,
                "claimKey": claim_key,
                "resultSummary": "Implemented",
                "patchRef": "0005",
                "workerId": "codex-cloud"
            }),
        )?;
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["resultSummary"], "Implemented");
        assert_eq!(completed["patchRef"], "0005");
        Ok(())
    }
}
