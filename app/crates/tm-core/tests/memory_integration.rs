use chrono::{Duration, Utc};
use tempfile::{Builder, TempDir};
use tm_core::{
    ASSISTANT_ACTION_APPROVAL_TTL_SECONDS, CreateMemoryInput, DEFAULT_TM_HOME, Error, MemoryKind,
    MemoryPatch, MemoryRetention, MemorySearchFilter, MemorySensitivity, MutationApprovalPolicy,
    MutationCommand, MutationExpectedVersion, MutationRequest, Result, TmCore, TmHome,
};

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new().prefix("tm-memory-").tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn memory_input(title: &str, body: &str, openai_allowed: bool) -> CreateMemoryInput {
    CreateMemoryInput {
        kind: MemoryKind::Preference,
        title: title.to_owned(),
        body: body.to_owned(),
        sensitivity: if openai_allowed {
            MemorySensitivity::Normal
        } else {
            MemorySensitivity::Private
        },
        openai_allowed,
        retention: MemoryRetention::UntilDeleted,
    }
}

fn approve(core: &TmCore, action: &tm_core::AssistantActionRequest) -> Result<()> {
    let execution = core.approve_and_execute_assistant_action(
        &action.id,
        action.revision,
        &action.payload_sha256,
        &format!("memory-approval:{}", action.id),
        "memory-test-approval",
    )?;
    assert_eq!(
        execution.action.status,
        tm_core::AssistantActionStatus::Completed
    );
    Ok(())
}

fn create_direct(core: &TmCore, index: usize, input: CreateMemoryInput) -> Result<String> {
    let result = core.execute_remote_mutation(MutationRequest {
        idempotency_key: format!("memory-scale-create-{index:04}"),
        expected_version: MutationExpectedVersion::Absent,
        actor: "single_user".to_owned(),
        request_id: format!("memory-scale-{index:04}"),
        approval_policy: MutationApprovalPolicy::ExplicitUserConfirmation,
        command: MutationCommand::MemoryCreate { input },
    })?;
    Ok(result.resource_id)
}

#[test]
fn explicit_memory_is_locked_until_approval_and_private_memory_is_not_retrieved() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let normal = core.propose_memory_create_action(
        memory_input("집중 시간", "오전에는 긴 집중 작업을 선호한다", true),
        "memory-create-request",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    assert!(core.list_assistant_memories(false)?.is_empty());
    approve(&core, &normal)?;

    let private = core.propose_memory_create_action(
        memory_input("개인 참고", "이 내용은 TM 안에서만 사용한다", false),
        "private-memory-request",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    approve(&core, &private)?;

    let local = core.search_assistant_memories(MemorySearchFilter {
        query: "내용".to_owned(),
        kind: None,
        openai_only: false,
        max_items: 12,
        max_bytes: 6 * 1024,
    })?;
    assert_eq!(local.items.len(), 1);
    assert_eq!(local.items[0].sensitivity, MemorySensitivity::Private);

    let openai = core.search_assistant_memories(MemorySearchFilter {
        query: "집중 내용".to_owned(),
        kind: None,
        openai_only: true,
        max_items: 12,
        max_bytes: 6 * 1024,
    })?;
    assert_eq!(openai.items.len(), 1);
    assert_eq!(openai.items[0].title, "집중 시간");
    assert!(openai.items[0].openai_allowed);
    assert!(!openai.vector_service_used);
    Ok(())
}

#[test]
fn memory_update_and_delete_use_revision_cas_and_append_audit_events() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let create = core.propose_memory_create_action(
        memory_input("운동", "주 3회 운동", true),
        "memory-cas-create",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    approve(&core, &create)?;
    let memory = core.list_assistant_memories(false)?.remove(0);

    let update = core.propose_memory_update_action(
        &memory.id,
        memory.revision,
        MemoryPatch {
            kind: MemoryKind::Routine,
            title: "운동 루틴".to_owned(),
            body: "주 4회 운동".to_owned(),
            sensitivity: MemorySensitivity::Normal,
            openai_allowed: true,
            retention: MemoryRetention::UntilDeleted,
        },
        "memory-cas-update",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    approve(&core, &update)?;
    let updated = core.get_assistant_memory(&memory.id)?;
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.body, "주 4회 운동");

    let stale = core.propose_memory_delete_action(
        &memory.id,
        1,
        "memory-stale-delete",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    );
    assert!(matches!(stale, Err(Error::Conflict(_))));

    let delete = core.propose_memory_delete_action(
        &memory.id,
        updated.revision,
        "memory-delete",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    approve(&core, &delete)?;
    assert!(core.list_assistant_memories(false)?.is_empty());
    let deleted = core.get_assistant_memory(&memory.id)?;
    assert!(deleted.deleted_at.is_some());
    assert_eq!(deleted.revision, 3);
    let events = core.list_assistant_memory_events(&memory.id)?;
    assert_eq!(
        events
            .iter()
            .map(|event| event.event_type.as_str())
            .collect::<Vec<_>>(),
        vec!["created", "updated", "deleted"]
    );
    Ok(())
}

#[test]
fn prohibited_secret_material_is_rejected_before_an_approval_is_created() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let result = core.propose_memory_create_action(
        memory_input("credential", "API key: sk-example-secret", true),
        "memory-secret-request",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    );
    assert!(matches!(result, Err(Error::InvalidInput(_))));
    assert!(core.list_assistant_actions()?.is_empty());
    assert!(core.list_assistant_memories(false)?.is_empty());
    Ok(())
}

#[test]
fn retrieval_budget_stays_bounded_as_memory_count_grows() -> Result<()> {
    let (_temporary, core) = fixture()?;
    for index in 0..250 {
        create_direct(
            &core,
            index,
            memory_input(
                &format!("집중 선호 {index:03}"),
                &format!("집중 작업 관련 선호 데이터 {index:03}"),
                true,
            ),
        )?;
    }
    let result = core.search_assistant_memories(MemorySearchFilter {
        query: "집중".to_owned(),
        kind: Some(MemoryKind::Preference),
        openai_only: true,
        max_items: 6,
        max_bytes: 2 * 1024,
    })?;
    assert!(result.items.len() <= 6);
    assert!(result.bytes_used <= 2 * 1024);
    assert!(result.omitted >= 194);
    assert_eq!(result.retrieval, "sqlite_fts5_structured_filters");
    assert!(!result.vector_service_used);
    Ok(())
}

#[test]
fn local_rollups_are_hierarchical_non_billable_and_retention_expires_them() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let source_id = create_direct(
        &core,
        1,
        memory_input("아침 루틴", "아침에는 스트레칭을 한다", true),
    )?;
    let today = Utc::now().date_naive();
    let report = core.regenerate_memory_summaries(today)?;
    assert!(report.daily.is_some());
    assert!(report.weekly.is_some());
    assert!(report.monthly.is_some());
    assert_eq!(report.openai_calls, 0);
    let summaries = core
        .list_assistant_memories(false)?
        .into_iter()
        .filter(|memory| memory.kind == MemoryKind::Summary)
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 3);

    let source = core.get_assistant_memory(&source_id)?;
    let delete = core.propose_memory_delete_action(
        &source.id,
        source.revision,
        "memory-rollup-source-delete",
        ASSISTANT_ACTION_APPROVAL_TTL_SECONDS,
    )?;
    approve(&core, &delete)?;
    let maintenance = core.run_memory_maintenance(Utc::now())?;
    assert!(maintenance.source_deleted >= 1);

    let expiry = core.run_memory_maintenance(Utc::now() + Duration::days(1_200))?;
    assert!(expiry.expired >= 1 || core.list_assistant_memories(false)?.is_empty());
    Ok(())
}

#[test]
fn restore_cannot_rewind_memory_provenance_or_rollups() -> Result<()> {
    let (_temporary, core) = fixture()?;
    create_direct(
        &core,
        1,
        memory_input("복구 보호", "기억 원장은 과거 상태로 돌아가지 않는다", true),
    )?;
    let backup = core.create_backup()?;
    core.regenerate_memory_summaries(Utc::now().date_naive())?;

    let restore = core.restore_backup(&backup.path);
    assert!(matches!(restore, Err(Error::Conflict(message)) if message.contains("memory")));
    assert_eq!(
        core.list_assistant_memories(false)?
            .into_iter()
            .filter(|memory| memory.kind == MemoryKind::Summary)
            .count(),
        3
    );
    Ok(())
}
