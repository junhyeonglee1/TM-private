use axum::{
    Extension, Json,
    extract::{Query, State, rejection::QueryRejection},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tm_core::{
    ChecklistItem, Note, NoteType, Project, SessionStatus, Tag, Task, TaskStatus, TmCore, WorkLog,
    WorkSession,
};
use uuid::Uuid;

use super::{ApiEnvelope, ApiError, AppState, RequestId};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;
const MAX_OFFSET: usize = 10_000;
const MAX_RESPONSE_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy)]
struct PageSpec {
    limit: usize,
    offset: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Collection<T> {
    items: Vec<T>,
    page: PageInfo,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    limit: usize,
    offset: usize,
    returned: usize,
    total: usize,
    next_offset: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ProjectsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    archived: Option<bool>,
    sort: Option<ProjectSort>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum ProjectSort {
    #[default]
    Name,
    UpdatedDesc,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TasksQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    project_id: Option<String>,
    status: Option<TaskStatus>,
    due_from: Option<NaiveDate>,
    due_to: Option<NaiveDate>,
    sort: Option<TaskSort>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum TaskSort {
    #[default]
    UpdatedDesc,
    DueAsc,
    PriorityDesc,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ChecklistQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    task_id: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TagsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    task_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SessionsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    project_id: Option<String>,
    status: Option<SessionStatus>,
    sort: Option<SessionSort>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SessionSort {
    #[default]
    StartedDesc,
    UpdatedDesc,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct WorklogsQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    project_id: Option<String>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    sort: Option<WorklogSort>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum WorklogSort {
    #[default]
    DateDesc,
    UpdatedDesc,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct NotesQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    note_type: Option<NoteType>,
    date_from: Option<NaiveDate>,
    date_to: Option<NaiveDate>,
    sort: Option<NoteSort>,
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum NoteSort {
    #[default]
    UpdatedDesc,
    DateDesc,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectDto {
    id: String,
    name: String,
    description: String,
    color: Option<String>,
    sort_order: i64,
    created_at: String,
    updated_at: String,
    archived_at: Option<String>,
}

impl From<Project> for ProjectDto {
    fn from(value: Project) -> Self {
        Self {
            id: value.id,
            name: value.name,
            description: value.description,
            color: value.color,
            sort_order: value.sort_order,
            created_at: value.created_at,
            updated_at: value.updated_at,
            archived_at: value.archived_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDto {
    id: String,
    project_id: Option<String>,
    title: String,
    description: String,
    status: TaskStatus,
    priority: u8,
    due_date: Option<NaiveDate>,
    completed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<Task> for TaskDto {
    fn from(value: Task) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            title: value.title,
            description: value.description,
            status: value.status,
            priority: value.priority,
            due_date: value.due_date,
            completed_at: value.completed_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChecklistItemDto {
    id: String,
    task_id: String,
    body: String,
    is_done: bool,
    sort_order: i64,
    created_at: String,
    updated_at: String,
    completed_at: Option<String>,
}

impl From<ChecklistItem> for ChecklistItemDto {
    fn from(value: ChecklistItem) -> Self {
        Self {
            id: value.id,
            task_id: value.task_id,
            body: value.body,
            is_done: value.is_done,
            sort_order: value.sort_order,
            created_at: value.created_at,
            updated_at: value.updated_at,
            completed_at: value.completed_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TagDto {
    id: String,
    name: String,
    color: Option<String>,
    created_at: String,
}

impl From<Tag> for TagDto {
    fn from(value: Tag) -> Self {
        Self {
            id: value.id,
            name: value.name,
            color: value.color,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionDto {
    id: String,
    project_id: Option<String>,
    goal: String,
    status: SessionStatus,
    started_at: String,
    ended_at: Option<String>,
    result: String,
    blockers: String,
    next_action: String,
    created_at: String,
    updated_at: String,
}

impl From<WorkSession> for SessionDto {
    fn from(value: WorkSession) -> Self {
        Self {
            id: value.id,
            project_id: value.project_id,
            goal: value.goal,
            status: value.status,
            started_at: value.started_at,
            ended_at: value.ended_at,
            result: value.result,
            blockers: value.blockers,
            next_action: value.next_action,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorklogDto {
    id: String,
    session_id: Option<String>,
    project_id: Option<String>,
    log_date: NaiveDate,
    title: String,
    body: String,
    created_at: String,
    updated_at: String,
}

impl From<WorkLog> for WorklogDto {
    fn from(value: WorkLog) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            project_id: value.project_id,
            log_date: value.log_date,
            title: value.title,
            body: value.body,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NoteDto {
    id: String,
    note_type: NoteType,
    title: String,
    body: String,
    source_worklog_id: Option<String>,
    note_date: Option<NaiveDate>,
    created_at: String,
    updated_at: String,
}

impl From<Note> for NoteDto {
    fn from(value: Note) -> Self {
        Self {
            id: value.id,
            note_type: value.note_type,
            title: value.title,
            body: value.body,
            source_worklog_id: value.source_worklog_id,
            note_date: value.note_date,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

pub(super) async fn projects(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<ProjectsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let archived = query.archived.unwrap_or(false);
    let mut items = core_read(state.core, &request_id, |core| core.list_projects(false)).await?;
    items.retain(|item| item.archived_at.is_some() == archived);
    match query.sort.unwrap_or_default() {
        ProjectSort::Name => items.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        }),
        ProjectSort::UpdatedDesc => items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
    }
    collection_response(
        request_id,
        &headers,
        "projects",
        items.into_iter().map(ProjectDto::from).collect(),
        page,
    )
}

pub(super) async fn tasks(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<TasksQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_optional_id(query.project_id.as_deref(), "projectId", &request_id)?;
    validate_date_range(
        query.due_from,
        query.due_to,
        "dueFrom",
        "dueTo",
        &request_id,
    )?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let mut items = core_read(state.core, &request_id, |core| core.list_tasks(false)).await?;
    items.retain(|item| {
        query
            .project_id
            .as_ref()
            .is_none_or(|project_id| item.project_id.as_ref() == Some(project_id))
            && query.status.is_none_or(|status| item.status == status)
            && optional_date_in_range(item.due_date, query.due_from, query.due_to)
    });
    match query.sort.unwrap_or_default() {
        TaskSort::UpdatedDesc => items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
        TaskSort::DueAsc => items.sort_by(|left, right| {
            left.due_date
                .is_none()
                .cmp(&right.due_date.is_none())
                .then_with(|| left.due_date.cmp(&right.due_date))
                .then_with(|| left.id.cmp(&right.id))
        }),
        TaskSort::PriorityDesc => items.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.id.cmp(&right.id))
        }),
    }
    collection_response(
        request_id,
        &headers,
        "tasks",
        items.into_iter().map(TaskDto::from).collect(),
        page,
    )
}

pub(super) async fn checklist(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<ChecklistQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_id(&query.task_id, "taskId", &request_id)?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let task_id = query.task_id;
    let mut items = core_read(state.core, &request_id, move |core| {
        core.list_checklist_items(&task_id)
    })
    .await?;
    items.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.id.cmp(&right.id))
    });
    collection_response(
        request_id,
        &headers,
        "checklist",
        items.into_iter().map(ChecklistItemDto::from).collect(),
        page,
    )
}

pub(super) async fn tags(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<TagsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_optional_id(query.task_id.as_deref(), "taskId", &request_id)?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let task_id = query.task_id;
    let mut items = core_read(state.core, &request_id, move |core| {
        core.list_tags(task_id.as_deref())
    })
    .await?;
    items.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    collection_response(
        request_id,
        &headers,
        "tags",
        items.into_iter().map(TagDto::from).collect(),
        page,
    )
}

pub(super) async fn sessions(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<SessionsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_optional_id(query.project_id.as_deref(), "projectId", &request_id)?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let mut items = core_read(state.core, &request_id, |core| core.list_sessions(false)).await?;
    items.retain(|item| {
        query
            .project_id
            .as_ref()
            .is_none_or(|project_id| item.project_id.as_ref() == Some(project_id))
            && query.status.is_none_or(|status| item.status == status)
    });
    match query.sort.unwrap_or_default() {
        SessionSort::StartedDesc => items.sort_by(|left, right| {
            right
                .started_at
                .cmp(&left.started_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
        SessionSort::UpdatedDesc => items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
    }
    collection_response(
        request_id,
        &headers,
        "sessions",
        items.into_iter().map(SessionDto::from).collect(),
        page,
    )
}

pub(super) async fn worklogs(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<WorklogsQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_optional_id(query.project_id.as_deref(), "projectId", &request_id)?;
    validate_date_range(
        query.date_from,
        query.date_to,
        "dateFrom",
        "dateTo",
        &request_id,
    )?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let mut items = core_read(state.core, &request_id, |core| core.list_worklogs(false)).await?;
    items.retain(|item| {
        query
            .project_id
            .as_ref()
            .is_none_or(|project_id| item.project_id.as_ref() == Some(project_id))
            && date_in_range(item.log_date, query.date_from, query.date_to)
    });
    match query.sort.unwrap_or_default() {
        WorklogSort::DateDesc => items.sort_by(|left, right| {
            right
                .log_date
                .cmp(&left.log_date)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| left.id.cmp(&right.id))
        }),
        WorklogSort::UpdatedDesc => items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
    }
    collection_response(
        request_id,
        &headers,
        "worklogs",
        items.into_iter().map(WorklogDto::from).collect(),
        page,
    )
}

pub(super) async fn notes(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    query: Result<Query<NotesQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let query = parse_query(query, &request_id)?;
    validate_date_range(
        query.date_from,
        query.date_to,
        "dateFrom",
        "dateTo",
        &request_id,
    )?;
    let page = page_spec(query.limit, query.offset, &request_id)?;
    let mut items = core_read(state.core, &request_id, |core| core.list_notes(false)).await?;
    items.retain(|item| {
        query
            .note_type
            .is_none_or(|note_type| item.note_type == note_type)
            && optional_date_in_range(item.note_date, query.date_from, query.date_to)
    });
    match query.sort.unwrap_or_default() {
        NoteSort::UpdatedDesc => items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        }),
        NoteSort::DateDesc => items.sort_by(|left, right| {
            right
                .note_date
                .cmp(&left.note_date)
                .then_with(|| right.created_at.cmp(&left.created_at))
                .then_with(|| left.id.cmp(&right.id))
        }),
    }
    collection_response(
        request_id,
        &headers,
        "notes",
        items.into_iter().map(NoteDto::from).collect(),
        page,
    )
}

fn parse_query<T>(
    query: Result<Query<T>, QueryRejection>,
    request_id: &RequestId,
) -> Result<T, ApiError> {
    query.map(|Query(value)| value).map_err(|_| ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_QUERY",
        message: "query parameters are invalid or unsupported".to_owned(),
        request_id: request_id.0.clone(),
    })
}

fn page_spec(
    limit: Option<usize>,
    offset: Option<usize>,
    request_id: &RequestId,
) -> Result<PageSpec, ApiError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let offset = offset.unwrap_or(0);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(invalid_query(
            format!("limit must be between 1 and {MAX_LIMIT}"),
            request_id,
        ));
    }
    if offset > MAX_OFFSET {
        return Err(invalid_query(
            format!("offset must not exceed {MAX_OFFSET}"),
            request_id,
        ));
    }
    Ok(PageSpec { limit, offset })
}

fn validate_optional_id(
    value: Option<&str>,
    name: &str,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    value.map_or(Ok(()), |value| validate_id(value, name, request_id))
}

fn validate_id(value: &str, name: &str, request_id: &RequestId) -> Result<(), ApiError> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| invalid_query(format!("{name} must be a UUID"), request_id))
}

fn validate_date_range(
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    from_name: &str,
    to_name: &str,
    request_id: &RequestId,
) -> Result<(), ApiError> {
    if from.zip(to).is_some_and(|(from, to)| from > to) {
        return Err(invalid_query(
            format!("{from_name} must not be later than {to_name}"),
            request_id,
        ));
    }
    Ok(())
}

fn date_in_range(value: NaiveDate, from: Option<NaiveDate>, to: Option<NaiveDate>) -> bool {
    from.is_none_or(|from| value >= from) && to.is_none_or(|to| value <= to)
}

fn optional_date_in_range(
    value: Option<NaiveDate>,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> bool {
    if from.is_none() && to.is_none() {
        return true;
    }
    value.is_some_and(|value| date_in_range(value, from, to))
}

fn invalid_query(message: String, request_id: &RequestId) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        code: "INVALID_QUERY",
        message,
        request_id: request_id.0.clone(),
    }
}

async fn core_read<T>(
    core: TmCore,
    request_id: &RequestId,
    operation: impl FnOnce(&TmCore) -> tm_core::Result<T> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
{
    let error_request_id = request_id.0.clone();
    tokio::task::spawn_blocking(move || operation(&core))
        .await
        .map_err(|_| ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: "READ_WORKER_FAILED",
            message: "TM read worker failed".to_owned(),
            request_id: error_request_id.clone(),
        })?
        .map_err(|_| {
            tracing::error!(request_id = %error_request_id, "TM read operation failed");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "DATABASE_READ_FAILED",
                message: "TM data could not be read".to_owned(),
                request_id: error_request_id,
            }
        })
}

fn collection_response<T: Serialize>(
    request_id: RequestId,
    request_headers: &HeaderMap,
    resource: &'static str,
    items: Vec<T>,
    page: PageSpec,
) -> Result<Response, ApiError> {
    let total = items.len();
    let page_items = items
        .into_iter()
        .skip(page.offset)
        .take(page.limit)
        .collect::<Vec<_>>();
    let returned = page_items.len();
    let next_offset = (page.offset + returned < total).then_some(page.offset + returned);
    let data = Collection {
        items: page_items,
        page: PageInfo {
            limit: page.limit,
            offset: page.offset,
            returned,
            total,
            next_offset,
        },
    };
    let version_bytes = serde_json::to_vec(&data).map_err(|_| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "RESPONSE_SERIALIZATION_FAILED",
        message: "TM response could not be serialized".to_owned(),
        request_id: request_id.0.clone(),
    })?;
    let envelope = ApiEnvelope {
        request_id: request_id.0.clone(),
        data,
    };
    let serialized = serde_json::to_vec(&envelope).map_err(|_| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "RESPONSE_SERIALIZATION_FAILED",
        message: "TM response could not be serialized".to_owned(),
        request_id: request_id.0.clone(),
    })?;
    if serialized.len() > MAX_RESPONSE_BYTES {
        return Err(ApiError {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code: "RESPONSE_TOO_LARGE",
            message: format!("response exceeds {MAX_RESPONSE_BYTES} bytes; request a smaller page"),
            request_id: request_id.0,
        });
    }

    let hash = Sha256::digest(&version_bytes);
    let etag = format!("\"{hash:x}\"");
    let etag_header = HeaderValue::from_str(&etag).map_err(|_| ApiError {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        code: "ETAG_GENERATION_FAILED",
        message: "TM response version could not be generated".to_owned(),
        request_id: request_id.0.clone(),
    })?;
    let not_modified = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate == "*" || candidate == etag)
        });

    tracing::info!(
        request_id = %request_id.0,
        resource,
        total,
        returned,
        offset = page.offset,
        "read-only TM API request completed"
    );

    if not_modified {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        response.headers_mut().insert(header::ETAG, etag_header);
        return Ok(response);
    }

    let mut response = Json(envelope).into_response();
    response.headers_mut().insert(header::ETAG, etag_header);
    Ok(response)
}
