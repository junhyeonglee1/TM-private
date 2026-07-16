use std::sync::{Arc, Barrier};

use rusqlite::Connection;
use tempfile::TempDir;
use tm_core::{
    CreateNoteInput, CreateProjectInput, CreateTaskInput, Error, MutationApprovalPolicy,
    MutationCommand, MutationExpectedVersion, MutationOperation, MutationRequest, NotePatch,
    NoteType, Result, TaskPatch, TaskStatus, TmCore, TmHome, TrashEntityType,
};
use uuid::Uuid;

fn fixture() -> Result<(TempDir, TmCore)> {
    let temporary = tempfile::Builder::new().prefix("tm-mutation-").tempdir()?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn request(
    idempotency_key: &str,
    request_id: &str,
    expected_version: MutationExpectedVersion,
    command: MutationCommand,
) -> MutationRequest {
    MutationRequest {
        idempotency_key: idempotency_key.to_owned(),
        expected_version,
        actor: "single-user".to_owned(),
        request_id: request_id.to_owned(),
        approval_policy: MutationApprovalPolicy::ExplicitUserConfirmation,
        command,
    }
}

fn task_input(title: &str) -> CreateTaskInput {
    CreateTaskInput {
        project_id: None,
        title: title.to_owned(),
        description: String::new(),
        status: TaskStatus::Todo,
        priority: 0,
        due_date: None,
    }
}

#[test]
fn controlled_mutations_are_versioned_idempotent_and_audited() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let create = request(
        "task-create-key-0001",
        "request-task-create",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: task_input("controlled task"),
        },
    );
    let created = core.execute_remote_mutation(create.clone())?;
    assert_eq!(created.operation, MutationOperation::TaskCreate);
    assert_eq!(created.version, 1);
    assert!(!created.replayed);

    let replay = core.execute_remote_mutation(MutationRequest {
        request_id: "request-task-create-retry".to_owned(),
        ..create.clone()
    })?;
    assert!(replay.replayed);
    assert_eq!(replay.resource_id, created.resource_id);
    assert_eq!(core.list_tasks(false)?.len(), 1);
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);

    let mismatched = core.execute_remote_mutation(MutationRequest {
        command: MutationCommand::TaskCreate {
            input: task_input("different payload"),
        },
        ..create
    });
    assert!(matches!(mismatched, Err(Error::Conflict(_))));

    let updated = core.execute_remote_mutation(request(
        "task-update-key-0001",
        "request-task-update",
        MutationExpectedVersion::Exact(1),
        MutationCommand::TaskUpdate {
            task_id: created.resource_id.clone(),
            patch: TaskPatch {
                title: Some("controlled task updated".to_owned()),
                ..TaskPatch::default()
            },
        },
    ))?;
    assert_eq!(updated.version, 2);
    assert_eq!(
        core.get_task(&created.resource_id)?.title,
        "controlled task updated"
    );

    let stale = core.execute_remote_mutation(request(
        "task-update-key-0002",
        "request-task-stale",
        MutationExpectedVersion::Exact(1),
        MutationCommand::TaskUpdate {
            task_id: created.resource_id.clone(),
            patch: TaskPatch {
                title: Some("stale overwrite".to_owned()),
                ..TaskPatch::default()
            },
        },
    ));
    assert!(matches!(stale, Err(Error::Conflict(_))));
    assert_eq!(core.list_mutation_audit_events()?.len(), 2);

    let note = core.execute_remote_mutation(request(
        "note-create-key-0001",
        "request-note-create",
        MutationExpectedVersion::Absent,
        MutationCommand::NoteCreate {
            input: CreateNoteInput {
                note_type: NoteType::Decision,
                title: "controlled note".to_owned(),
                body: "body".to_owned(),
                note_date: None,
            },
        },
    ))?;
    assert_eq!(note.version, 1);
    let note_updated = core.execute_remote_mutation(request(
        "note-update-key-0001",
        "request-note-update",
        MutationExpectedVersion::Exact(1),
        MutationCommand::NoteUpdate {
            note_id: note.resource_id.clone(),
            patch: NotePatch {
                body: Some("updated body".to_owned()),
                ..NotePatch::default()
            },
        },
    ))?;
    assert_eq!(note_updated.version, 2);

    let item = core.add_checklist_item(&created.resource_id, "controlled checklist")?;
    let item_updated = core.execute_remote_mutation(request(
        "check-update-key-0001",
        "request-check-update",
        MutationExpectedVersion::Exact(item.version),
        MutationCommand::ChecklistSetDone {
            item_id: item.id.clone(),
            is_done: true,
        },
    ))?;
    assert_eq!(item_updated.version, 2);
    assert!(core.list_checklist_items(&created.resource_id)?[0].is_done);

    let audits = core.list_mutation_audit_events()?;
    assert_eq!(audits.len(), 5);
    assert!(audits.iter().all(|event| event.result == "completed"));
    assert!(audits.iter().all(|event| event.actor == "single-user"));
    assert!(audits[0].before.is_none());
    assert!(audits[1].before.is_some());
    Ok(())
}

#[test]
fn failed_mutation_rolls_back_and_does_not_consume_idempotency_key() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("rollback target"))?;
    let key = "rollback-update-key-0001";
    let failed = core.execute_remote_mutation(request(
        key,
        "request-rollback-failed",
        MutationExpectedVersion::Exact(task.version),
        MutationCommand::TaskUpdate {
            task_id: task.id.clone(),
            patch: TaskPatch {
                project_id: Some(Uuid::now_v7().to_string()),
                ..TaskPatch::default()
            },
        },
    ));
    assert!(matches!(failed, Err(Error::NotFound { .. })));
    assert_eq!(core.get_task(&task.id)?.version, 1);
    assert!(core.list_mutation_audit_events()?.is_empty());

    let recovered = core.execute_remote_mutation(request(
        key,
        "request-rollback-retry",
        MutationExpectedVersion::Exact(task.version),
        MutationCommand::TaskUpdate {
            task_id: task.id.clone(),
            patch: TaskPatch {
                title: Some("rollback recovered".to_owned()),
                ..TaskPatch::default()
            },
        },
    ))?;
    assert_eq!(recovered.version, 2);
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);
    Ok(())
}

#[test]
fn concurrent_duplicate_submission_changes_data_once() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("concurrent target"))?;
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let core = core.clone();
            let barrier = Arc::clone(&barrier);
            let task_id = task.id.clone();
            std::thread::spawn(move || {
                barrier.wait();
                core.execute_remote_mutation(request(
                    "concurrent-key-000001",
                    &format!("concurrent-request-{index}"),
                    MutationExpectedVersion::Exact(1),
                    MutationCommand::TaskUpdate {
                        task_id,
                        patch: TaskPatch {
                            title: Some("concurrent winner".to_owned()),
                            ..TaskPatch::default()
                        },
                    },
                ))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .map_err(|_| Error::Invariant("mutation thread panicked".to_owned()))?
        })
        .collect::<Result<Vec<_>>>()?;

    assert_eq!(results.iter().filter(|result| result.replayed).count(), 1);
    assert_eq!(results.iter().filter(|result| !result.replayed).count(), 1);
    assert_eq!(core.get_task(&task.id)?.version, 2);
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);
    Ok(())
}

#[test]
fn mutation_audit_and_idempotency_rows_are_immutable() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.execute_remote_mutation(request(
        "immutable-key-0000001",
        "immutable-request",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: task_input("immutable audit"),
        },
    ))?;
    let connection = Connection::open(core.home().database_path())?;
    assert!(
        connection
            .execute("UPDATE mutation_audit_events SET result = 'completed'", [],)
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM mutation_audit_events", [])
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM mutation_idempotency_records", [])
            .is_err()
    );
    Ok(())
}

#[test]
fn mutation_validation_limits_content_and_status_transitions() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let invalid_key = core.execute_remote_mutation(request(
        "short",
        "invalid-key-request",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: task_input("invalid key"),
        },
    ));
    assert!(matches!(invalid_key, Err(Error::InvalidInput(_))));

    let oversized_title = core.execute_remote_mutation(request(
        "validation-key-000001",
        "oversized-title-request",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: task_input(&"x".repeat(501)),
        },
    ));
    assert!(matches!(oversized_title, Err(Error::InvalidInput(_))));

    let done = core.create_task(CreateTaskInput {
        status: TaskStatus::Done,
        ..task_input("completed task")
    })?;
    let invalid_transition = core.execute_remote_mutation(request(
        "validation-key-000002",
        "invalid-transition-request",
        MutationExpectedVersion::Exact(done.version),
        MutationCommand::TaskUpdate {
            task_id: done.id.clone(),
            patch: TaskPatch {
                status: Some(TaskStatus::Blocked),
                ..TaskPatch::default()
            },
        },
    ));
    assert!(matches!(invalid_transition, Err(Error::InvalidInput(_))));
    assert_eq!(core.get_task(&done.id)?.status, TaskStatus::Done);

    let project = core.create_project(CreateProjectInput {
        name: "archived target".to_owned(),
        description: String::new(),
        color: None,
    })?;
    core.move_to_trash(TrashEntityType::Project, &project.id)?;
    let inactive_project = core.execute_remote_mutation(request(
        "validation-key-000003",
        "inactive-project-request",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: CreateTaskInput {
                project_id: Some(project.id),
                ..task_input("inactive project task")
            },
        },
    ));
    assert!(matches!(inactive_project, Err(Error::Conflict(_))));

    let checklist_owner = core.create_task(task_input("trashed checklist owner"))?;
    let checklist = core.add_checklist_item(&checklist_owner.id, "cannot change in trash")?;
    core.move_to_trash(TrashEntityType::Task, &checklist_owner.id)?;
    let trashed_checklist = core.execute_remote_mutation(request(
        "validation-key-000004",
        "trashed-checklist-request",
        MutationExpectedVersion::Exact(checklist.version),
        MutationCommand::ChecklistSetDone {
            item_id: checklist.id,
            is_done: true,
        },
    ));
    assert!(matches!(trashed_checklist, Err(Error::Conflict(_))));
    assert!(core.list_mutation_audit_events()?.is_empty());
    Ok(())
}

#[test]
fn restore_cannot_rewind_append_only_mutation_ledger() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let before_mutation = core.create_backup()?;
    let created = core.execute_remote_mutation(request(
        "restore-guard-key-0001",
        "restore-guard-request",
        MutationExpectedVersion::Absent,
        MutationCommand::TaskCreate {
            input: task_input("restore guard"),
        },
    ))?;

    let restore = core.restore_backup(&before_mutation.path);
    assert!(matches!(restore, Err(Error::Conflict(_))));
    assert!(core.get_task(&created.resource_id).is_ok());
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);

    let matching = core.create_backup()?;
    core.restore_backup(&matching.path)?;
    assert!(core.get_task(&created.resource_id).is_ok());
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);
    Ok(())
}
