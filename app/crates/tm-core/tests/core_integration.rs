use std::sync::{Arc, Barrier};

use chrono::{NaiveDate, Utc};
use chrono_tz::Asia::Seoul;
use rusqlite::Connection;
use tempfile::{Builder, TempDir};
use tm_core::{
    ChecklistMutationInput, CreateAttachmentInput, CreateLinkInput, CreateNoteAggregateInput,
    CreateNoteInput, CreateProjectInput, CreateTaskAggregateInput, CreateTaskInput,
    DEFAULT_TM_HOME, DigestKind, EndSessionInput, EntityType, Error, LinkTargetType,
    NoteLinksInput, NoteType, Result, StartSessionInput, TaskDayStatus, TaskPatch, TaskStatus,
    TmCore, TmHome, UpdateTaskAggregateInput,
};
use uuid::Uuid;

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new().prefix("tm-core-").tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
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

fn seoul_today() -> NaiveDate {
    Utc::now().with_timezone(&Seoul).date_naive()
}

#[test]
fn initializes_schema_with_uuid_v7_utc_and_wal() -> Result<()> {
    let (temporary, core) = fixture()?;
    let health = core.health()?;
    assert!(health.ok);
    assert_eq!(health.schema_version, 13);
    assert_eq!(health.journal_mode.to_ascii_lowercase(), "wal");
    assert!(
        std::path::Path::new(&health.database_path)
            .ends_with(std::path::Path::new("data").join("tm.sqlite3"))
    );

    let initialized_at = core.database_initialized_at()?;
    let reopened = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(reopened.database_initialized_at()?, initialized_at);

    let task = core.create_task(task_input("UUIDv7 확인"))?;
    let parsed = Uuid::parse_str(&task.id)
        .map_err(|error| Error::Invariant(format!("invalid generated UUID: {error}")))?;
    assert_eq!(parsed.get_version_num(), 7);
    assert!(task.created_at.ends_with('Z'));
    Ok(())
}

#[test]
fn task_mutations_emit_before_after_and_events_are_append_only() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("변경 전"))?;
    let updated = core.update_task(
        &task.id,
        TaskPatch {
            title: Some("변경 후".to_owned()),
            status: Some(TaskStatus::InProgress),
            priority: Some(3),
            ..TaskPatch::default()
        },
    )?;
    assert_eq!(updated.title, "변경 후");

    let item = core.add_checklist_item(&task.id, "체크 항목")?;
    core.update_checklist_item(&item.id, None, Some(true), None)?;
    core.set_task_tags(&task.id, &["중요".to_owned()])?;

    let events = core.list_task_events(&task.id)?;
    assert_eq!(events[0].event_type, "task.created");
    let changed = events
        .iter()
        .find(|event| event.event_type == "task.changed")
        .ok_or_else(|| Error::Invariant("task.changed event missing".to_owned()))?;
    assert_eq!(
        changed
            .before
            .as_ref()
            .and_then(|value| value["title"].as_str()),
        Some("변경 전")
    );
    assert_eq!(
        changed
            .after
            .as_ref()
            .and_then(|value| value["title"].as_str()),
        Some("변경 후")
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "checklist.changed")
    );
    assert!(events.iter().any(|event| event.event_type == "tag.added"));

    let raw = Connection::open(core.home().database_path())?;
    let update_result = raw.execute(
        "UPDATE task_events SET actor = 'tampered' WHERE id = ?1",
        [&events[0].id],
    );
    assert!(update_result.is_err());
    let delete_result = raw.execute("DELETE FROM task_events WHERE id = ?1", [&events[0].id]);
    assert!(delete_result.is_err());
    Ok(())
}

#[test]
fn finalized_day_entries_never_revert_or_change_identity() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("과거 기록"))?;
    let yesterday = seoul_today()
        .pred_opt()
        .ok_or_else(|| Error::Invariant("today has no predecessor".to_owned()))?;
    let planned = core.plan_task(&task.id, yesterday, None)?;
    let finalized = core.finalize_day_entry(&planned.id, TaskDayStatus::Skipped, Some("보류"))?;
    assert_eq!(finalized.status, TaskDayStatus::Skipped);
    assert!(core.plan_task(&task.id, yesterday, None).is_err());
    assert!(
        core.finalize_day_entry(&planned.id, TaskDayStatus::Done, None)
            .is_err()
    );

    let raw = Connection::open(core.home().database_path())?;
    assert!(
        raw.execute(
            "UPDATE task_day_entries SET status = 'planned', finalized_at = NULL WHERE id = ?1",
            [&planned.id],
        )
        .is_err()
    );
    assert!(
        raw.execute(
            "UPDATE task_day_entries SET entry_date = '2099-01-01' WHERE id = ?1",
            [&planned.id],
        )
        .is_err()
    );
    assert!(
        raw.execute("DELETE FROM task_day_entries WHERE id = ?1", [&planned.id])
            .is_err()
    );
    Ok(())
}

#[test]
fn carry_over_is_explicit_and_keeps_source_history() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("이월 대상"))?;
    let yesterday = seoul_today()
        .pred_opt()
        .ok_or_else(|| Error::Invariant("today has no predecessor".to_owned()))?;
    let original = core.plan_task(&task.id, yesterday, None)?;
    let (deferred, today) = core.carry_over_day_entry(&original.id, seoul_today())?;
    assert_eq!(deferred.entry_date, yesterday);
    assert_eq!(deferred.status, TaskDayStatus::Deferred);
    assert_eq!(today.entry_date, seoul_today());
    assert_eq!(today.status, TaskDayStatus::Planned);
    assert_ne!(deferred.id, today.id);
    Ok(())
}

#[test]
fn ending_session_worklog_and_followup_task_is_atomic() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let project = core.create_project(CreateProjectInput {
        name: "원자성 프로젝트".to_owned(),
        description: String::new(),
        color: None,
    })?;
    let mut task_data = task_input("연결 Task");
    task_data.project_id = Some(project.id.clone());
    let task = core.create_task(task_data)?;
    let session = core.start_session(StartSessionInput {
        project_id: Some(project.id),
        goal: "원자적 종료 검증".to_owned(),
        task_ids: vec![task.id],
    })?;

    let failed = core.end_session(
        &session.id,
        EndSessionInput {
            result: "작업함".to_owned(),
            blockers: String::new(),
            next_action: "다음 작업".to_owned(),
            create_followup_task: true,
            followup_title: None,
            followup_project_id: Some(Uuid::now_v7().to_string()),
        },
    );
    assert!(failed.is_err());
    assert_eq!(core.list_sessions(false)?[0].status.to_string(), "running");
    assert!(core.list_worklogs(false)?.is_empty());
    assert_eq!(core.list_tasks(false)?.len(), 1);

    let completed = core.end_session(
        &session.id,
        EndSessionInput {
            result: "완료 결과".to_owned(),
            blockers: "없음".to_owned(),
            next_action: "후속 Task".to_owned(),
            create_followup_task: true,
            followup_title: None,
            followup_project_id: None,
        },
    )?;
    assert_eq!(completed.session.status.to_string(), "completed");
    assert!(completed.followup_task.is_some());
    assert_eq!(core.list_worklogs(false)?.len(), 1);
    assert_eq!(core.list_tasks(false)?.len(), 2);
    Ok(())
}

#[test]
fn notes_can_link_multiple_entities_and_fts_covers_every_document_type() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let first = core.create_task(task_input("알파태스크 검색어"))?;
    let second = core.create_task(task_input("추가 Task"))?;
    let session = core.start_session(StartSessionInput {
        project_id: None,
        goal: "베타세션 검색어".to_owned(),
        task_ids: vec![first.id.clone()],
    })?;
    let completion = core.end_session(
        &session.id,
        EndSessionInput {
            result: "감마기록 검색어".to_owned(),
            blockers: String::new(),
            next_action: String::new(),
            create_followup_task: false,
            followup_title: None,
            followup_project_id: None,
        },
    )?;
    let note = core.create_note(CreateNoteInput {
        note_type: NoteType::Concept,
        title: "델타노트 검색어".to_owned(),
        body: "재사용 지식".to_owned(),
        note_date: None,
    })?;
    for task_id in [&first.id, &second.id] {
        core.create_link(CreateLinkInput {
            source_type: EntityType::Note,
            source_id: note.id.clone(),
            target_type: LinkTargetType::Task,
            target_id: Some(task_id.clone()),
            target_value: None,
            relation: "documents".to_owned(),
        })?;
    }
    core.create_link(CreateLinkInput {
        source_type: EntityType::Note,
        source_id: note.id.clone(),
        target_type: LinkTargetType::Session,
        target_id: Some(session.id.clone()),
        target_value: None,
        relation: "documents".to_owned(),
    })?;
    assert_eq!(
        core.list_links(Some(EntityType::Note), Some(&note.id))?
            .len(),
        3
    );

    let promoted =
        core.promote_worklog(&completion.worklog.id, NoteType::Howto, Some("승격 Note"))?;
    assert_eq!(
        promoted.source_worklog_id.as_deref(),
        Some(completion.worklog.id.as_str())
    );
    assert_eq!(core.list_worklogs(false)?.len(), 1);

    assert_eq!(
        core.search("알파태스크", 10)?[0].entity_type,
        EntityType::Task
    );
    assert_eq!(
        core.search("베타세션", 10)?[0].entity_type,
        EntityType::Session
    );
    assert_eq!(
        core.search("감마기록", 10)?[0].entity_type,
        EntityType::Session
    );
    assert_eq!(
        core.search("델타노트", 10)?[0].entity_type,
        EntityType::Note
    );
    assert!(
        core.search("세션 종료", 20)?
            .iter()
            .any(|hit| hit.entity_type == EntityType::Worklog)
    );
    Ok(())
}

#[test]
fn concurrent_digest_prepare_has_exactly_one_sender_and_sent_is_immutable() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let core = Arc::new(core);
    let barrier = Arc::new(Barrier::new(12));
    let date = seoul_today();
    let handles: Vec<_> = (0..12)
        .map(|_| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.prepare_digest(DigestKind::Morning, date)
            })
        })
        .collect();
    let mut preparations = Vec::new();
    for handle in handles {
        let result = handle
            .join()
            .map_err(|_| Error::Invariant("digest worker panicked".to_owned()))??;
        preparations.push(result);
    }
    assert_eq!(
        preparations
            .iter()
            .filter(|result| result.should_send)
            .count(),
        1
    );
    let key = preparations[0].delivery_key.clone();
    let sent = core.complete_digest(&key, "slack:123")?;
    assert_eq!(sent.status, "sent");
    assert!(!core.prepare_digest(DigestKind::Morning, date)?.should_send);

    let raw = Connection::open(core.home().database_path())?;
    assert!(
        raw.execute(
            "UPDATE digest_deliveries SET status = 'failed' WHERE delivery_key = ?1",
            [&key],
        )
        .is_err()
    );
    assert!(
        raw.execute(
            "DELETE FROM digest_deliveries WHERE delivery_key = ?1",
            [&key]
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn failed_digest_can_be_claimed_once_but_claimed_never_expires_automatically() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let date = seoul_today();
    let first = core.prepare_digest(DigestKind::Evening, date)?;
    assert!(first.should_send);
    assert!(!core.prepare_digest(DigestKind::Evening, date)?.should_send);
    core.fail_digest(&first.delivery_key, "명시적 실패")?;
    assert!(core.prepare_digest(DigestKind::Evening, date)?.should_send);
    assert!(!core.prepare_digest(DigestKind::Evening, date)?.should_send);
    Ok(())
}

#[test]
fn backup_restore_preserves_relations_history_and_delivery_ledger() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("백업 Task"))?;
    core.update_task(
        &task.id,
        TaskPatch {
            description: Some("백업 전 변경".to_owned()),
            ..TaskPatch::default()
        },
    )?;
    let note = core.create_note(CreateNoteInput {
        note_type: NoteType::Decision,
        title: "백업 Note".to_owned(),
        body: "결정 내용".to_owned(),
        note_date: None,
    })?;
    core.create_link(CreateLinkInput {
        source_type: EntityType::Note,
        source_id: note.id.clone(),
        target_type: LinkTargetType::Task,
        target_id: Some(task.id.clone()),
        target_value: None,
        relation: "documents".to_owned(),
    })?;
    let backup = core.create_backup()?;

    let sent_date = seoul_today();
    let sent = core.prepare_digest(DigestKind::Morning, sent_date)?;
    core.complete_digest(&sent.delivery_key, "slack:restore")?;
    let claimed_date = sent_date
        .succ_opt()
        .ok_or_else(|| Error::Invariant("today has no successor".to_owned()))?;
    let claimed = core.prepare_digest(DigestKind::Morning, claimed_date)?;
    assert!(claimed.should_send);
    core.create_task(task_input("복원 시 사라질 Task"))?;

    core.restore_backup(&backup.path)?;
    assert_eq!(core.list_tasks(false)?.len(), 1);
    assert_eq!(core.list_task_events(&task.id)?.len(), 2);
    assert_eq!(
        core.list_links(Some(EntityType::Note), Some(&note.id))?
            .len(),
        1
    );
    assert!(
        !core
            .prepare_digest(DigestKind::Morning, sent_date)?
            .should_send
    );
    assert!(
        !core
            .prepare_digest(DigestKind::Morning, claimed_date)?
            .should_send
    );
    let status = core.digest_status()?;
    assert!(status.iter().any(|delivery| delivery.status == "sent"));
    assert!(status.iter().any(|delivery| delivery.status == "claimed"));
    Ok(())
}

#[test]
fn trash_restore_and_full_exports_cover_all_logical_tables() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("휴지통 Task"))?;
    core.move_to_trash(tm_core::TrashEntityType::Task, &task.id)?;
    assert!(core.list_tasks(false)?.is_empty());
    assert_eq!(core.list_trash()?.len(), 1);
    assert!(core.search("휴지통", 10)?.is_empty());
    core.restore_from_trash(tm_core::TrashEntityType::Task, &task.id)?;
    assert_eq!(core.list_tasks(false)?.len(), 1);
    assert_eq!(core.search("휴지통", 10)?.len(), 1);
    core.register_attachment(CreateAttachmentInput {
        owner_type: EntityType::Task,
        owner_id: task.id.clone(),
        relative_path: "task/attachment.txt".to_owned(),
        original_name: "attachment.txt".to_owned(),
        media_type: Some("text/plain".to_owned()),
        byte_size: 12,
        sha256: "a".repeat(64),
    })?;
    assert_eq!(
        core.list_attachments(Some(EntityType::Task), Some(&task.id), false)?
            .len(),
        1
    );
    assert!(
        core.register_attachment(CreateAttachmentInput {
            owner_type: EntityType::Task,
            owner_id: task.id.clone(),
            relative_path: "../escape.txt".to_owned(),
            original_name: "escape.txt".to_owned(),
            media_type: None,
            byte_size: 0,
            sha256: "b".repeat(64),
        })
        .is_err()
    );

    let export = core.export_json()?;
    let tables = export["tables"]
        .as_object()
        .ok_or_else(|| Error::Invariant("export tables object missing".to_owned()))?;
    for required in [
        "projects",
        "tasks",
        "checklist_items",
        "tags",
        "task_tags",
        "task_day_entries",
        "task_events",
        "work_sessions",
        "session_tasks",
        "worklogs",
        "notes",
        "entity_links",
        "attachments",
        "digest_deliveries",
    ] {
        assert!(
            tables.contains_key(required),
            "missing export table {required}"
        );
    }
    let markdown = core.export_markdown()?;
    assert!(markdown.contains("`tasks`"));
    assert!(markdown.contains("휴지통 Task"));
    let files = core.create_export()?;
    assert!(std::path::Path::new(&files.json_path).is_file());
    assert!(std::path::Path::new(&files.markdown_path).is_file());
    Ok(())
}

#[test]
fn independent_core_instances_can_write_concurrently() -> Result<()> {
    let (temporary, first) = fixture()?;
    let second = TmCore::open(TmHome::new(temporary.path()))?;
    let first = Arc::new(first);
    let second = Arc::new(second);
    let barrier = Arc::new(Barrier::new(20));
    let handles: Vec<_> = (0..20)
        .map(|index| {
            let core = if index % 2 == 0 {
                Arc::clone(&first)
            } else {
                Arc::clone(&second)
            };
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.create_task(task_input(&format!("동시 Task {index}")))
            })
        })
        .collect();
    for handle in handles {
        handle
            .join()
            .map_err(|_| Error::Invariant("database worker panicked".to_owned()))??;
    }
    assert_eq!(first.list_tasks(false)?.len(), 20);
    Ok(())
}

#[test]
fn backup_lifecycle_and_retention_limits_are_enforced() -> Result<()> {
    let (_temporary, core) = fixture()?;
    core.create_task(task_input("백업 수명주기"))?;
    let startup = core.create_startup_backup()?;
    let shutdown = core.create_shutdown_backup()?;
    assert_ne!(startup.path, shutdown.path);
    assert!(core.create_daily_backup_if_due()?.is_some());
    assert!(core.create_daily_backup_if_due()?.is_none());
    for _ in 0..31 {
        core.create_backup()?;
    }
    let database_backups = core.list_backups()?;
    assert_eq!(database_backups.len(), 30);

    std::fs::write(core.home().app_dir().join("snapshot-source.txt"), "source")?;
    for _ in 0..11 {
        core.create_source_snapshot()?;
    }
    let source_count = std::fs::read_dir(core.home().source_backups_dir())?
        .filter_map(std::result::Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "zip"))
        .count();
    assert_eq!(source_count, 10);
    Ok(())
}

#[test]
fn create_task_aggregate_rolls_back_task_when_tag_fails() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let result = core.create_task_aggregate(CreateTaskAggregateInput {
        task: task_input("rollback create"),
        tags: vec!["valid-tag".to_owned(), "   ".to_owned()],
    });
    assert!(result.is_err());
    assert!(core.list_tasks(true)?.is_empty());
    assert!(core.list_tags(None)?.is_empty());
    let exported = core.export_json()?;
    assert_eq!(
        exported["tables"]["task_events"].as_array().map(Vec::len),
        Some(0)
    );
    Ok(())
}

#[test]
fn update_task_aggregate_rejects_foreign_checklist_and_rolls_everything_back() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let first = core.create_task_aggregate(CreateTaskAggregateInput {
        task: task_input("first original"),
        tags: vec!["original-tag".to_owned()],
    })?;
    let first_item = core.add_checklist_item(&first.task.id, "first checklist")?;
    let second = core.create_task(task_input("second"))?;
    let second_item = core.add_checklist_item(&second.id, "second checklist")?;
    let event_count = core.list_task_events(&first.task.id)?.len();

    let result = core.update_task_aggregate(UpdateTaskAggregateInput {
        task_id: first.task.id.clone(),
        patch: TaskPatch {
            title: Some("must roll back".to_owned()),
            status: Some(TaskStatus::InProgress),
            ..TaskPatch::default()
        },
        tags: vec!["replacement-tag".to_owned()],
        checklist: vec![ChecklistMutationInput {
            id: Some(second_item.id.clone()),
            body: "foreign mutation".to_owned(),
            is_done: true,
        }],
    });
    assert!(result.is_err());

    let unchanged = core.get_task(&first.task.id)?;
    assert_eq!(unchanged.title, "first original");
    assert_eq!(unchanged.status, TaskStatus::Todo);
    assert_eq!(
        core.list_tags(Some(&first.task.id))?
            .into_iter()
            .map(|tag| tag.name)
            .collect::<Vec<_>>(),
        vec!["original-tag"]
    );
    assert_eq!(core.list_checklist_items(&first.task.id)?, vec![first_item]);
    assert_eq!(core.list_checklist_items(&second.id)?, vec![second_item]);
    assert_eq!(core.list_task_events(&first.task.id)?.len(), event_count);
    Ok(())
}

#[test]
fn create_note_aggregate_rolls_back_note_and_prior_links_on_any_link_error() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let task = core.create_task(task_input("note target"))?;
    let note = || CreateNoteInput {
        note_type: NoteType::Reference,
        title: "atomic note".to_owned(),
        body: "body".to_owned(),
        note_date: None,
    };

    let missing_target = core.create_note_aggregate(CreateNoteAggregateInput {
        note: note(),
        links: NoteLinksInput {
            task_ids: vec![task.id.clone()],
            session_ids: vec![Uuid::now_v7().to_string()],
            ..NoteLinksInput::default()
        },
    });
    assert!(missing_target.is_err());
    assert!(core.list_notes(true)?.is_empty());
    assert!(core.list_links(None, None)?.is_empty());

    let invalid_url = core.create_note_aggregate(CreateNoteAggregateInput {
        note: note(),
        links: NoteLinksInput {
            task_ids: vec![task.id.clone()],
            urls: vec!["not a URL".to_owned()],
            ..NoteLinksInput::default()
        },
    });
    assert!(invalid_url.is_err());
    assert!(core.list_notes(true)?.is_empty());
    assert!(core.list_links(None, None)?.is_empty());

    let created = core.create_note_aggregate(CreateNoteAggregateInput {
        note: note(),
        links: NoteLinksInput {
            task_ids: vec![task.id],
            file_paths: vec![r"C:\reference\design.md".to_owned()],
            urls: vec!["https://example.com/reference".to_owned()],
            ..NoteLinksInput::default()
        },
    })?;
    assert_eq!(created.links.len(), 3);
    assert_eq!(core.list_notes(false)?.len(), 1);
    Ok(())
}
