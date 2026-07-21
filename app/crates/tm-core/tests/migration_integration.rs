use rusqlite::Connection;
use tempfile::{Builder, TempDir};
use tm_core::{
    CreateTaskInput, DEFAULT_TM_HOME, Error, Result, TaskPatch, TaskStatus, TmCore, TmHome,
};

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
fn manifest_is_deterministic_and_does_not_expose_row_contents() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let secret_title = "manifest에 노출되면 안 되는 합성 제목";
    let secret_description = "합성 설명 원문";
    core.create_task(task_input(secret_title, secret_description))?;

    let first = core.migration_manifest()?;
    let second = core.migration_manifest()?;
    assert!(first.logically_matches(&second));
    assert_eq!(first.schema_version, 8);
    assert_eq!(first.migration_versions, vec![1, 2, 3, 4, 5, 6, 7, 8]);
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
