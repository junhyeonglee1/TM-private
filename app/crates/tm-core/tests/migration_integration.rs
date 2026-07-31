use std::path::PathBuf;

use rusqlite::{Connection, functions::FunctionFlags, params};
use tempfile::{Builder, TempDir};
use tm_core::{
    CreateTaskInput, DEFAULT_TM_HOME, Error, Result, TaskPatch, TaskStatus, TmCore, TmHome,
};
use uuid::Uuid;

const SCHEMA_THIRTEEN_MIGRATIONS: [&str; 13] = [
    include_str!("../migrations/0001_initial.sql"),
    include_str!("../migrations/0002_change_requests.sql"),
    include_str!("../migrations/0003_change_request_strict_cas.sql"),
    include_str!("../migrations/0004_controlled_mutations.sql"),
    include_str!("../migrations/0005_ai_budget_guard.sql"),
    include_str!("../migrations/0006_assistant_action_approvals.sql"),
    include_str!("../migrations/0007_assistant_memory.sql"),
    include_str!("../migrations/0008_durable_scheduler.sql"),
    include_str!("../migrations/0009_device_auth.sql"),
    include_str!("../migrations/0010_task_reports.sql"),
    include_str!("../migrations/0011_calendar_events.sql"),
    include_str!("../migrations/0012_stock_watchlist.sql"),
    include_str!("../migrations/0013_stock_daily_screen.sql"),
];

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-migration-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn schema_thirteen_fixture(prefix: &str) -> Result<(TempDir, PathBuf)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new().prefix(prefix).tempdir_in(test_runs)?;
    let database_path = temporary.path().join("data").join("tm.sqlite3");
    std::fs::create_dir_all(
        database_path.parent().ok_or_else(|| {
            Error::Invariant("schema 13 fixture database has no parent".to_owned())
        })?,
    )?;
    let connection = Connection::open(&database_path)?;
    connection.create_scalar_function("tm_uuid_v7", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(Uuid::now_v7().to_string())
    })?;
    connection.create_scalar_function("tm_now_utc", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok("2026-07-30T00:00:00.000Z".to_owned())
    })?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    for (index, migration) in SCHEMA_THIRTEEN_MIGRATIONS.iter().enumerate() {
        let version = i64::try_from(index + 1)
            .map_err(|error| Error::Invariant(format!("invalid fixture version: {error}")))?;
        connection.execute_batch(migration)?;
        connection.execute(
            "INSERT INTO schema_migrations(version, name, applied_at)
             VALUES (?1, ?2, '2026-07-30T00:00:00.000Z')",
            params![version, format!("schema-{version}-fixture")],
        )?;
        connection.pragma_update(None, "user_version", version)?;
    }
    drop(connection);
    Ok((temporary, database_path))
}

fn task_input(title: &str, description: &str) -> CreateTaskInput {
    CreateTaskInput {
        project_id: None,
        title: title.to_owned(),
        description: description.to_owned(),
        status: TaskStatus::Todo,
        priority: 0,
        due_date: None,
    }
}

#[test]
fn schema_fourteen_adopts_one_active_uncategorized_project_and_backfills_tasks() -> Result<()> {
    let (temporary, database_path) = schema_thirteen_fixture("tm-schema14-adopt-")?;
    let oldest_id = Uuid::now_v7().to_string();
    let duplicate_id = Uuid::now_v7().to_string();
    let archived_id = Uuid::now_v7().to_string();
    let deleted_id = Uuid::now_v7().to_string();
    let active_task_id = Uuid::now_v7().to_string();
    let deleted_task_id = Uuid::now_v7().to_string();
    let done_task_id = Uuid::now_v7().to_string();
    let original_updated_at = "2026-07-01T01:02:03.000Z";
    let done_created_at = "2026-06-01T01:00:00.000Z";
    let done_updated_at = "2026-06-02T02:00:00.000Z";
    let done_completed_at = "2026-06-02T01:30:00.000Z";
    let connection = Connection::open(&database_path)?;
    connection.create_scalar_function("tm_uuid_v7", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok(Uuid::now_v7().to_string())
    })?;
    connection.create_scalar_function("tm_now_utc", 0, FunctionFlags::SQLITE_UTF8, |_| {
        Ok("2026-07-30T00:00:00.000Z".to_owned())
    })?;
    connection.execute(
        "INSERT INTO projects(
            id, name, description, sort_order, created_at, updated_at
         ) VALUES (?1, '  기타  ', '', 0, '2026-01-01T00:00:00.000Z', ?5),
                  (?2, '기타', '', 0, '2026-02-01T00:00:00.000Z', ?5),
                  (?3, '기타', '', 0, '2025-01-01T00:00:00.000Z', ?5),
                  (?4, '기타', '', 0, '2024-01-01T00:00:00.000Z', ?5)",
        params![
            oldest_id,
            duplicate_id,
            archived_id,
            deleted_id,
            original_updated_at
        ],
    )?;
    connection.execute(
        "UPDATE projects
         SET archived_at = '2026-03-01T00:00:00.000Z'
         WHERE id = ?1",
        [&archived_id],
    )?;
    connection.execute(
        "UPDATE projects
         SET deleted_at = '2026-03-02T00:00:00.000Z'
         WHERE id = ?1",
        [&deleted_id],
    )?;
    connection.execute(
        "INSERT INTO tasks(
            id, project_id, title, description, status, priority,
            completed_at, created_at, updated_at, deleted_at, version
         ) VALUES (?1, NULL, 'legacy active', '', 'todo', 0, NULL, ?4, ?4, NULL, 7),
                  (?2, NULL, 'legacy deleted', '', 'todo', 0, NULL, ?4, ?4,
                   '2026-07-02T00:00:00.000Z', 4),
                  (?3, NULL, 'legacy done title', 'legacy done description',
                   'done', 3, '2026-06-02T01:30:00.000Z',
                   '2026-06-01T01:00:00.000Z', '2026-06-02T02:00:00.000Z', NULL, 3)",
        params![
            active_task_id,
            deleted_task_id,
            done_task_id,
            original_updated_at
        ],
    )?;
    drop(connection);

    let core = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(core.health()?.schema_version, 14);
    let projects = core.list_projects(true)?;
    let system_projects = projects
        .iter()
        .filter(|project| project.system_key.as_deref() == Some("uncategorized"))
        .collect::<Vec<_>>();
    assert_eq!(system_projects.len(), 1);
    assert_eq!(system_projects[0].id, oldest_id);
    assert_eq!(system_projects[0].name, "기타");
    assert!(
        projects
            .iter()
            .find(|project| project.id == duplicate_id)
            .is_some_and(|project| project.system_key.is_none())
    );
    assert!(
        projects
            .iter()
            .find(|project| project.id == archived_id)
            .is_some_and(|project| project.archived_at.is_some())
    );
    assert!(
        projects
            .iter()
            .find(|project| project.id == deleted_id)
            .is_some_and(|project| project.deleted_at.is_some())
    );

    let active_task = core.get_task(&active_task_id)?;
    assert_eq!(active_task.project_id.as_deref(), Some(oldest_id.as_str()));
    assert_eq!(active_task.version, 8);
    assert_eq!(active_task.updated_at, original_updated_at);
    let deleted_task = core.get_task(&deleted_task_id)?;
    assert_eq!(deleted_task.project_id.as_deref(), Some(oldest_id.as_str()));
    assert_eq!(deleted_task.version, 5);
    assert_eq!(deleted_task.updated_at, original_updated_at);
    assert_eq!(
        deleted_task.deleted_at.as_deref(),
        Some("2026-07-02T00:00:00.000Z")
    );
    let done_task = core.get_task(&done_task_id)?;
    assert_eq!(done_task.project_id.as_deref(), Some(oldest_id.as_str()));
    assert_eq!(done_task.title, "legacy done title");
    assert_eq!(done_task.description, "legacy done description");
    assert_eq!(done_task.status, TaskStatus::Done);
    assert_eq!(done_task.priority, 3);
    assert_eq!(done_task.completed_at.as_deref(), Some(done_completed_at));
    assert_eq!(done_task.created_at, done_created_at);
    assert_eq!(done_task.updated_at, done_updated_at);
    assert!(done_task.deleted_at.is_none());
    assert_eq!(done_task.version, 4);

    let connection = Connection::open(&database_path)?;
    let null_tasks: i64 = connection.query_row(
        "SELECT COUNT(*) FROM tasks WHERE project_id IS NULL",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(null_tasks, 0);
    for task_id in [&active_task_id, &deleted_task_id, &done_task_id] {
        let migration_events: i64 = connection.query_row(
            "SELECT COUNT(*)
             FROM task_events
             WHERE task_id = ?1
               AND event_type = 'task.changed'
               AND json_extract(after_json, '$.projectId') = ?2",
            params![task_id, oldest_id],
            |row| row.get(0),
        )?;
        assert_eq!(migration_events, 1);
    }
    let event_count_before_reopen: i64 =
        connection.query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))?;
    drop(connection);
    assert!(
        core.list_backups()?
            .iter()
            .any(|backup| backup.file_name.contains("pre-migration"))
    );

    let reopened = TmCore::open(TmHome::new(temporary.path()))?;
    let connection = Connection::open(reopened.home().database_path())?;
    let event_count_after_reopen: i64 =
        connection.query_row("SELECT COUNT(*) FROM task_events", [], |row| row.get(0))?;
    assert_eq!(event_count_after_reopen, event_count_before_reopen);
    assert_eq!(
        reopened
            .list_projects(true)?
            .iter()
            .filter(|project| project.system_key.as_deref() == Some("uncategorized"))
            .count(),
        1
    );
    Ok(())
}

#[test]
fn schema_fourteen_does_not_revive_archived_or_deleted_name_matches() -> Result<()> {
    let (temporary, database_path) = schema_thirteen_fixture("tm-schema14-create-")?;
    let archived_id = Uuid::now_v7().to_string();
    let deleted_id = Uuid::now_v7().to_string();
    let connection = Connection::open(&database_path)?;
    connection.execute(
        "INSERT INTO projects(
            id, name, description, sort_order, created_at, updated_at,
            archived_at, deleted_at
         ) VALUES (?1, '기타', '', 0, '2026-01-01T00:00:00.000Z',
                   '2026-01-01T00:00:00.000Z', '2026-02-01T00:00:00.000Z', NULL),
                  (?2, '기타', '', 0, '2026-01-02T00:00:00.000Z',
                   '2026-01-02T00:00:00.000Z', NULL, '2026-02-02T00:00:00.000Z')",
        params![archived_id, deleted_id],
    )?;
    drop(connection);

    let core = TmCore::open(TmHome::new(temporary.path()))?;
    let projects = core.list_projects(true)?;
    let system = projects
        .iter()
        .find(|project| project.system_key.as_deref() == Some("uncategorized"))
        .ok_or_else(|| Error::Invariant("uncategorized project missing".to_owned()))?;
    assert_ne!(system.id, archived_id);
    assert_ne!(system.id, deleted_id);
    assert!(system.archived_at.is_none());
    assert!(system.deleted_at.is_none());
    assert_eq!(projects.len(), 3);
    Ok(())
}

#[test]
fn manifest_is_deterministic_and_does_not_expose_row_contents() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let secret_title = "manifest에 노출되면 안 되는 합성 제목";
    let secret_description = "합성 설명 원문";
    core.create_task(task_input(secret_title, secret_description))?;

    let first = core.migration_manifest()?;
    let second = core.migration_manifest()?;
    assert!(first.logically_matches(&second));
    assert_eq!(first.schema_version, 14);
    assert_eq!(
        first.migration_versions,
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14]
    );
    assert_eq!(first.logical_sha256.len(), 64);
    assert_eq!(first.tables["tasks"].row_count, 1);
    assert_eq!(first.tables["task_events"].row_count, 1);

    let serialized = serde_json::to_string(&first)?;
    assert!(!serialized.contains(secret_title));
    assert!(!serialized.contains(secret_description));
    Ok(())
}

#[test]
fn dry_run_snapshot_passes_read_only_preflight_and_logical_comparison() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.create_task(task_input("dry-run 합성 Task", "snapshot 비교"))?;

    let dry_run = core.migration_dry_run()?;
    assert!(dry_run.verified);
    assert!(dry_run.source.logically_matches(&dry_run.snapshot));
    assert!(std::path::Path::new(&dry_run.snapshot_artifact.path).is_file());

    let inspected = TmCore::inspect_migration_database(&dry_run.snapshot_artifact.path)?;
    assert!(dry_run.source.logically_matches(&inspected));
    Ok(())
}

#[test]
fn preflight_rejects_a_database_with_an_unsupported_schema() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let backup = core.create_backup()?;
    let connection = Connection::open(&backup.path)?;
    connection.pragma_update(None, "user_version", 99_i64)?;
    drop(connection);

    let result = TmCore::inspect_migration_database(&backup.path);
    assert!(matches!(result, Err(Error::InvalidBackup(_))));
    Ok(())
}

#[test]
fn schema_fourteen_semantics_reject_missing_triggers_and_index_everywhere() -> Result<()> {
    let (corrupt_temporary, corrupt_core) = fixture()?;
    let active_database = corrupt_core.home().database_path().to_path_buf();
    let connection = Connection::open(&active_database)?;
    connection.execute("DROP TRIGGER tasks_project_required_insert", [])?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(connection);
    assert!(!corrupt_core.health()?.ok);
    assert!(matches!(
        corrupt_core.create_backup(),
        Err(Error::InvalidBackup(_))
    ));
    drop(corrupt_core);
    assert!(TmCore::open(TmHome::new(corrupt_temporary.path())).is_err());

    let (temporary, core) = fixture()?;
    core.create_task(task_input(
        "schema semantics backup fixture",
        "orphan reference detection",
    ))?;
    let valid_backup = core.create_backup()?;
    for (suffix, corruption_statement) in [
        (
            "missing-trigger",
            "DROP TRIGGER projects_uncategorized_protect_delete;",
        ),
        ("missing-index", "DROP INDEX idx_projects_system_key;"),
        (
            "orphan-task-project",
            "UPDATE tasks SET project_id = '00000000-0000-0000-0000-000000000000';",
        ),
    ] {
        let corrupted = temporary.path().join(format!("{suffix}.sqlite3"));
        std::fs::copy(&valid_backup.path, &corrupted)?;
        let connection = Connection::open(&corrupted)?;
        connection.create_scalar_function("tm_uuid_v7", 0, FunctionFlags::SQLITE_UTF8, |_| {
            Ok(Uuid::now_v7().to_string())
        })?;
        connection.create_scalar_function("tm_now_utc", 0, FunctionFlags::SQLITE_UTF8, |_| {
            Ok("2026-07-30T00:00:00.000Z".to_owned())
        })?;
        connection.execute_batch(corruption_statement)?;
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
        drop(connection);

        assert!(TmCore::inspect_migration_database(&corrupted).is_err());
        assert!(matches!(
            core.restore_backup(&corrupted),
            Err(Error::InvalidBackup(_))
        ));
        assert!(core.health()?.ok);
    }
    Ok(())
}

#[test]
fn restore_rejects_schema_fourteen_backups_with_an_incomplete_manifest() -> Result<()> {
    let (temporary, core) = fixture()?;
    let backup = core.create_backup()?;

    let missing_migration = temporary.path().join("missing-migration.sqlite3");
    std::fs::copy(&backup.path, &missing_migration)?;
    let connection = Connection::open(&missing_migration)?;
    connection.execute("DELETE FROM schema_migrations WHERE version = 14", [])?;
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(connection);
    assert!(matches!(
        core.restore_backup(&missing_migration),
        Err(Error::InvalidBackup(_))
    ));

    let missing_table = temporary.path().join("missing-required-table.sqlite3");
    std::fs::copy(&backup.path, &missing_table)?;
    let connection = Connection::open(&missing_table)?;
    connection.execute_batch(
        "DROP TABLE stock_watchlist_items;
         PRAGMA wal_checkpoint(TRUNCATE);",
    )?;
    drop(connection);
    assert!(matches!(
        core.restore_backup(&missing_table),
        Err(Error::InvalidBackup(_))
    ));
    Ok(())
}

#[test]
fn snapshot_can_restore_the_logical_state_after_a_failed_cutover() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("rollback 이전", "보존되어야 하는 상태"))?;
    let before = core.migration_manifest()?;
    let backup = core.create_backup()?;

    core.update_task(
        &task.id,
        TaskPatch {
            title: Some("rollback 이후 변경".to_owned()),
            ..TaskPatch::default()
        },
    )?;
    assert!(!before.logically_matches(&core.migration_manifest()?));

    core.restore_backup(&backup.path)?;
    let restored = core.migration_manifest()?;
    assert!(before.logically_matches(&restored));
    Ok(())
}
