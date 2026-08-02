use std::{fmt, str::FromStr};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = Error;

            fn from_str(value: &str) -> Result<Self> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(Error::Invariant(format!(
                        "unknown {} value: {other}",
                        stringify!($name)
                    ))),
                }
            }
        }
    };
}

string_enum!(TaskStatus {
    Inbox => "inbox",
    Todo => "todo",
    InProgress => "in_progress",
    Blocked => "blocked",
    Done => "done",
    Cancelled => "cancelled",
});

string_enum!(TaskDayStatus {
    Planned => "planned",
    Done => "done",
    Deferred => "deferred",
    Skipped => "skipped",
});

string_enum!(SessionStatus {
    Running => "running",
    Completed => "completed",
    Cancelled => "cancelled",
});

string_enum!(NoteType {
    Concept => "concept",
    Howto => "howto",
    Decision => "decision",
    Reference => "reference",
    Daily => "daily",
});

string_enum!(EntityType {
    Task => "task",
    Session => "session",
    Worklog => "worklog",
    Note => "note",
});

string_enum!(LinkTargetType {
    Task => "task",
    Session => "session",
    Worklog => "worklog",
    Note => "note",
    File => "file",
    Url => "url",
});

string_enum!(ChangeRequestKind {
    Bug => "bug",
    Ui => "ui",
    Feature => "feature",
    Other => "other",
});

string_enum!(ChangeRequestStatus {
    Draft => "draft",
    Approved => "approved",
    Claimed => "claimed",
    Completed => "completed",
    Failed => "failed",
    Cancelled => "cancelled",
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequest {
    pub id: String,
    pub kind: ChangeRequestKind,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    pub description: String,
    pub desired_outcome: String,
    pub reproduction_steps: String,
    pub priority: u8,
    pub status: ChangeRequestStatus,
    pub revision: u32,
    pub approved_revision: Option<u32>,
    pub attempt_count: u32,
    pub requested_by: String,
    pub updated_by: String,
    pub created_at: String,
    pub updated_at: String,
    pub approved_at: Option<String>,
    pub approved_by: Option<String>,
    pub claimed_at: Option<String>,
    pub claimed_by: Option<String>,
    pub completed_at: Option<String>,
    pub completed_by: Option<String>,
    pub failed_at: Option<String>,
    pub failed_by: Option<String>,
    pub cancelled_at: Option<String>,
    pub cancelled_by: Option<String>,
    pub result_summary: Option<String>,
    pub patch_ref: Option<String>,
    pub failure_reason: Option<String>,
    pub cancellation_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestEvent {
    pub id: String,
    pub change_request_id: String,
    pub event_type: String,
    pub from_status: Option<ChangeRequestStatus>,
    pub to_status: ChangeRequestStatus,
    pub revision: u32,
    pub actor: String,
    pub details: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateChangeRequestInput {
    pub kind: ChangeRequestKind,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub desired_outcome: String,
    #[serde(default)]
    pub reproduction_steps: String,
    #[serde(default)]
    pub priority: u8,
    pub requested_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChangeRequestInput {
    pub expected_revision: u32,
    pub expected_attempt_count: u32,
    pub kind: ChangeRequestKind,
    pub project_id: Option<String>,
    pub task_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub desired_outcome: String,
    #[serde(default)]
    pub reproduction_steps: String,
    #[serde(default)]
    pub priority: u8,
    pub updated_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequestClaim {
    pub should_process: bool,
    pub request: Option<ChangeRequest>,
    pub claim_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub system_key: Option<String>,
    pub name: String,
    pub description: String,
    pub color: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub project_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub priority: u8,
    pub due_date: Option<NaiveDate>,
    pub completed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub id: String,
    pub task_id: String,
    pub event_type: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
    pub actor: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistItem {
    pub id: String,
    pub task_id: String,
    pub body: String,
    pub is_done: bool,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskDayEntry {
    pub id: String,
    pub task_id: String,
    pub entry_date: NaiveDate,
    pub status: TaskDayStatus,
    pub note: String,
    pub created_at: String,
    pub updated_at: String,
    pub finalized_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkSession {
    pub id: String,
    pub project_id: Option<String>,
    pub goal: String,
    pub status: SessionStatus,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub result: String,
    pub blockers: String,
    pub next_action: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkLog {
    pub id: String,
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub log_date: NaiveDate,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub note_type: NoteType,
    pub title: String,
    pub body: String,
    pub source_worklog_id: Option<String>,
    pub note_date: Option<NaiveDate>,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EntityLink {
    pub id: String,
    pub source_type: EntityType,
    pub source_id: String,
    pub target_type: LinkTargetType,
    pub target_id: Option<String>,
    pub target_value: Option<String>,
    pub relation: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub owner_type: EntityType,
    pub owner_id: String,
    pub relative_path: String,
    pub original_name: String,
    pub media_type: Option<String>,
    pub byte_size: u64,
    pub sha256: String,
    pub created_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateAttachmentInput {
    pub owner_type: EntityType,
    pub owner_id: String,
    pub relative_path: String,
    pub original_name: String,
    pub media_type: Option<String>,
    pub byte_size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrashEntityType {
    Task,
    Note,
    Project,
    Worklog,
    Session,
}

impl TrashEntityType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Note => "note",
            Self::Project => "project",
            Self::Worklog => "worklog",
            Self::Session => "session",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    pub id: String,
    pub entity_type: TrashEntityType,
    pub title: String,
    pub deleted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub path: String,
    pub file_name: String,
    pub created_at: String,
    pub byte_size: u64,
    pub trigger: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupVerification {
    pub sha256: String,
    pub byte_size: u64,
    pub schema_version: i64,
    pub integrity_check: String,
    pub schema_semantics_validated: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiBudgetPolicy {
    pub warning_limit_microusd: u64,
    pub hard_limit_microusd: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiBudgetStatus {
    pub budget_month: String,
    pub warning_limit_microusd: u64,
    pub hard_limit_microusd: u64,
    pub committed_microusd: u64,
    pub remaining_microusd: u64,
    pub warning_reached: bool,
    pub hard_stop_reached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiBudgetReservation {
    pub request_id: String,
    pub budget_month: String,
    pub reserved_microusd: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiTokenUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub entity_type: EntityType,
    pub entity_id: String,
    pub title: String,
    pub excerpt: String,
    pub rank_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    pub project_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_task_status")]
    pub status: TaskStatus,
    #[serde(default)]
    pub priority: u8,
    pub due_date: Option<NaiveDate>,
}

const fn default_task_status() -> TaskStatus {
    TaskStatus::Inbox
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub priority: Option<u8>,
    pub project_id: Option<String>,
    #[serde(default)]
    pub clear_project: bool,
    pub due_date: Option<NaiveDate>,
    #[serde(default)]
    pub clear_due_date: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskAggregateInput {
    pub task: CreateTaskInput,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChecklistMutationInput {
    pub id: Option<String>,
    pub body: String,
    #[serde(default)]
    pub is_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTaskAggregateInput {
    pub task_id: String,
    pub patch: TaskPatch,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub checklist: Vec<ChecklistMutationInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskAggregate {
    pub task: Task,
    pub tags: Vec<Tag>,
    pub checklist: Vec<ChecklistItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartSessionInput {
    pub project_id: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EndSessionInput {
    pub result: String,
    #[serde(default)]
    pub blockers: String,
    #[serde(default)]
    pub next_action: String,
    #[serde(default)]
    pub create_followup_task: bool,
    pub followup_title: Option<String>,
    pub followup_project_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCompletion {
    pub session: WorkSession,
    pub worklog: WorkLog,
    pub followup_task: Option<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateWorkLogInput {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    pub log_date: Option<NaiveDate>,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteInput {
    pub note_type: NoteType,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub note_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotePatch {
    pub note_type: Option<NoteType>,
    pub title: Option<String>,
    pub body: Option<String>,
    pub note_date: Option<NaiveDate>,
    #[serde(default)]
    pub clear_note_date: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteLinksInput {
    #[serde(default)]
    pub task_ids: Vec<String>,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub worklog_ids: Vec<String>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteAggregateInput {
    pub note: CreateNoteInput,
    #[serde(default)]
    pub links: NoteLinksInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteAggregate {
    pub note: Note,
    pub links: Vec<EntityLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateLinkInput {
    pub source_type: EntityType,
    pub source_id: String,
    pub target_type: LinkTargetType,
    pub target_id: Option<String>,
    pub target_value: Option<String>,
    #[serde(default = "default_relation")]
    pub relation: String,
}

fn default_relation() -> String {
    "related".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub ok: bool,
    pub database_path: String,
    pub schema_version: i64,
    pub sqlite_version: String,
    pub journal_mode: String,
    pub foreign_keys: bool,
    pub integrity_check: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExportArtifact {
    pub json_path: String,
    pub markdown_path: String,
    pub created_at: String,
    pub sha256: String,
}
