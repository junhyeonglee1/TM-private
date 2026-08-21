use chrono::NaiveDate;
use serde_json::json;
use tempfile::TempDir;
use tm_core::{
    CreateTaskInput, DigestKind, Error, Result, TaskReportCompletion, TaskReportStart, TaskStatus,
    TmCore, TmHome,
};

fn fixture() -> Result<(TempDir, TmCore)> {
    let temporary = tempfile::Builder::new()
        .prefix("tm-task-report-")
        .tempdir()?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

#[test]
fn preview_selects_task_facts_without_exposing_descriptions() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let date = NaiveDate::from_ymd_opt(2026, 7, 22).expect("valid date");
    let task = core.create_task(CreateTaskInput {
        project_id: None,
        title: "오늘 우선 처리".to_owned(),
        description: "AI에 전달되면 안 되는 상세 설명".to_owned(),
        status: TaskStatus::Todo,
        priority: 3,
        due_date: Some(date),
    })?;
    core.plan_task(&task.id, date, None)?;

    let facts = core.preview_digest(DigestKind::Morning, date)?;
    assert_eq!(facts.today_planned.len(), 1);
    assert_eq!(facts.today_planned[0].title, "오늘 우선 처리");
    let serialized = serde_json::to_string(&facts)?;
    assert!(!serialized.contains("AI에 전달되면 안 되는 상세 설명"));
    assert!(core.digest_status()?.is_empty());
    Ok(())
}

#[test]
fn daily_attempt_limit_and_append_only_feedback_are_enforced() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let date = NaiveDate::from_ymd_opt(2026, 7, 22).expect("valid date");
    for index in 1..=4 {
        core.begin_task_report(&TaskReportStart {
            id: &format!("report-{index}"),
            date,
            actor: "primary-admin",
            candidate_count: 1,
            prompt_version: "step17-task-report-v1",
            model: "gpt-5.6-terra",
            daily_limit: 4,
        })?;
    }
    let fifth = core.begin_task_report(&TaskReportStart {
        id: "report-5",
        date,
        actor: "primary-admin",
        candidate_count: 1,
        prompt_version: "step17-task-report-v1",
        model: "gpt-5.6-terra",
        daily_limit: 4,
    });
    assert!(matches!(
        fifth,
        Err(Error::AiDailyLimitExceeded { limit: 4, .. })
    ));

    let report = json!({
        "headline": "우선순위",
        "summary": "요약",
        "priorities": [{
            "taskId": "task-1",
            "rank": 1,
            "reason": "이유",
            "nextAction": "바로 시작",
            "alert": ""
        }],
        "alerts": []
    });
    let completed = core.complete_task_report(
        "report-1",
        &TaskReportCompletion {
            status: "succeeded",
            response_id: Some("resp_1".to_owned()),
            upstream_request_id: None,
            result: Some(report),
            input_tokens: Some(100),
            cached_input_tokens: Some(0),
            output_tokens: Some(50),
            total_tokens: Some(150),
            estimated_cost_microusd: 1_000,
            latency_ms: 250,
            failure_code: None,
        },
    )?;
    assert_eq!(completed.status, "succeeded");
    let rated = core.rate_task_report("report-1", true, "primary-admin")?;
    assert_eq!(rated.helpful, Some(true));
    assert!(matches!(
        core.rate_task_report("report-1", false, "primary-admin"),
        Err(Error::Conflict(_))
    ));
    Ok(())
}

#[test]
fn restore_cannot_rewind_task_report_usage_or_feedback() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let before_report = core.create_backup()?;
    let date = NaiveDate::from_ymd_opt(2026, 7, 22).expect("valid date");
    let report = json!({
        "headline": "열린 Task 없음",
        "summary": "기록만 검증합니다.",
        "priorities": [],
        "alerts": []
    });
    core.record_empty_task_report(
        "empty-report",
        date,
        "primary-admin",
        "step17-task-report-v1",
        "gpt-5.6-terra",
        &report,
    )?;
    core.rate_task_report("empty-report", false, "primary-admin")?;

    assert!(matches!(
        core.restore_backup(&before_report.path),
        Err(Error::Conflict(message)) if message.contains("Task report")
    ));
    Ok(())
}
