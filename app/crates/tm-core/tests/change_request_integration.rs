use std::sync::{Arc, Barrier};

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempDir};
use tm_core::{
    ChangeRequestKind, ChangeRequestStatus, CreateChangeRequestInput, CreateProjectInput,
    CreateTaskInput, DEFAULT_TM_HOME, Error, Result, TaskStatus, TmCore, TmHome,
    UpdateChangeRequestInput,
};
use uuid::Uuid;

fn fixture() -> Result<(TempDir, TmCore)> {
    let temporary = temporary_root("tm-change-request-")?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn temporary_root(prefix: &str) -> Result<TempDir> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    Builder::new()
        .prefix(prefix)
        .tempdir_in(test_runs)
        .map_err(Into::into)
}

fn create_input(title: &str, priority: u8) -> CreateChangeRequestInput {
    CreateChangeRequestInput {
        kind: ChangeRequestKind::Feature,
        project_id: None,
        task_id: None,
        title: title.to_owned(),
        description: format!("{title} 설명"),
        desired_outcome: format!("{title} 기대 결과"),
        reproduction_steps: String::new(),
        priority,
        requested_by: "tester".to_owned(),
    }
}

fn update_input(
    expected_revision: u32,
    expected_attempt_count: u32,
    title: &str,
) -> UpdateChangeRequestInput {
    UpdateChangeRequestInput {
        expected_revision,
        expected_attempt_count,
        kind: ChangeRequestKind::Ui,
        project_id: None,
        task_id: None,
        title: title.to_owned(),
        description: format!("{title} 수정 설명"),
        desired_outcome: format!("{title} 수정 결과"),
        reproduction_steps: "선택 재현 절차".to_owned(),
        priority: 3,
        updated_by: "editor".to_owned(),
    }
}

fn claim_key(claim: &tm_core::ChangeRequestClaim) -> Result<&str> {
    claim
        .claim_key
        .as_deref()
        .ok_or_else(|| Error::Invariant("claim key missing".to_owned()))
}

fn task_input(project_id: Option<String>, title: &str) -> CreateTaskInput {
    CreateTaskInput {
        project_id,
        title: title.to_owned(),
        description: String::new(),
        status: TaskStatus::Todo,
        priority: 0,
        due_date: None,
    }
}

fn file_sha256(path: &std::path::Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[test]
fn current_schema_exports_queue_and_validates_required_content() -> Result<()> {
    let (_temporary, core) = fixture()?;
    assert_eq!(core.health()?.schema_version, 15);

    let mut invalid = create_input("필수 검증", 1);
    invalid.description.clear();
    assert!(core.create_change_request(invalid).is_err());
    let mut invalid = create_input("필수 검증", 1);
    invalid.desired_outcome.clear();
    assert!(core.create_change_request(invalid).is_err());
    let mut valid = create_input("재현 절차 선택", 1);
    valid.reproduction_steps.clear();
    assert_eq!(
        core.create_change_request(valid)?.status,
        ChangeRequestStatus::Draft
    );

    let export = core.export_json()?;
    assert!(export["tables"]["change_requests"].is_array());
    assert!(export["tables"]["change_request_events"].is_array());
    Ok(())
}

#[test]
fn opening_v1_database_creates_pre_migration_backup_and_applies_all_migrations() -> Result<()> {
    let temporary = temporary_root("tm-change-request-v1-")?;
    let data = temporary.path().join("data");
    std::fs::create_dir_all(&data)?;
    let connection = Connection::open(data.join("tm.sqlite3"))?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    connection.execute(
        "INSERT INTO schema_migrations(version, name, applied_at)
         VALUES (1, 'local-first-foundation', '2026-01-01T00:00:00.000Z')",
        [],
    )?;
    connection.pragma_update(None, "user_version", 1_i64)?;
    drop(connection);

    let core = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(core.health()?.schema_version, 15);
    assert!(
        core.list_backups()?
            .iter()
            .any(|backup| backup.trigger == "pre_migration")
    );
    Ok(())
}

#[test]
fn opening_v2_database_creates_pre_migration_backup_and_applies_current_schema() -> Result<()> {
    let temporary = temporary_root("tm-change-request-v2-")?;
    let data = temporary.path().join("data");
    std::fs::create_dir_all(&data)?;
    let connection = Connection::open(data.join("tm.sqlite3"))?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    connection.execute(
        "INSERT INTO schema_migrations(version, name, applied_at)
         VALUES (1, 'local-first-foundation', '2026-01-01T00:00:00.000Z')",
        [],
    )?;
    connection.execute_batch(include_str!("../migrations/0002_change_requests.sql"))?;
    connection.execute(
        "INSERT INTO schema_migrations(version, name, applied_at)
         VALUES (2, 'manual-change-request-queue', '2026-01-02T00:00:00.000Z')",
        [],
    )?;
    connection.pragma_update(None, "user_version", 2_i64)?;
    drop(connection);

    let core = TmCore::open(TmHome::new(temporary.path()))?;
    assert_eq!(core.health()?.schema_version, 15);
    let backups = core.list_backups()?;
    let pre_migration = backups
        .iter()
        .find(|backup| backup.trigger == "pre_migration")
        .ok_or_else(|| Error::Invariant("schema 2 pre-migration backup missing".to_owned()))?;
    let preserved = Connection::open(&pre_migration.path)?;
    let preserved_version: i64 =
        preserved.pragma_query_value(None, "user_version", |row| row.get(0))?;
    assert_eq!(preserved_version, 2);

    let request = core.create_change_request(create_input("schema 3 strict CAS", 1))?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let returned = core.return_change_request_to_draft(&request.id, 1, 0, "reviewer")?;
    assert_eq!(returned.revision, 2);
    let created = &core.list_change_request_events(&request.id)?[0];
    assert_eq!(
        created.details["snapshot"]["description"],
        request.description
    );
    Ok(())
}

#[test]
fn draft_revision_and_manual_approval_are_optimistic_and_explicit() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let draft = core.create_change_request(create_input("승인 요청", 2))?;
    assert_eq!(draft.revision, 1);
    assert!(!core.claim_next_change_request("worker")?.should_process);
    assert!(
        core.update_change_request(&draft.id, update_input(2, 0, "stale"))
            .is_err()
    );

    let updated = core.update_change_request(&draft.id, update_input(1, 0, "수정된 요청"))?;
    assert_eq!(updated.revision, 2);
    let events = core.list_change_request_events(&draft.id)?;
    let created = events
        .iter()
        .find(|event| event.event_type == "created")
        .ok_or_else(|| Error::Invariant("created event missing".to_owned()))?;
    assert_eq!(created.details["snapshot"]["title"], "승인 요청");
    assert_eq!(created.details["snapshot"]["description"], "승인 요청 설명");
    let edited = events
        .iter()
        .find(|event| event.event_type == "draft_updated")
        .ok_or_else(|| Error::Invariant("draft_updated event missing".to_owned()))?;
    assert_eq!(edited.details["before"]["title"], "승인 요청");
    assert_eq!(edited.details["after"]["title"], "수정된 요청");
    assert_eq!(
        edited.details["after"]["desiredOutcome"],
        "수정된 요청 수정 결과"
    );
    assert!(
        core.approve_change_request(&draft.id, 1, 0, "reviewer")
            .is_err()
    );
    let approved = core.approve_change_request(&draft.id, 2, 0, "reviewer")?;
    assert_eq!(approved.revision, 2);
    assert_eq!(approved.approved_revision, Some(2));

    let returned = core.return_change_request_to_draft(&draft.id, 2, 0, "reviewer")?;
    assert_eq!(returned.status, ChangeRequestStatus::Draft);
    assert_eq!(returned.revision, 3);
    assert!(
        core.approve_change_request(&draft.id, 2, 0, "stale-reviewer")
            .is_err()
    );
    let revised = core.update_change_request(&draft.id, update_input(3, 0, "재수정 요청"))?;
    assert_eq!(revised.revision, 4);
    assert_eq!(
        core.approve_change_request(&draft.id, 4, 0, "reviewer")?
            .approved_revision,
        Some(4)
    );
    Ok(())
}

#[test]
fn concurrent_claim_has_one_winner_and_terminal_rows_are_immutable() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = core.create_change_request(create_input("동시 claim", 3))?;
    core.approve_change_request(&request.id, request.revision, 0, "reviewer")?;
    let core = Arc::new(core);
    let barrier = Arc::new(Barrier::new(12));
    let handles: Vec<_> = (0..12)
        .map(|index| {
            let core = Arc::clone(&core);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.claim_next_change_request(&format!("worker-{index}"))
            })
        })
        .collect();
    let mut claims = Vec::new();
    for handle in handles {
        claims.push(
            handle
                .join()
                .map_err(|_| Error::Invariant("claim worker panicked".to_owned()))??,
        );
    }
    assert_eq!(
        claims.iter().filter(|claim| claim.should_process).count(),
        1
    );
    let winner = claims
        .iter()
        .find(|claim| claim.should_process)
        .ok_or_else(|| Error::Invariant("claim winner missing".to_owned()))?;
    let claimed = winner
        .request
        .as_ref()
        .ok_or_else(|| Error::Invariant("claimed request missing".to_owned()))?;
    let key = claim_key(winner)?;
    Uuid::parse_str(key)
        .map_err(|error| Error::Invariant(format!("claim key is not UUID: {error}")))?;
    assert!(
        !serde_json::to_value(claimed)?
            .as_object()
            .is_some_and(|object| object.contains_key("claimKey"))
    );
    let worker = claimed
        .claimed_by
        .as_deref()
        .ok_or_else(|| Error::Invariant("claimed worker missing".to_owned()))?;
    assert!(
        core.complete_change_request(&request.id, "wrong-key", "결과", None, worker)
            .is_err()
    );
    assert!(
        core.complete_change_request(&request.id, key, "결과", None, "wrong-worker")
            .is_err()
    );
    let completed =
        core.complete_change_request(&request.id, key, "완료 결과", Some("patch-0002"), worker)?;
    assert_eq!(completed.status, ChangeRequestStatus::Completed);
    assert_eq!(completed.patch_ref.as_deref(), Some("patch-0002"));

    let raw = Connection::open(core.home().database_path())?;
    assert!(
        raw.execute(
            "UPDATE change_requests SET title = 'tampered' WHERE id = ?1",
            [&request.id],
        )
        .is_err()
    );
    assert!(
        raw.execute("DELETE FROM change_requests WHERE id = ?1", [&request.id])
            .is_err()
    );
    let event = core.list_change_request_events(&request.id)?[0].id.clone();
    assert!(
        raw.execute(
            "UPDATE change_request_events SET actor = 'tampered' WHERE id = ?1",
            [&event],
        )
        .is_err()
    );
    assert!(
        raw.execute("DELETE FROM change_request_events WHERE id = ?1", [&event])
            .is_err()
    );
    Ok(())
}

#[test]
fn failure_reapproval_return_to_draft_and_abandon_are_safe() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = core.create_change_request(create_input("실패 재시도", 2))?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let first_claim = core.claim_next_change_request("worker")?;
    let first_key = claim_key(&first_claim)?.to_owned();
    let failed = core.fail_change_request(&request.id, &first_key, "실패 원인", "worker")?;
    assert_eq!(failed.status, ChangeRequestStatus::Failed);
    assert!(
        core.update_change_request(&request.id, update_input(1, 1, "직접 수정 금지"))
            .is_err()
    );

    core.approve_change_request(&request.id, 1, 1, "reviewer")?;
    let second_claim = core.claim_next_change_request("worker")?;
    let second_key = claim_key(&second_claim)?.to_owned();
    assert_ne!(first_key, second_key);
    assert!(
        core.complete_change_request(&request.id, &first_key, "stale", None, "worker")
            .is_err()
    );
    assert!(
        core.abandon_change_request(&request.id, 1, 1, "stale abandon", "operator")
            .is_err()
    );
    let abandoned =
        core.abandon_change_request(&request.id, 1, 2, "worker 응답 유실", "operator")?;
    assert_eq!(abandoned.status, ChangeRequestStatus::Failed);
    assert!(
        core.list_change_request_events(&request.id)?
            .iter()
            .any(|event| event.event_type == "abandoned")
    );

    let returned = core.return_change_request_to_draft(&request.id, 1, 2, "reviewer")?;
    assert_eq!(returned.status, ChangeRequestStatus::Draft);
    assert_eq!(returned.revision, 2);
    let updated = core.update_change_request(&request.id, update_input(2, 2, "실패 후 수정"))?;
    assert_eq!(updated.revision, 3);
    core.approve_change_request(&request.id, 3, 2, "reviewer")?;
    let third_claim = core.claim_next_change_request("worker-2")?;
    core.complete_change_request(
        &request.id,
        claim_key(&third_claim)?,
        "재시도 완료",
        None,
        "worker-2",
    )?;
    Ok(())
}

#[test]
fn priority_order_and_cancellation_contract_are_enforced() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let low = core.create_change_request(create_input("낮음", 0))?;
    core.approve_change_request(&low.id, 1, 0, "reviewer")?;
    let high = core.create_change_request(create_input("높음", 3))?;
    core.approve_change_request(&high.id, 1, 0, "reviewer")?;
    assert_eq!(
        core.claim_next_change_request("worker")?
            .request
            .map(|request| request.id),
        Some(high.id)
    );

    let draft = core.create_change_request(create_input("취소", 1))?;
    let cancelled = core.cancel_change_request(&draft.id, 1, 0, "요청 철회", "reviewer")?;
    assert_eq!(cancelled.status, ChangeRequestStatus::Cancelled);
    assert!(
        core.approve_change_request(&draft.id, 1, 0, "reviewer")
            .is_err()
    );
    Ok(())
}

#[test]
fn restoring_v1_backup_migrates_and_preserves_non_rewindable_ledger() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let backup = core.create_backup()?;
    let backup_path = std::path::Path::new(&backup.path);
    let v1 = Connection::open(backup_path)?;
    v1.execute_batch(
        "DROP TRIGGER tasks_project_required_insert;
         DROP TRIGGER tasks_project_required_update;
         DROP TRIGGER projects_uncategorized_protect_update;
         DROP TRIGGER projects_uncategorized_protect_delete;
         DROP TRIGGER projects_uncategorized_name_reserved_insert;
         DROP TRIGGER projects_uncategorized_name_reserved_update;
         DROP INDEX idx_projects_system_key;
         DELETE FROM projects WHERE system_key = 'uncategorized';
         ALTER TABLE projects DROP COLUMN system_key;
         DROP TRIGGER stock_ai_reports_no_delete;
         DROP TRIGGER stock_ai_reports_identity_immutable;
         DROP TRIGGER stock_screen_results_no_delete;
         DROP TRIGGER stock_screen_results_no_update;
         DROP TRIGGER stock_screen_runs_no_delete;
         DROP TRIGGER stock_screen_runs_identity_immutable;
         DROP TRIGGER stock_universe_members_no_delete;
         DROP TRIGGER stock_universe_members_no_update;
         DROP TRIGGER stock_universe_snapshots_no_delete;
         DROP TRIGGER stock_universe_snapshots_no_update;
         DROP TRIGGER stock_market_data_batches_no_delete;
         DROP TRIGGER stock_market_data_batches_no_update;
         DROP TABLE stock_ai_reports;
         DROP TABLE stock_screen_results;
         DROP TABLE stock_screen_runs;
         DROP TABLE stock_daily_bars;
         DROP TABLE stock_market_sessions;
         DROP TABLE stock_market_data_batches;
         DROP TABLE stock_universe_members;
         DROP TABLE stock_universe_snapshots;
         DROP TABLE stock_watchlist_items;
         DROP TABLE calendar_events;
         DROP TABLE task_report_feedback;
         DROP TABLE task_report_runs;
         DROP TABLE device_auth_events;
         DROP TABLE registered_devices;
         DROP TABLE device_pairings;
         DROP TABLE scheduler_effects;
         DROP TABLE scheduler_attempts;
         DROP TABLE scheduler_runs;
         DROP TABLE scheduler_jobs;
         DROP TABLE assistant_memory_search;
         DROP TABLE assistant_memory_events;
         DROP TABLE assistant_memory_sources;
         DROP TABLE assistant_memories;
         DROP TABLE assistant_action_events;
         DROP TABLE assistant_action_requests;
         DROP TABLE ai_budget_ledger;
         DROP TABLE mutation_audit_events;
         DROP TABLE mutation_idempotency_records;
         ALTER TABLE tasks DROP COLUMN version;
         ALTER TABLE checklist_items DROP COLUMN version;
         ALTER TABLE notes DROP COLUMN version;
         DROP TABLE change_request_events;
         DROP TABLE change_requests;
         DELETE FROM schema_migrations WHERE version >= 2;
         PRAGMA user_version = 1;",
    )?;
    drop(v1);

    let claimed = core.create_change_request(create_input("보존 claimed", 3))?;
    core.approve_change_request(&claimed.id, 1, 0, "reviewer")?;
    let claim = core.claim_next_change_request("worker-claimed")?;
    assert!(claim.should_process);

    let completed = core.create_change_request(create_input("보존 completed", 2))?;
    core.approve_change_request(&completed.id, 1, 0, "reviewer")?;
    let claim = core.claim_next_change_request("worker-completed")?;
    core.complete_change_request(
        &completed.id,
        claim_key(&claim)?,
        "완료",
        None,
        "worker-completed",
    )?;

    let failed = core.create_change_request(create_input("보존 failed", 1))?;
    core.approve_change_request(&failed.id, 1, 0, "reviewer")?;
    let claim = core.claim_next_change_request("worker-failed")?;
    core.fail_change_request(&failed.id, claim_key(&claim)?, "실패", "worker-failed")?;

    let cancelled = core.create_change_request(create_input("보존 cancelled", 0))?;
    core.cancel_change_request(&cancelled.id, 1, 0, "취소", "reviewer")?;
    let protected_ids = [claimed.id, completed.id, failed.id, cancelled.id];

    core.restore_backup(backup_path)?;
    assert_eq!(core.health()?.schema_version, 15);
    let restored = core.list_change_requests()?;
    for id in &protected_ids {
        assert!(restored.iter().any(|request| &request.id == id));
        assert!(!core.list_change_request_events(id)?.is_empty());
    }
    assert!(
        !core
            .claim_next_change_request("another-worker")?
            .should_process
    );
    Ok(())
}

#[test]
fn restoring_approved_snapshot_cannot_rewind_same_request_after_completion() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = core.create_change_request(create_input("완료 보존", 2))?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let old_approved_backup = core.create_backup()?;

    let claim = core.claim_next_change_request("worker")?;
    core.complete_change_request(
        &request.id,
        claim_key(&claim)?,
        "완료 결과",
        Some("patch-0003"),
        "worker",
    )?;
    let expected_events = core.list_change_request_events(&request.id)?;

    core.restore_backup(&old_approved_backup.path)?;
    let restored = core
        .list_change_requests()?
        .into_iter()
        .find(|candidate| candidate.id == request.id)
        .ok_or_else(|| Error::Invariant("completed request missing after restore".to_owned()))?;
    assert_eq!(restored.status, ChangeRequestStatus::Completed);
    assert_eq!(restored.attempt_count, 1);
    assert_eq!(restored.patch_ref.as_deref(), Some("patch-0003"));
    assert_eq!(
        core.list_change_request_events(&request.id)?,
        expected_events
    );
    assert!(
        !core
            .claim_next_change_request("other-worker")?
            .should_process
    );
    Ok(())
}

#[test]
fn restoring_approved_snapshot_cannot_revive_withdrawn_approval() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = core.create_change_request(create_input("승인 철회 보존", 2))?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let approved_backup = core.create_backup()?;

    let withdrawn = core.return_change_request_to_draft(&request.id, 1, 0, "reviewer")?;
    assert_eq!(withdrawn.status, ChangeRequestStatus::Draft);
    assert_eq!(withdrawn.revision, 2);
    assert_eq!(withdrawn.attempt_count, 0);
    let expected_events = core.list_change_request_events(&request.id)?;

    core.restore_backup(&approved_backup.path)?;
    let restored = core
        .list_change_requests()?
        .into_iter()
        .find(|candidate| candidate.id == request.id)
        .ok_or_else(|| {
            Error::Invariant("withdrawn approval disappeared after restore".to_owned())
        })?;
    assert_eq!(restored, withdrawn);
    assert_eq!(
        core.list_change_request_events(&request.id)?,
        expected_events
    );
    assert!(!core.claim_next_change_request("worker")?.should_process);
    Ok(())
}

#[test]
fn restore_preserves_failed_draft_edit_and_reapproval_attempt_history() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let request = core.create_change_request(create_input("재승인 보존", 2))?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let old_approved_backup = core.create_backup()?;

    let first_claim = core.claim_next_change_request("worker-1")?;
    core.fail_change_request(
        &request.id,
        claim_key(&first_claim)?,
        "첫 실행 실패",
        "worker-1",
    )?;
    let draft = core.return_change_request_to_draft(&request.id, 1, 1, "reviewer")?;
    assert_eq!(draft.revision, 2);
    let edited = core.update_change_request(&request.id, update_input(2, 1, "수정 후 재승인"))?;
    assert_eq!(edited.revision, 3);
    let expected = core.approve_change_request(&request.id, 3, 1, "reviewer")?;
    let expected_events = core.list_change_request_events(&request.id)?;

    core.restore_backup(&old_approved_backup.path)?;
    let restored = core
        .list_change_requests()?
        .into_iter()
        .find(|candidate| candidate.id == request.id)
        .ok_or_else(|| Error::Invariant("reapproved request missing after restore".to_owned()))?;
    assert_eq!(restored, expected);
    assert_eq!(
        core.list_change_request_events(&request.id)?,
        expected_events
    );

    let second_claim = core.claim_next_change_request("worker-2")?;
    let claimed = second_claim
        .request
        .as_ref()
        .ok_or_else(|| Error::Invariant("reapproved request was not claimable".to_owned()))?;
    assert_eq!(claimed.id, request.id);
    assert_eq!(claimed.attempt_count, 2);
    Ok(())
}

#[test]
fn restore_preflight_rejects_missing_references_before_active_database_copy() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let backup_before_references = core.create_backup()?;

    let project = core.create_project(CreateProjectInput {
        name: "복원 참조 프로젝트".to_owned(),
        description: String::new(),
        color: None,
    })?;
    let task = core.create_task(task_input(Some(project.id.clone()), "복원 참조 Task"))?;
    let mut input = create_input("참조 보존", 3);
    input.project_id = Some(project.id.clone());
    input.task_id = Some(task.id.clone());
    let request = core.create_change_request(input)?;
    core.approve_change_request(&request.id, 1, 0, "reviewer")?;
    let claim = core.claim_next_change_request("worker")?;
    core.complete_change_request(
        &request.id,
        claim_key(&claim)?,
        "참조 포함 완료",
        None,
        "worker",
    )?;
    let expected_events = core.list_change_request_events(&request.id)?;

    let database_path = core.home().database_path();
    let checkpoint = Connection::open(&database_path)?;
    checkpoint.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    drop(checkpoint);
    let before_hash = file_sha256(&database_path)?;

    let result = core.restore_backup(&backup_before_references.path);
    assert!(matches!(result, Err(Error::Conflict(_))));
    assert_eq!(file_sha256(&database_path)?, before_hash);
    assert!(
        core.list_projects(false)?
            .iter()
            .any(|candidate| candidate.id == project.id)
    );
    assert!(
        core.list_tasks(false)?
            .iter()
            .any(|candidate| candidate.id == task.id)
    );
    let preserved = core
        .list_change_requests()?
        .into_iter()
        .find(|candidate| candidate.id == request.id)
        .ok_or_else(|| Error::Invariant("protected request disappeared".to_owned()))?;
    assert_eq!(preserved.status, ChangeRequestStatus::Completed);
    assert_eq!(
        core.list_change_request_events(&request.id)?,
        expected_events
    );
    Ok(())
}
