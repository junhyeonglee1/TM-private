use std::sync::{Arc, Barrier};

use rusqlite::{Connection, params};
use tempfile::TempDir;
use tm_core::{AssistantActionStatus, CreateTaskInput, Error, Result, TaskStatus, TmCore, TmHome};

fn fixture() -> Result<(TempDir, TmHome, TmCore)> {
    let temporary = tempfile::Builder::new()
        .prefix("tm-assistant-action-")
        .tempdir()?;
    let home = TmHome::new(temporary.path());
    let core = TmCore::open(home.clone())?;
    Ok((temporary, home, core))
}

fn input(title: &str) -> CreateTaskInput {
    CreateTaskInput {
        project_id: None,
        title: title.to_owned(),
        description: "locked description".to_owned(),
        status: TaskStatus::Todo,
        priority: 2,
        due_date: None,
    }
}

#[test]
fn proposal_is_locked_and_does_not_create_a_task() -> Result<()> {
    let (_temporary, home, core) = fixture()?;
    let action =
        core.propose_task_create_action(input("approval target"), "request-propose", 600)?;

    assert_eq!(action.status, AssistantActionStatus::Pending);
    assert_eq!(action.revision, 1);
    assert_eq!(action.payload_sha256.len(), 64);
    assert!(core.list_tasks(false)?.is_empty());

    let connection = Connection::open(home.database_path())?;
    let tamper = connection.execute(
        "UPDATE assistant_action_requests SET payload_json = ?1, revision = revision + 1 WHERE id = ?2",
        params!["{}", action.id],
    );
    assert!(tamper.is_err());
    assert_eq!(core.get_assistant_action(&action.id)?, action);
    Ok(())
}

#[test]
fn approval_executes_once_and_exact_retry_replays() -> Result<()> {
    let (_temporary, _home, core) = fixture()?;
    let action = core.propose_task_create_action(input("create once"), "request-propose", 600)?;
    let key = "approval-idempotency-0001";

    let first = core.approve_and_execute_assistant_action(
        &action.id,
        action.revision,
        &action.payload_sha256,
        key,
        "request-approve",
    )?;
    assert_eq!(first.action.status, AssistantActionStatus::Completed);
    assert!(!first.mutation_replayed);

    let retry = core.approve_and_execute_assistant_action(
        &action.id,
        action.revision,
        &action.payload_sha256,
        key,
        "request-approve-retry",
    )?;
    assert_eq!(retry.action.status, AssistantActionStatus::Completed);
    assert!(retry.mutation_replayed);
    assert_eq!(core.list_tasks(false)?.len(), 1);

    let audits = core.list_mutation_audit_events()?;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].actor, "tm_ai_assistant");
    assert_eq!(audits[0].approval_policy, "ai_action_approval");
    Ok(())
}

#[test]
fn stale_tampered_expired_and_rejected_approvals_are_refused() -> Result<()> {
    let (_temporary, _home, core) = fixture()?;
    let action = core.propose_task_create_action(input("guarded"), "request-propose", 600)?;

    let stale = core.approve_and_execute_assistant_action(
        &action.id,
        2,
        &action.payload_sha256,
        "approval-idempotency-stale",
        "request-stale",
    );
    assert!(matches!(stale, Err(Error::Conflict(_))));

    let mut wrong_hash = action.payload_sha256.clone();
    wrong_hash.replace_range(0..1, if &wrong_hash[0..1] == "0" { "1" } else { "0" });
    let tampered = core.approve_and_execute_assistant_action(
        &action.id,
        action.revision,
        &wrong_hash,
        "approval-idempotency-tamper",
        "request-tamper",
    );
    assert!(matches!(tampered, Err(Error::Conflict(_))));

    let rejected = core.reject_assistant_action(
        &action.id,
        action.revision,
        &action.payload_sha256,
        "request-reject",
        Some("not wanted"),
    )?;
    assert_eq!(rejected.status, AssistantActionStatus::Rejected);
    let after_reject = core.approve_and_execute_assistant_action(
        &action.id,
        rejected.revision,
        &action.payload_sha256,
        "approval-idempotency-rejected",
        "request-after-reject",
    );
    assert!(matches!(after_reject, Err(Error::Conflict(_))));

    let expired = core.propose_task_create_action(input("expire"), "request-expire", 0)?;
    assert_eq!(
        core.get_assistant_action(&expired.id)?.status,
        AssistantActionStatus::Expired
    );
    let after_expiry = core.approve_and_execute_assistant_action(
        &expired.id,
        expired.revision,
        &expired.payload_sha256,
        "approval-idempotency-expired",
        "request-after-expiry",
    );
    assert!(matches!(after_expiry, Err(Error::Conflict(_))));
    assert!(core.list_tasks(false)?.is_empty());
    Ok(())
}

#[test]
fn concurrent_exact_approval_creates_one_task() -> Result<()> {
    let (_temporary, _home, core) = fixture()?;
    let action = core.propose_task_create_action(input("concurrent"), "request-propose", 600)?;
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|index| {
            let core = core.clone();
            let action = action.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                core.approve_and_execute_assistant_action(
                    &action.id,
                    action.revision,
                    &action.payload_sha256,
                    "approval-idempotency-concurrent",
                    &format!("request-concurrent-{index}"),
                )
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("approval thread"))
        .collect::<Result<Vec<_>>>()?;

    assert_eq!(results.len(), 2);
    assert_eq!(
        results
            .iter()
            .filter(|result| result.mutation_replayed)
            .count(),
        1
    );
    assert_eq!(core.list_tasks(false)?.len(), 1);
    assert_eq!(core.list_mutation_audit_events()?.len(), 1);
    Ok(())
}

#[test]
fn interrupted_executing_state_recovers_through_mutation_idempotency() -> Result<()> {
    let (_temporary, home, core) = fixture()?;
    let action = core.propose_task_create_action(input("recover"), "request-propose", 600)?;
    let key = "approval-idempotency-recovery";
    let connection = Connection::open(home.database_path())?;
    connection.execute(
        "UPDATE assistant_action_requests
         SET status = 'approved', revision = 2, approval_idempotency_key = ?1, approved_at = ?2
         WHERE id = ?3",
        params![key, action.created_at, action.id],
    )?;
    connection.execute(
        "UPDATE assistant_action_requests
         SET status = 'executing', revision = 3, executing_at = ?1
         WHERE id = ?2",
        params![action.created_at, action.id],
    )?;
    drop(connection);

    let recovered = core.approve_and_execute_assistant_action(
        &action.id,
        action.revision,
        &action.payload_sha256,
        key,
        "request-recover",
    )?;
    assert_eq!(recovered.action.status, AssistantActionStatus::Completed);
    assert_eq!(core.list_tasks(false)?.len(), 1);
    Ok(())
}

#[test]
fn restore_cannot_rewind_the_assistant_approval_ledger() -> Result<()> {
    let (_temporary, _home, core) = fixture()?;
    let before_proposal = core.create_backup()?;
    let action =
        core.propose_task_create_action(input("preserve approval"), "request-propose", 600)?;

    let rewind = core.restore_backup(&before_proposal.path);
    assert!(matches!(rewind, Err(Error::Conflict(_))));
    assert_eq!(
        core.get_assistant_action(&action.id)?.status,
        AssistantActionStatus::Pending
    );

    let matching = core.create_backup()?;
    core.restore_backup(&matching.path)?;
    assert_eq!(
        core.get_assistant_action(&action.id)?.payload_sha256,
        action.payload_sha256
    );
    Ok(())
}
