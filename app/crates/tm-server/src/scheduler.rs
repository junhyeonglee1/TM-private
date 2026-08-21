use std::{collections::BTreeMap, time::Duration};

use chrono::{DateTime, Days, NaiveDate, Utc};
use serde_json::{Value, json};
use tm_core::{
    AiTokenUsage, Error as CoreError, ListStockScreenResultsInput, SCHEDULER_POLL_SECONDS,
    STOCK_AI_MAXIMUM_COST_MICROUSD, STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD, STOCK_AI_OPERATION,
    SchedulerClaim, StockAiReport, StockAiReportCompletion, StockAiReportStart, StockDailyBarInput,
    StockScreenBandFilter, StockScreenDirection, StockScreenHorizon, StockScreenListItem,
    StockScreenRun, StockScreenStartInput, StockUniverseMemberInput, StoreStockMarketDataInput,
    StoreStockUniverseInput, TmCore,
};
use tokio::{
    sync::watch,
    task::JoinHandle,
    time::{Instant, MissedTickBehavior},
};
use uuid::Uuid;

use crate::{
    mail_api::MailConfig,
    mail_crypto::MailCrypto,
    mail_worker,
    openai::{OpenAiClient, estimate_model_cost_microusd},
    stock::{
        STOCK_AI_MAX_CANDIDATES, STOCK_UNIVERSE_MAX_AGE_DAYS, StockAiCandidate, StockAiErrorKind,
        StockConfig, StockDataClient, StockDataError, UniverseDownload, run_ai_summary,
    },
};

const MAX_CLAIMS_PER_CYCLE: usize = 16;
const STOCK_UNIVERSE_REFRESH_DAYS: i64 = 7;
const STOCK_AI_PROMPT_VERSION: &str = "stock-daily-v1";
const STOCK_LICENSE_URL: &str = "https://opendatacommons.org/licenses/pddl/1-0/";
const STOCK_DATA_SOURCE: &str = "alpaca";
const STOCK_HEARTBEAT_SECONDS: u64 = 30;
const STOCK_HEARTBEAT_MAX_GAP_SECONDS: u64 = 180;

#[derive(Debug, Clone, Copy)]
struct StockAiRunOutcome {
    status: &'static str,
    cost_microusd: u64,
    openai_calls: u8,
}

impl StockAiRunOutcome {
    const fn new(status: &'static str, cost_microusd: u64, openai_calls: u8) -> Self {
        Self {
            status,
            cost_microusd,
            openai_calls,
        }
    }
}

pub fn spawn(
    core: TmCore,
    stock_data: StockDataClient,
    stock_config: StockConfig,
    openai: OpenAiClient,
    mail_config: MailConfig,
) -> JoinHandle<()> {
    let worker_id = format!("tm-server-{}", Uuid::now_v7());
    tokio::spawn(async move {
        let mail_crypto = if mail_config.enabled() {
            match MailCrypto::from_env() {
                Ok(crypto) => Some(crypto),
                Err(error) => {
                    tracing::error!(worker_id, %error, "mail scheduler encryption setup failed");
                    return;
                }
            }
        } else {
            None
        };
        let configured = {
            let core = core.clone();
            let enabled = stock_config.screen_enabled();
            let scheduler_mail_config = mail_config.clone();
            tokio::task::spawn_blocking(move || {
                core.configure_stock_scheduler(enabled, Utc::now())?;
                core.configure_mail_scheduler(
                    scheduler_mail_config.enabled(),
                    scheduler_mail_config.gmail_enabled(),
                    scheduler_mail_config.naver_enabled(),
                    scheduler_mail_config.ai_enabled(),
                    scheduler_mail_config.reports_enabled(),
                    Utc::now(),
                )
            })
            .await
        };
        match configured {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::error!(worker_id, %error, "scheduler configuration failed");
                return;
            }
            Err(error) => {
                tracing::error!(worker_id, %error, "scheduler configuration worker failed");
                return;
            }
        }

        let mut interval = tokio::time::interval(Duration::from_secs(SCHEDULER_POLL_SECONDS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = run_cycle(
                &core,
                &stock_data,
                &stock_config,
                &openai,
                &mail_config,
                mail_crypto.as_ref(),
                &worker_id,
            )
            .await
            {
                tracing::error!(worker_id, %error, "scheduler cycle failed");
            }
        }
    })
}

async fn run_cycle(
    core: &TmCore,
    stock_data: &StockDataClient,
    stock_config: &StockConfig,
    openai: &OpenAiClient,
    mail_config: &MailConfig,
    mail_crypto: Option<&MailCrypto>,
    worker_id: &str,
) -> Result<(), CoreError> {
    let reconcile_core = core.clone();
    let reconcile =
        tokio::task::spawn_blocking(move || reconcile_core.reconcile_scheduler(Utc::now()))
            .await
            .map_err(|_| CoreError::Invariant("scheduler reconcile worker failed".to_owned()))??;

    let mut succeeded = 0_usize;
    let mut failed = 0_usize;
    let mut recovered = 0_usize;
    for _ in 0..MAX_CLAIMS_PER_CYCLE {
        let claim_core = core.clone();
        let claim_worker_id = worker_id.to_owned();
        let (claim, lease_recoveries) = tokio::task::spawn_blocking(move || {
            claim_core.claim_scheduler_run(&claim_worker_id, Utc::now())
        })
        .await
        .map_err(|_| CoreError::Invariant("scheduler claim worker failed".to_owned()))??;
        recovered = recovered.saturating_add(lease_recoveries);
        let Some(claim) = claim else {
            break;
        };

        let outcome = if claim.job_kind == "stock.daily_screen" {
            execute_stock_claim(
                core.clone(),
                stock_data.clone(),
                stock_config.clone(),
                openai.clone(),
                claim.clone(),
            )
            .await
        } else if claim.job_kind.starts_with("mail.") {
            execute_mail_claim(
                core.clone(),
                mail_config.clone(),
                mail_crypto.cloned(),
                openai.clone(),
                claim.clone(),
            )
            .await
        } else {
            let execute_core = core.clone();
            let execute_claim = claim.clone();
            tokio::task::spawn_blocking(move || {
                execute_core.execute_scheduler_claim(&execute_claim, Utc::now())
            })
            .await
            .map_err(|_| CoreError::Invariant("scheduler execution worker failed".to_owned()))?
        };
        match outcome {
            Ok(()) => succeeded = succeeded.saturating_add(1),
            Err(error) => {
                failed = failed.saturating_add(1);
                if matches!(
                    &error,
                    CoreError::Conflict(message)
                        if message.contains("stock scheduler claim")
                            || message.contains("scheduler claim is no longer active")
                ) {
                    tracing::warn!(
                        run_id = %claim.run_id,
                        %error,
                        "stock scheduler claim was lost; leaving recovery to the active worker"
                    );
                    continue;
                }
                let fail_core = core.clone();
                let fail_claim = claim.clone();
                let failure = bounded_scheduler_error(&error);
                tokio::task::spawn_blocking(move || {
                    fail_core.fail_scheduler_claim(&fail_claim, &failure, true, Utc::now())
                })
                .await
                .map_err(|_| {
                    CoreError::Invariant("scheduler failure worker failed".to_owned())
                })??;
            }
        }
    }

    if reconcile.scheduled > 0
        || reconcile.skipped > 0
        || recovered > 0
        || succeeded > 0
        || failed > 0
    {
        tracing::info!(
            worker_id,
            scheduled = reconcile.scheduled,
            skipped = reconcile.skipped,
            lease_recoveries = recovered,
            succeeded,
            failed,
            "scheduler cycle completed"
        );
    }
    Ok(())
}

async fn execute_mail_claim(
    core: TmCore,
    config: MailConfig,
    crypto: Option<MailCrypto>,
    openai: OpenAiClient,
    claim: SchedulerClaim,
) -> Result<(), CoreError> {
    let (stop_heartbeat, mut claim_lost, heartbeat) = spawn_heartbeat(core.clone(), claim.clone());
    let operation = run_mail_job(&core, &config, crypto.as_ref(), &openai, &claim);
    tokio::pin!(operation);
    let result = tokio::select! {
        result = &mut operation => result,
        changed = claim_lost.changed() => {
            let _ = changed;
            Err(CoreError::Conflict(
                "scheduler claim is no longer active for mail work".to_owned(),
            ))
        }
    };
    let _ = stop_heartbeat.send(true);
    let _ = heartbeat.await;
    if *claim_lost.borrow() {
        return Err(CoreError::Conflict(
            "scheduler claim is no longer active before mail completion".to_owned(),
        ));
    }
    let result = result?;
    let verify_core = core.clone();
    let verify_claim = claim.clone();
    tokio::task::spawn_blocking(move || {
        verify_core.heartbeat_scheduler_claim(&verify_claim, Utc::now())
    })
    .await
    .map_err(|_| {
        CoreError::Invariant("mail scheduler final heartbeat worker failed".to_owned())
    })??;
    tokio::task::spawn_blocking(move || {
        core.complete_scheduler_claim_with_result(&claim, &result, Utc::now())
    })
    .await
    .map_err(|_| CoreError::Invariant("mail scheduler completion worker failed".to_owned()))??;
    Ok(())
}

async fn run_mail_job(
    core: &TmCore,
    config: &MailConfig,
    crypto: Option<&MailCrypto>,
    openai: &OpenAiClient,
    claim: &SchedulerClaim,
) -> Result<Value, CoreError> {
    if !config.enabled() {
        return Ok(json!({
            "status": "disabled",
            "kind": claim.job_kind,
            "openAiCalls": 0,
            "providerMutations": 0,
        }));
    }
    let crypto = crypto.ok_or_else(|| {
        CoreError::Invariant("mail scheduler encryption is unavailable".to_owned())
    })?;
    match mail_worker::execute(core, config, crypto, openai, claim).await {
        Ok(result) => Ok(result),
        Err(_error) if claim.job_kind == "mail.triage" => {
            tracing::warn!(
                run_id = %claim.run_id,
                "mail AI triage fell back to the existing rule decision"
            );
            Ok(json!({
                "status": "rule_fallback",
                "failureCode": "mail_triage_failed",
                "openAiCalls": 0,
                "providerMutations": 0,
                "fallbackActive": true,
            }))
        }
        Err(error) => Err(error),
    }
}

async fn execute_stock_claim(
    core: TmCore,
    stock_data: StockDataClient,
    stock_config: StockConfig,
    openai: OpenAiClient,
    claim: SchedulerClaim,
) -> Result<(), CoreError> {
    let (stop_heartbeat, mut claim_lost, heartbeat) = spawn_heartbeat(core.clone(), claim.clone());
    let operation = run_stock_screen(&core, &stock_data, &stock_config, &openai, &claim);
    tokio::pin!(operation);
    let result = tokio::select! {
        result = &mut operation => result,
        changed = claim_lost.changed() => {
            let _ = changed;
            Err(CoreError::Conflict(
                "stock scheduler claim was lost while work was active".to_owned(),
            ))
        }
    };
    let _ = stop_heartbeat.send(true);
    let _ = heartbeat.await;
    if *claim_lost.borrow() {
        return Err(CoreError::Conflict(
            "stock scheduler claim was lost before completion".to_owned(),
        ));
    }
    let result = result?;
    let verify_core = core.clone();
    let verify_claim = claim.clone();
    tokio::task::spawn_blocking(move || {
        verify_core.heartbeat_scheduler_claim(&verify_claim, Utc::now())
    })
    .await
    .map_err(|_| {
        CoreError::Invariant("stock scheduler final heartbeat worker failed".to_owned())
    })??;
    let complete_core = core.clone();
    let complete_claim = claim.clone();
    tokio::task::spawn_blocking(move || {
        complete_core.complete_scheduler_claim_with_result(&complete_claim, &result, Utc::now())
    })
    .await
    .map_err(|_| CoreError::Invariant("stock scheduler completion worker failed".to_owned()))??;
    Ok(())
}

fn spawn_heartbeat(
    core: TmCore,
    claim: SchedulerClaim,
) -> (watch::Sender<bool>, watch::Receiver<bool>, JoinHandle<()>) {
    let (stop_sender, mut stop_receiver) = watch::channel(false);
    let (lost_sender, lost_receiver) = watch::channel(false);
    let handle = tokio::spawn(async move {
        let mut last_success = Instant::now();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(STOCK_HEARTBEAT_SECONDS)) => {
                    let heartbeat_core = core.clone();
                    let heartbeat_claim = claim.clone();
                    match tokio::task::spawn_blocking(move || {
                        heartbeat_core.heartbeat_scheduler_claim(&heartbeat_claim, Utc::now())
                    }).await {
                        Ok(Ok(_)) => last_success = Instant::now(),
                        Ok(Err(CoreError::Conflict(error))) => {
                            tracing::warn!(run_id = %claim.run_id, %error, "scheduler heartbeat failed");
                            let _ = lost_sender.send(true);
                            break;
                        }
                        Ok(Err(error)) => {
                            tracing::warn!(run_id = %claim.run_id, %error, "scheduler heartbeat transient failure");
                            if last_success.elapsed()
                                >= Duration::from_secs(STOCK_HEARTBEAT_MAX_GAP_SECONDS)
                            {
                                let _ = lost_sender.send(true);
                                break;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(run_id = %claim.run_id, %error, "scheduler heartbeat worker failed");
                            let _ = lost_sender.send(true);
                            break;
                        }
                    }
                }
                changed = stop_receiver.changed() => {
                    if changed.is_err() || *stop_receiver.borrow() {
                        break;
                    }
                }
            }
        }
    });
    (stop_sender, lost_receiver, handle)
}

async fn run_stock_screen(
    core: &TmCore,
    stock_data: &StockDataClient,
    stock_config: &StockConfig,
    openai: &OpenAiClient,
    claim: &SchedulerClaim,
) -> Result<Value, CoreError> {
    if !stock_config.screen_enabled()
        || !stock_config.license_ack()
        || !stock_config.alpaca_configured()
    {
        return Ok(json!({
            "status": "disabled",
            "kind": "stock.daily_screen",
            "openAiCalls": 0,
        }));
    }

    let scheduled_for = DateTime::parse_from_rfc3339(&claim.scheduled_for)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| CoreError::Invariant("stock scheduler claim time is invalid".to_owned()))?;
    let provisional_date = core.latest_expected_stock_market_date(scheduled_for)?;
    let stable_run_id = claim.run_id.clone();
    let recovery_run = blocking(core.clone(), move |core| {
        core.get_stock_screen_run(&stable_run_id)
    })
    .await?;
    if let Some(existing_run) = recovery_run.as_ref() {
        if existing_run.status != "started" {
            return existing_stock_run_result(core, openai, stock_config, existing_run.clone())
                .await;
        }
        if existing_run.universe_snapshot_id.is_none() {
            return record_provider_failure(
                core,
                &claim.run_id,
                existing_run.market_date,
                "recovery_metadata_missing",
            )
            .await;
        }
    }
    let latest = blocking(core.clone(), |core| core.get_latest_stock_screen()).await?;
    if recovery_run.is_none()
        && latest
            .latest_success
            .as_ref()
            .is_some_and(|success| success.market_date >= provisional_date)
    {
        return Ok(json!({
            "status": "no_new_market_session",
            "kind": "stock.daily_screen",
            "marketDate": provisional_date,
            "openAiCalls": 0,
        }));
    }
    let existing = if let Some(snapshot_id) = recovery_run
        .as_ref()
        .and_then(|run| run.universe_snapshot_id.clone())
    {
        let snapshot = blocking(core.clone(), move |core| {
            core.get_stock_universe_snapshot(&snapshot_id)
        })
        .await?
        .ok_or_else(|| {
            CoreError::Invariant("pinned stock universe snapshot is missing".to_owned())
        })?;
        Some(snapshot)
    } else {
        blocking(core.clone(), |core| core.latest_stock_universe()).await?
    };
    let universe_age = existing
        .as_ref()
        .and_then(|snapshot| age_days(&snapshot.fetched_at));
    let should_refresh = recovery_run.is_none()
        && (existing.is_none()
            || universe_age.is_none_or(|days| days >= STOCK_UNIVERSE_REFRESH_DAYS));
    let target_market_date = recovery_run
        .as_ref()
        .map_or(provisional_date, |run| run.market_date);

    let downloaded_universe = if should_refresh {
        match stock_data.fetch_universe().await {
            Ok(download) => Some(download),
            Err(error)
                if existing.is_some()
                    && universe_age.is_some_and(|days| days <= STOCK_UNIVERSE_MAX_AGE_DAYS) =>
            {
                tracing::warn!(
                    code = stock_data_error_code(&error),
                    "stock universe refresh failed; using the last permitted snapshot"
                );
                None
            }
            Err(error) => {
                return record_provider_failure(
                    core,
                    &claim.run_id,
                    target_market_date,
                    stock_data_error_code(&error),
                )
                .await;
            }
        }
    } else {
        None
    };

    let member_inputs = if let Some(download) = &downloaded_universe {
        download
            .members
            .iter()
            .map(|member| StockUniverseMemberInput {
                ticker: member.symbol.clone(),
                display_name: member.display_name.clone(),
                sector: member.sector.clone(),
                sub_industry: member.sub_industry.clone(),
            })
            .collect::<Vec<_>>()
    } else {
        let snapshot = existing.as_ref().ok_or_else(|| {
            CoreError::Invariant("stock universe disappeared during collection".to_owned())
        })?;
        let snapshot_id = snapshot.id.clone();
        blocking(core.clone(), move |core| {
            core.stock_universe_members(&snapshot_id)
        })
        .await?
    };
    let symbols = member_inputs
        .iter()
        .map(|member| member.ticker.clone())
        .collect::<Vec<_>>();
    let mut market = match stock_data.fetch_daily_bars(&symbols, scheduled_for).await {
        Ok(market) => market,
        Err(error) => {
            return record_provider_failure(
                core,
                &claim.run_id,
                target_market_date,
                stock_data_error_code(&error),
            )
            .await;
        }
    };
    // Alpaca can expose an in-progress current-day daily bar. A scheduler claim is
    // pinned to the latest session that was expected to be complete at its own
    // scheduled time, so neither a late retry nor an intraday restart may advance
    // that claim to a partial or later session.
    market
        .sessions
        .retain(|session_date| *session_date <= target_market_date);
    market
        .bars
        .retain(|bar| bar.session_date <= target_market_date);
    let market_date = target_market_date;
    if !market.sessions.contains(&market_date) {
        let failure_code = if recovery_run.is_some() {
            "recovery_market_date_unavailable"
        } else {
            "provider_market_date_unavailable"
        };
        return record_provider_failure(core, &claim.run_id, market_date, failure_code).await;
    }

    let universe = if let Some(download) = downloaded_universe {
        let input = universe_store_input(download, member_inputs, market_date);
        blocking(core.clone(), move |core| core.store_stock_universe(input)).await?
    } else {
        existing.ok_or_else(|| {
            CoreError::Invariant("stock universe is missing after refresh".to_owned())
        })?
    };
    if recovery_run.is_none()
        && age_days(&universe.fetched_at).is_none_or(|days| days > STOCK_UNIVERSE_MAX_AGE_DAYS)
    {
        return record_provider_failure(
            core,
            &claim.run_id,
            market_date,
            "universe_snapshot_stale",
        )
        .await;
    }
    if universe.effective_date > market_date {
        return record_provider_failure(
            core,
            &claim.run_id,
            market_date,
            "universe_effective_date_after_market_date",
        )
        .await;
    }

    if recovery_run.is_none()
        && latest
            .latest_success
            .as_ref()
            .is_some_and(|success| success.market_date >= market_date)
    {
        return Ok(json!({
            "status": "no_new_market_session",
            "kind": "stock.daily_screen",
            "marketDate": market_date,
            "openAiCalls": 0,
        }));
    }

    let market_data_sha256 = market.source_sha256.clone();
    let bars = market
        .bars
        .into_iter()
        .map(|bar| {
            let close_microusd = u64::try_from(bar.close_microusd).map_err(|_| {
                CoreError::Invariant("stock close price cannot be represented".to_owned())
            })?;
            Ok(StockDailyBarInput {
                ticker: bar.symbol,
                session_date: bar.session_date,
                close_microusd,
            })
        })
        .collect::<Result<Vec<_>, CoreError>>()?;
    let market_input = StoreStockMarketDataInput {
        source: STOCK_DATA_SOURCE.to_owned(),
        feed: market.feed.to_owned(),
        adjustment: market.adjustment.to_owned(),
        source_sha256: market_data_sha256.clone(),
        sessions: market.sessions,
        bars,
    };
    blocking(core.clone(), move |core| {
        core.store_stock_market_data(market_input)
    })
    .await?;

    let screen_run_id = claim.run_id.clone();
    let pinned_universe_id = universe.id.clone();
    let run = blocking(core.clone(), move |core| {
        core.begin_stock_screen_run(StockScreenStartInput {
            run_id: Some(screen_run_id),
            market_date,
            universe_snapshot_id: Some(pinned_universe_id),
        })
    })
    .await?;
    let completed = if run.status == "started" {
        let run_id = run.id.clone();
        let universe_id = universe.id.clone();
        blocking(core.clone(), move |core| {
            core.complete_stock_screen_run(&run_id, &universe_id, &market_data_sha256)
        })
        .await?
    } else {
        run
    };

    let retain_from = market_date
        .checked_sub_days(Days::new(120))
        .unwrap_or(NaiveDate::MIN);
    let _ = blocking(core.clone(), move |core| {
        core.prune_stock_market_data(retain_from)
    })
    .await?;

    existing_stock_run_result(core, openai, stock_config, completed).await
}

async fn run_stock_ai(
    core: &TmCore,
    openai: &OpenAiClient,
    config: &StockConfig,
    run_id: &str,
    market_date: NaiveDate,
) -> Result<StockAiRunOutcome, CoreError> {
    let report_id = format!("stock-ai-{run_id}");
    let request_id = format!("stock:{run_id}");
    let existing_report_id = report_id.clone();
    if let Some(existing) = blocking(core.clone(), move |core| {
        core.get_stock_ai_report(&existing_report_id)
    })
    .await?
    {
        return reconcile_stock_ai_report(core, openai, existing, &request_id).await;
    }

    let candidates = load_ai_candidates(core, run_id).await?;
    if candidates.is_empty() {
        return Ok(StockAiRunOutcome::new("not_needed", 0, 0));
    }

    let report_start_id = report_id.clone();
    let report_run_id = run_id.to_owned();
    let model = config.openai_model().to_owned();
    blocking(core.clone(), move |core| {
        core.begin_stock_ai_report(StockAiReportStart {
            id: &report_start_id,
            screen_run_id: &report_run_id,
            prompt_version: STOCK_AI_PROMPT_VERSION,
            model: &model,
        })
    })
    .await?;

    let reservation = match blocking(core.clone(), {
        let request_id = request_id.clone();
        let model = config.openai_model().to_owned();
        let policy = openai.config().budget_policy();
        move |core| {
            core.reserve_ai_budget_with_operation_limit(
                &request_id,
                "openai",
                &model,
                STOCK_AI_OPERATION,
                STOCK_AI_MAXIMUM_COST_MICROUSD,
                policy,
                STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD,
            )
        }
    })
    .await
    {
        Ok(reservation) => reservation,
        Err(CoreError::AiBudgetExceeded { .. }) => {
            complete_ai_failure(core, &report_id, "failed", "budget_blocked", 0, None, None)
                .await?;
            return Ok(StockAiRunOutcome::new("budget_blocked", 0, 0));
        }
        Err(error) => return Err(error),
    };

    let calling_report_id = report_id.clone();
    let call_authorized = blocking(core.clone(), move |core| {
        core.try_mark_stock_ai_report_calling(&calling_report_id)
    })
    .await?;
    if !call_authorized {
        let report_lookup_id = report_id.clone();
        let existing = blocking(core.clone(), move |core| {
            core.get_stock_ai_report(&report_lookup_id)
        })
        .await?
        .ok_or_else(|| {
            CoreError::Invariant("stock AI report disappeared before its call".to_owned())
        })?;
        return reconcile_stock_ai_report(core, openai, existing, &request_id).await;
    }

    let execution = run_ai_summary(
        openai.clone(),
        config.openai_model(),
        market_date,
        &candidates,
    )
    .await;
    match execution {
        Ok(execution) => {
            let usage = execution.usage;
            let Some(estimated_cost) = usage
                .as_ref()
                .and_then(|usage| estimate_model_cost_microusd(config.openai_model(), usage))
            else {
                let settlement_reservation = reservation.clone();
                let policy = openai.config().budget_policy();
                blocking(core.clone(), move |core| {
                    core.settle_ai_budget(
                        &settlement_reservation,
                        STOCK_AI_MAXIMUM_COST_MICROUSD,
                        None,
                        "upstream_cost_estimate",
                        policy,
                    )
                })
                .await?;
                complete_ai_failure(
                    core,
                    &report_id,
                    "ai_uncertain",
                    "usage_missing",
                    STOCK_AI_MAXIMUM_COST_MICROUSD,
                    Some(execution.response_id),
                    execution.upstream_request_id,
                )
                .await?;
                return Ok(StockAiRunOutcome::new(
                    "uncertain",
                    STOCK_AI_MAXIMUM_COST_MICROUSD,
                    1,
                ));
            };
            if estimated_cost > STOCK_AI_MAXIMUM_COST_MICROUSD {
                let settlement_usage = usage.map(to_ai_token_usage);
                let settlement_reservation = reservation.clone();
                let policy = openai.config().budget_policy();
                blocking(core.clone(), move |core| {
                    core.settle_ai_budget(
                        &settlement_reservation,
                        estimated_cost,
                        settlement_usage,
                        "upstream_cost_estimate",
                        policy,
                    )
                })
                .await?;
                complete_ai_failure(
                    core,
                    &report_id,
                    "ai_uncertain",
                    "usage_exceeded_reservation",
                    estimated_cost,
                    Some(execution.response_id),
                    execution.upstream_request_id,
                )
                .await?;
                return Ok(StockAiRunOutcome::new("uncertain", estimated_cost, 1));
            }
            let settlement_usage = usage.map(to_ai_token_usage);
            let settlement_reservation = reservation.clone();
            let policy = openai.config().budget_policy();
            blocking(core.clone(), move |core| {
                core.settle_ai_budget(
                    &settlement_reservation,
                    estimated_cost,
                    settlement_usage,
                    "succeeded",
                    policy,
                )
            })
            .await?;
            let result = serde_json::to_value(&execution.content)?;
            let completion = StockAiReportCompletion {
                status: "succeeded",
                response_id: Some(execution.response_id),
                upstream_request_id: execution.upstream_request_id,
                result: Some(result),
                input_tokens: usage.map(|usage| usage.input_tokens),
                cached_input_tokens: usage.map(|usage| usage.cached_input_tokens),
                output_tokens: usage.map(|usage| usage.output_tokens),
                total_tokens: usage.map(|usage| usage.total_tokens),
                estimated_cost_microusd: estimated_cost,
                failure_code: None,
            };
            let completion_id = report_id.clone();
            blocking(core.clone(), move |core| {
                core.complete_stock_ai_report(&completion_id, &completion)
            })
            .await?;
            Ok(StockAiRunOutcome::new("succeeded", estimated_cost, 1))
        }
        Err(error) => {
            let possibly_billed = error.possibly_billed;
            let upstream_request_id = error.upstream_request_id;
            let estimated_cost = if possibly_billed {
                STOCK_AI_MAXIMUM_COST_MICROUSD
            } else {
                0
            };
            let outcome = if possibly_billed {
                "upstream_cost_estimate"
            } else {
                "preflight_failed"
            };
            let settlement_reservation = reservation.clone();
            let policy = openai.config().budget_policy();
            blocking(core.clone(), move |core| {
                core.settle_ai_budget(
                    &settlement_reservation,
                    estimated_cost,
                    None,
                    outcome,
                    policy,
                )
            })
            .await?;
            let failure_code = match error.kind {
                StockAiErrorKind::OpenAi(_) => "openai_unavailable",
                StockAiErrorKind::InvalidConfiguration => "invalid_configuration",
                StockAiErrorKind::InvalidResponse => "invalid_response",
            };
            let status = if possibly_billed {
                "ai_uncertain"
            } else {
                "failed"
            };
            complete_ai_failure(
                core,
                &report_id,
                status,
                failure_code,
                estimated_cost,
                None,
                upstream_request_id,
            )
            .await?;
            Ok(StockAiRunOutcome::new(
                if possibly_billed {
                    "uncertain"
                } else {
                    "failed"
                },
                estimated_cost,
                1,
            ))
        }
    }
}

async fn existing_stock_run_result(
    core: &TmCore,
    openai: &OpenAiClient,
    config: &StockConfig,
    run: StockScreenRun,
) -> Result<Value, CoreError> {
    let ai = if run.status == "succeeded" && run.result_count > 0 && config.ai_enabled() {
        run_stock_ai(core, openai, config, &run.id, run.market_date).await?
    } else if config.ai_enabled() {
        StockAiRunOutcome::new("not_needed", 0, 0)
    } else {
        StockAiRunOutcome::new("disabled", 0, 0)
    };
    Ok(json!({
        "status": run.status,
        "kind": "stock.daily_screen",
        "runId": run.id,
        "marketDate": run.market_date,
        "resultCount": run.result_count,
        "failureCode": run.failure_code,
        "coverage": {
            "current": run.current_covered,
            "baseline5": run.baseline_5_covered,
            "baseline21": run.baseline_21_covered,
            "total": run.total_members,
        },
        "aiStatus": ai.status,
        "aiCostMicrousd": ai.cost_microusd,
        "openAiCalls": ai.openai_calls,
    }))
}

async fn reconcile_stock_ai_report(
    core: &TmCore,
    openai: &OpenAiClient,
    report: StockAiReport,
    request_id: &str,
) -> Result<StockAiRunOutcome, CoreError> {
    if report.status != "started" {
        return terminal_stock_ai_outcome(&report);
    }

    let reservation_request_id = request_id.to_owned();
    let reservation = blocking(core.clone(), move |core| {
        core.get_ai_budget_reservation(&reservation_request_id)
    })
    .await?;
    let settlement_request_id = request_id.to_owned();
    let settlement = blocking(core.clone(), move |core| {
        core.get_ai_budget_settlement(&settlement_request_id)
    })
    .await?;

    if report.request_started_at.is_none() {
        let cost = if let Some(settlement) = settlement {
            settlement.actual_cost_microusd
        } else if let Some(reservation) = reservation {
            let policy = openai.config().budget_policy();
            blocking(core.clone(), move |core| {
                core.settle_ai_budget(&reservation, 0, None, "preflight_failed", policy)
            })
            .await?;
            0
        } else {
            0
        };
        complete_ai_failure(
            core,
            &report.id,
            "failed",
            "recovered_before_call",
            cost,
            None,
            None,
        )
        .await?;
        return Ok(StockAiRunOutcome::new("failed", cost, 0));
    }

    let had_reservation = reservation.is_some();
    let cost = if let Some(settlement) = settlement {
        settlement.actual_cost_microusd
    } else if let Some(reservation) = reservation {
        let policy = openai.config().budget_policy();
        blocking(core.clone(), move |core| {
            core.settle_ai_budget(
                &reservation,
                STOCK_AI_MAXIMUM_COST_MICROUSD,
                None,
                "upstream_cost_estimate",
                policy,
            )
        })
        .await?;
        STOCK_AI_MAXIMUM_COST_MICROUSD
    } else {
        STOCK_AI_MAXIMUM_COST_MICROUSD
    };
    let failure_code = if had_reservation {
        "recovered_after_call_started"
    } else {
        "recovered_without_reservation"
    };
    complete_ai_failure(
        core,
        &report.id,
        "ai_uncertain",
        failure_code,
        cost,
        report.response_id,
        report.upstream_request_id,
    )
    .await?;
    Ok(StockAiRunOutcome::new("uncertain", cost, 0))
}

fn terminal_stock_ai_outcome(report: &StockAiReport) -> Result<StockAiRunOutcome, CoreError> {
    let status = match report.status.as_str() {
        "succeeded" => "succeeded",
        "ai_uncertain" => "uncertain",
        "failed" if report.failure_code.as_deref() == Some("budget_blocked") => "budget_blocked",
        "failed" => "failed",
        other => {
            return Err(CoreError::Invariant(format!(
                "unsupported terminal stock AI report status: {other}"
            )));
        }
    };
    Ok(StockAiRunOutcome::new(
        status,
        report.estimated_cost_microusd,
        0,
    ))
}

async fn complete_ai_failure(
    core: &TmCore,
    report_id: &str,
    status: &'static str,
    failure_code: &'static str,
    estimated_cost_microusd: u64,
    response_id: Option<String>,
    upstream_request_id: Option<String>,
) -> Result<(), CoreError> {
    let completion = StockAiReportCompletion {
        status,
        response_id,
        upstream_request_id,
        result: None,
        input_tokens: None,
        cached_input_tokens: None,
        output_tokens: None,
        total_tokens: None,
        estimated_cost_microusd,
        failure_code: Some(failure_code.to_owned()),
    };
    let report_id = report_id.to_owned();
    blocking(core.clone(), move |core| {
        core.complete_stock_ai_report(&report_id, &completion)
    })
    .await?;
    Ok(())
}

async fn load_ai_candidates(
    core: &TmCore,
    run_id: &str,
) -> Result<Vec<StockAiCandidate>, CoreError> {
    let mut unique = BTreeMap::<(String, u8), (i64, StockAiCandidate)>::new();
    for horizon in [StockScreenHorizon::Five, StockScreenHorizon::TwentyOne] {
        for direction in [StockScreenDirection::Up, StockScreenDirection::Down] {
            for band in [
                StockScreenBandFilter::TenToTwenty,
                StockScreenBandFilter::TwentyPlus,
            ] {
                let input = ListStockScreenResultsInput {
                    run_id: run_id.to_owned(),
                    horizon,
                    direction,
                    band: Some(band),
                    cursor: None,
                    limit: Some(50),
                };
                let page = blocking(core.clone(), move |core| {
                    core.list_stock_screen_results(input)
                })
                .await?;
                for item in page.items {
                    let exact_return = item.return_micros;
                    let candidate = ai_candidate(item);
                    unique.insert(
                        (candidate.symbol.clone(), candidate.horizon),
                        (exact_return, candidate),
                    );
                }
            }
        }
    }
    let mut candidates = unique.into_values().collect::<Vec<_>>();
    candidates.sort_by(|(left_return, left), (right_return, right)| {
        right_return
            .unsigned_abs()
            .cmp(&left_return.unsigned_abs())
            .then_with(|| left.symbol.cmp(&right.symbol))
            .then_with(|| left.horizon.cmp(&right.horizon))
    });
    candidates.truncate(STOCK_AI_MAX_CANDIDATES);
    Ok(candidates
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect())
}

fn ai_candidate(item: StockScreenListItem) -> StockAiCandidate {
    let basis_points = item.return_micros / 100;
    StockAiCandidate {
        symbol: item.ticker,
        display_name: item.display_name,
        horizon: item.horizon.sessions(),
        return_micros: item.return_micros,
        return_basis_points: i32::try_from(basis_points).unwrap_or_else(|_| {
            if basis_points.is_negative() {
                i32::MIN
            } else {
                i32::MAX
            }
        }),
        direction: match item.direction {
            StockScreenDirection::Up => "up",
            StockScreenDirection::Down => "down",
        }
        .to_owned(),
        band: item.band.as_str().to_owned(),
    }
}

fn universe_store_input(
    download: UniverseDownload,
    members: Vec<StockUniverseMemberInput>,
    effective_date: NaiveDate,
) -> StoreStockUniverseInput {
    let source_revision = format!("{effective_date}:{}", download.revision);
    StoreStockUniverseInput {
        name: "S&P 500 constituents (DataHub/Wikipedia snapshot)".to_owned(),
        effective_date,
        source_url: download.source_url,
        source_revision,
        source_sha256: download.sha256,
        license_name: download.license,
        license_url: STOCK_LICENSE_URL.to_owned(),
        attribution_text: download.attribution,
        members,
    }
}

async fn record_provider_failure(
    core: &TmCore,
    run_id: &str,
    market_date: NaiveDate,
    failure_code: &'static str,
) -> Result<Value, CoreError> {
    let run_id = run_id.to_owned();
    let run = blocking(core.clone(), move |core| {
        core.begin_stock_screen_run(StockScreenStartInput {
            run_id: Some(run_id),
            market_date,
            universe_snapshot_id: None,
        })
    })
    .await?;
    let failed = if run.status == "started" {
        let run_id = run.id.clone();
        blocking(core.clone(), move |core| {
            core.fail_stock_screen_run(&run_id, "upstream_unavailable", failure_code)
        })
        .await?
    } else {
        run
    };
    Ok(json!({
        "status": failed.status,
        "kind": "stock.daily_screen",
        "runId": failed.id,
        "marketDate": failed.market_date,
        "failureCode": failed.failure_code,
        "openAiCalls": 0,
    }))
}

fn stock_data_error_code(error: &StockDataError) -> &'static str {
    match error {
        StockDataError::NotConfigured => "provider_not_configured",
        StockDataError::Authentication => "provider_authentication_failed",
        StockDataError::RateLimited => "provider_rate_limited",
        StockDataError::UpstreamUnavailable => "provider_unavailable",
        StockDataError::ResponseTooLarge => "provider_response_too_large",
        StockDataError::InvalidResponse => "provider_response_invalid",
        StockDataError::CoverageUnavailable => "provider_coverage_unavailable",
    }
}

fn age_days(timestamp: &str) -> Option<i64> {
    let fetched = DateTime::parse_from_rfc3339(timestamp)
        .ok()?
        .with_timezone(&Utc);
    Some(Utc::now().signed_duration_since(fetched).num_days().max(0))
}

fn to_ai_token_usage(usage: crate::openai::ProbeUsage) -> AiTokenUsage {
    AiTokenUsage {
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens,
    }
}

async fn blocking<T, F>(core: TmCore, operation: F) -> Result<T, CoreError>
where
    T: Send + 'static,
    F: FnOnce(TmCore) -> Result<T, CoreError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || operation(core))
        .await
        .map_err(|_| CoreError::Invariant("stock database worker failed".to_owned()))?
}

fn bounded_scheduler_error(error: &CoreError) -> String {
    let message = error.to_string();
    if message.len() <= 512 {
        message
    } else {
        "stock scheduler database operation failed".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ai_candidate, execute_stock_claim, reconcile_stock_ai_report, run_stock_ai,
        run_stock_screen,
    };
    use crate::{
        openai::{OpenAiClient, OpenAiConfig},
        stock::{DEFAULT_STOCK_OPENAI_MODEL, StockConfig, StockDataClient},
    };
    use axum::{
        Json, Router,
        body::Body,
        extract::State,
        http::StatusCode,
        response::Response,
        routing::{get, post},
    };
    use chrono::{Datelike, Days, NaiveDate, TimeZone, Utc, Weekday};
    use serde_json::{Value, json};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tm_core::{
        STOCK_AI_MAXIMUM_COST_MICROUSD, STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD, STOCK_AI_OPERATION,
        SchedulerClaim, StockAiReportStart, StockDailyBarInput, StockScreenBand,
        StockScreenDirection, StockScreenHorizon, StockScreenListItem, StockScreenStartInput,
        StockUniverseMemberInput, StoreStockMarketDataInput, StoreStockUniverseInput, TmCore,
        TmHome,
    };

    fn weekday_sessions(count: usize) -> Vec<NaiveDate> {
        let mut sessions = Vec::new();
        let mut date = Utc::now()
            .date_naive()
            .checked_sub_days(Days::new(2))
            .expect("past date");
        while sessions.len() < count {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                sessions.push(date);
            }
            date = date.pred_opt().expect("previous date");
        }
        sessions.sort_unstable();
        sessions
    }

    fn seed_ai_screen(core: &TmCore, run_id: &str) -> NaiveDate {
        let sessions = weekday_sessions(22);
        let universe = core
            .store_stock_universe(StoreStockUniverseInput {
                name: "AI recovery fixture".to_owned(),
                effective_date: sessions[0],
                source_url: format!("https://example.test/{run_id}.csv"),
                source_revision: run_id.to_owned(),
                source_sha256: "c".repeat(64),
                license_name: "PDDL-1.0".to_owned(),
                license_url: "https://opendatacommons.org/licenses/pddl/1-0/".to_owned(),
                attribution_text: "AI recovery fixture".to_owned(),
                members: vec![StockUniverseMemberInput {
                    ticker: "AAPL".to_owned(),
                    display_name: "Apple".to_owned(),
                    sector: Some("Technology".to_owned()),
                    sub_industry: None,
                }],
            })
            .expect("store AI fixture universe");
        let market_hash = "d".repeat(64);
        core.store_stock_market_data(StoreStockMarketDataInput {
            source: "fixture".to_owned(),
            feed: "sip".to_owned(),
            adjustment: "split".to_owned(),
            source_sha256: market_hash.clone(),
            sessions: sessions.clone(),
            bars: sessions
                .iter()
                .enumerate()
                .map(|(index, session_date)| StockDailyBarInput {
                    ticker: "AAPL".to_owned(),
                    session_date: *session_date,
                    close_microusd: if index + 1 == sessions.len() {
                        80_000_000
                    } else {
                        100_000_000
                    },
                })
                .collect(),
        })
        .expect("store AI fixture market data");
        core.begin_stock_screen_run(StockScreenStartInput {
            run_id: Some(run_id.to_owned()),
            market_date: *sessions.last().expect("market date"),
            universe_snapshot_id: Some(universe.id.clone()),
        })
        .expect("begin AI fixture screen");
        let completed = core
            .complete_stock_screen_run(run_id, &universe.id, &market_hash)
            .expect("complete AI fixture screen");
        assert_eq!(completed.status, "succeeded");
        assert!(completed.result_count > 0);
        completed.market_date
    }

    fn seed_ai_report(
        core: &TmCore,
        run_id: &str,
        report_id: &str,
        request_id: &str,
        openai: &OpenAiClient,
    ) -> tm_core::StockAiReport {
        seed_ai_screen(core, run_id);
        let report = core
            .begin_stock_ai_report(StockAiReportStart {
                id: report_id,
                screen_run_id: run_id,
                prompt_version: "stock-daily-v1",
                model: "gpt-5.4-nano-2026-03-17",
            })
            .expect("begin AI fixture report");
        core.reserve_ai_budget_with_operation_limit(
            request_id,
            "openai",
            "gpt-5.4-nano-2026-03-17",
            STOCK_AI_OPERATION,
            STOCK_AI_MAXIMUM_COST_MICROUSD,
            openai.config().budget_policy(),
            STOCK_AI_MONTHLY_HARD_LIMIT_MICROUSD,
        )
        .expect("reserve AI fixture budget");
        report
    }

    #[test]
    fn ai_candidate_converts_return_scale_to_basis_points() {
        let candidate = ai_candidate(StockScreenListItem {
            run_id: "run".to_owned(),
            ticker: "AAPL".to_owned(),
            display_name: "Apple".to_owned(),
            sector: None,
            horizon: StockScreenHorizon::Five,
            direction: StockScreenDirection::Down,
            band: StockScreenBand::Down10To20,
            current_date: NaiveDate::from_ymd_opt(2026, 7, 24).expect("date"),
            current_close_microusd: 90_000_000,
            baseline_date: NaiveDate::from_ymd_opt(2026, 7, 17).expect("date"),
            baseline_close_microusd: 100_000_000,
            return_micros: -100_000,
            return_pct: -10.0,
            universe_sha256: "a".repeat(64),
            market_data_sha256: "b".repeat(64),
        });
        assert_eq!(candidate.return_basis_points, -1_000);
        assert_eq!(candidate.horizon, 5);
    }

    #[tokio::test]
    async fn disabled_stock_gates_make_no_external_call() {
        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let config = StockConfig::default();
        let data = StockDataClient::new(config.clone()).expect("disabled data client");
        let claim = SchedulerClaim {
            run_id: "scheduler-run".to_owned(),
            job_key: "stock.daily_screen".to_owned(),
            job_kind: "stock.daily_screen".to_owned(),
            scheduled_for: "2026-07-27T01:30:00Z".to_owned(),
            idempotency_key: "stock:2026-07-27T01:30:00Z".to_owned(),
            attempt_number: 1,
            max_attempts: 5,
            worker_id: "test-worker".to_owned(),
            lease_expires_at: "2026-07-27T01:35:00Z".to_owned(),
        };
        let result = run_stock_screen(&core, &data, &config, &OpenAiClient::disabled(), &claim)
            .await
            .expect("disabled screen");
        assert_eq!(
            result.get("status").and_then(|value| value.as_str()),
            Some("disabled")
        );
        assert_eq!(
            result.get("openAiCalls").and_then(|value| value.as_u64()),
            Some(0)
        );
    }

    #[tokio::test]
    async fn successful_stock_ai_call_settles_exact_cost_once_and_is_terminal() {
        async fn structured_success(
            State(calls): State<Arc<AtomicUsize>>,
            Json(request): Json<Value>,
        ) -> Response {
            calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                request.get("model").and_then(Value::as_str),
                Some(DEFAULT_STOCK_OPENAI_MODEL)
            );
            assert_eq!(
                request.get("service_tier").and_then(Value::as_str),
                Some("default")
            );
            assert_eq!(
                request.pointer("/text/format/type").and_then(Value::as_str),
                Some("json_schema")
            );
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .header("x-request-id", "req_stock_success")
                .body(Body::from(
                    json!({
                        "id": "resp_stock_success",
                        "status": "completed",
                        "model": DEFAULT_STOCK_OPENAI_MODEL,
                        "output": [{
                            "type": "message",
                            "content": [{
                                "type": "output_text",
                                "text": "{\"notableSymbols\":[\"AAPL\"]}"
                            }]
                        }],
                        "usage": {
                            "input_tokens": 100,
                            "input_tokens_details": {"cached_tokens": 20},
                            "output_tokens": 10,
                            "total_tokens": 110
                        }
                    })
                    .to_string(),
                ))
                .expect("structured success response")
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind OpenAI provider");
        let address = listener.local_addr().expect("OpenAI provider address");
        let provider_calls = calls.clone();
        let provider = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v1/responses", post(structured_success))
                    .with_state(provider_calls),
            )
            .await
            .expect("serve OpenAI provider");
        });

        let origin = format!("http://{address}");
        let openai = OpenAiClient::new(
            OpenAiConfig::for_test(
                Some("test-key"),
                DEFAULT_STOCK_OPENAI_MODEL,
                &format!("{origin}/v1/"),
                Duration::from_secs(5),
            )
            .expect("OpenAI test configuration"),
        )
        .expect("OpenAI test client");
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let run_id = "screen-ai-success";
        let market_date = seed_ai_screen(&core, run_id);

        let outcome = run_stock_ai(&core, &openai, &config, run_id, market_date)
            .await
            .expect("run stock AI");
        assert_eq!(outcome.status, "succeeded");
        assert_eq!(outcome.cost_microusd, 29);
        assert_eq!(outcome.openai_calls, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let reservation = core
            .get_ai_budget_reservation("stock:screen-ai-success")
            .expect("read successful reservation")
            .expect("successful reservation");
        assert_eq!(
            reservation.reserved_microusd,
            STOCK_AI_MAXIMUM_COST_MICROUSD
        );
        let settlement = core
            .get_ai_budget_settlement("stock:screen-ai-success")
            .expect("read successful settlement")
            .expect("successful settlement");
        assert_eq!(settlement.actual_cost_microusd, 29);
        assert_eq!(settlement.outcome, "succeeded");

        let report = core
            .get_stock_ai_report("stock-ai-screen-ai-success")
            .expect("read successful AI report")
            .expect("successful AI report");
        assert_eq!(report.status, "succeeded");
        assert_eq!(report.response_id.as_deref(), Some("resp_stock_success"));
        assert_eq!(
            report.upstream_request_id.as_deref(),
            Some("req_stock_success")
        );
        assert_eq!(report.input_tokens, Some(100));
        assert_eq!(report.cached_input_tokens, Some(20));
        assert_eq!(report.output_tokens, Some(10));
        assert_eq!(report.total_tokens, Some(110));
        assert_eq!(report.estimated_cost_microusd, 29);
        assert!(report.result.is_some());
        assert!(report.failure_code.is_none());
        assert!(report.completed_at.is_some());

        let replay = run_stock_ai(&core, &openai, &config, run_id, market_date)
            .await
            .expect("replay terminal stock AI");
        assert_eq!(replay.status, "succeeded");
        assert_eq!(replay.cost_microusd, 29);
        assert_eq!(replay.openai_calls, 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        provider.abort();
    }

    #[tokio::test]
    async fn provider_rate_limit_is_terminal_and_replay_makes_no_second_call() {
        async fn rate_limited(State(calls): State<Arc<AtomicUsize>>) -> StatusCode {
            calls.fetch_add(1, Ordering::SeqCst);
            StatusCode::TOO_MANY_REQUESTS
        }

        let calls = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind rate-limited provider");
        let address = listener.local_addr().expect("provider address");
        let provider_calls = calls.clone();
        let provider = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/constituents.csv", get(rate_limited))
                    .with_state(provider_calls),
            )
            .await
            .expect("serve rate-limited provider");
        });

        let origin = format!("http://{address}");
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let data = StockDataClient::new(config.clone()).expect("stock data client");
        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let configured_at = Utc
            .with_ymd_and_hms(2099, 7, 25, 0, 0, 0)
            .single()
            .expect("fixed scheduler setup time");
        core.configure_stock_scheduler(true, configured_at)
            .expect("configure stock scheduler");
        let scheduled_for = core
            .stock_scheduler_next_run_at()
            .expect("read next stock run")
            .and_then(|value| {
                chrono::DateTime::parse_from_rfc3339(&value)
                    .ok()
                    .map(|value| value.with_timezone(&Utc))
            })
            .expect("parse next stock run");
        core.run_scheduler_cycle("prerequisite-drain-worker", scheduled_for)
            .expect("drain built-in scheduler work");
        let before_effects = core
            .scheduler_status(scheduled_for)
            .expect("read effects before stock claim")
            .effect_count;
        let (claim, _) = core
            .claim_scheduler_run("rate-limit-worker", scheduled_for)
            .expect("claim stock scheduler run");
        let claim = claim.expect("pending stock scheduler claim");
        assert_eq!(claim.job_kind, "stock.daily_screen");

        execute_stock_claim(
            core.clone(),
            data.clone(),
            config.clone(),
            OpenAiClient::disabled(),
            claim.clone(),
        )
        .await
        .expect("complete rate-limited scheduler claim");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let terminal = core
            .get_stock_screen_run(&claim.run_id)
            .expect("read failed screen")
            .expect("failed screen");
        assert_eq!(terminal.status, "upstream_unavailable");
        assert_eq!(
            terminal.failure_code.as_deref(),
            Some("provider_rate_limited")
        );
        assert!(terminal.completed_at.is_some());

        let scheduler_run = core
            .list_scheduler_runs()
            .expect("list scheduler runs")
            .into_iter()
            .find(|run| run.id == claim.run_id)
            .expect("completed stock scheduler run");
        assert_eq!(scheduler_run.status, "succeeded");
        let scheduler_result = scheduler_run.result.expect("scheduler effect result");
        assert_eq!(scheduler_result["status"], "upstream_unavailable");
        assert_eq!(scheduler_result["failureCode"], "provider_rate_limited");
        assert_eq!(scheduler_result["openAiCalls"], 0);
        assert_eq!(
            core.scheduler_status(scheduled_for)
                .expect("read effects after stock claim")
                .effect_count,
            before_effects + 1
        );

        let replay = run_stock_screen(&core, &data, &config, &OpenAiClient::disabled(), &claim)
            .await
            .expect("replay provider failure");
        assert_eq!(replay["status"], "upstream_unavailable");
        assert_eq!(replay["failureCode"], "provider_rate_limited");
        assert_eq!(replay["openAiCalls"], 0);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            core.scheduler_status(scheduled_for)
                .expect("read effects after replay")
                .effect_count,
            before_effects + 1
        );

        provider.abort();
    }

    #[tokio::test]
    async fn fresh_screen_ignores_provider_sessions_after_the_claim_cutoff() {
        async fn bars(State(payload): State<Value>) -> Json<Value> {
            Json(payload)
        }

        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let scheduled_for = Utc
            .with_ymd_and_hms(2026, 7, 25, 20, 0, 0)
            .single()
            .expect("fixed Saturday claim time");
        let target_market_date = core
            .latest_expected_stock_market_date(scheduled_for)
            .expect("expected complete market date");
        let mut confirmed_sessions = Vec::new();
        let mut date = target_market_date;
        while confirmed_sessions.len() < 22 {
            if !matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                confirmed_sessions.push(date);
            }
            date = date.pred_opt().expect("previous date");
        }
        confirmed_sessions.sort_unstable();
        let partial_later_date = target_market_date
            .checked_add_days(Days::new(1))
            .expect("later date");
        let mut provider_sessions = confirmed_sessions.clone();
        provider_sessions.push(partial_later_date);
        let provider_bars = provider_sessions
            .iter()
            .map(|date| json!({"t": format!("{date}T12:00:00Z"), "c": 100.0}))
            .collect::<Vec<_>>();
        let payload = json!({
            "bars": {
                "AAPL": provider_bars.clone(),
                "SPY": provider_bars,
            },
            "next_page_token": null,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stock provider");
        let address = listener.local_addr().expect("stock provider address");
        let provider = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v2/stocks/bars", get(bars))
                    .with_state(payload),
            )
            .await
            .expect("serve stock provider");
        });

        let universe = core
            .store_stock_universe(StoreStockUniverseInput {
                name: "Cutoff S&P fixture".to_owned(),
                effective_date: confirmed_sessions[0],
                source_url: "https://example.test/constituents.csv".to_owned(),
                source_revision: "cutoff-fixture".to_owned(),
                source_sha256: "e".repeat(64),
                license_name: "PDDL-1.0".to_owned(),
                license_url: "https://opendatacommons.org/licenses/pddl/1-0/".to_owned(),
                attribution_text: "Cutoff test fixture".to_owned(),
                members: vec![StockUniverseMemberInput {
                    ticker: "AAPL".to_owned(),
                    display_name: "Apple".to_owned(),
                    sector: Some("Technology".to_owned()),
                    sub_industry: None,
                }],
            })
            .expect("store cutoff universe");
        let origin = format!("http://{address}");
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let data = StockDataClient::new(config.clone()).expect("stock data client");
        let claim = SchedulerClaim {
            run_id: "fresh-screen-cutoff".to_owned(),
            job_key: "stock.daily_screen".to_owned(),
            job_kind: "stock.daily_screen".to_owned(),
            scheduled_for: scheduled_for.to_rfc3339(),
            idempotency_key: format!("stock:{}", scheduled_for.to_rfc3339()),
            attempt_number: 1,
            max_attempts: 5,
            worker_id: "cutoff-worker".to_owned(),
            lease_expires_at: (scheduled_for + chrono::Duration::minutes(5)).to_rfc3339(),
        };

        let result = run_stock_screen(&core, &data, &config, &OpenAiClient::disabled(), &claim)
            .await
            .expect("run cutoff screen");
        assert_eq!(result["status"], "no_candidates");
        let completed = core
            .get_stock_screen_run(&claim.run_id)
            .expect("read cutoff screen")
            .expect("completed cutoff screen");
        assert_eq!(completed.market_date, target_market_date);
        assert_eq!(
            completed.universe_snapshot_id.as_deref(),
            Some(universe.id.as_str())
        );
        assert_eq!(
            core.latest_stock_market_session()
                .expect("latest stored market session"),
            Some(target_market_date)
        );

        provider.abort();
    }

    #[tokio::test]
    async fn started_screen_recovery_keeps_its_pinned_market_date_and_universe() {
        async fn bars(State(payload): State<Value>) -> Json<Value> {
            Json(payload)
        }

        let session_dates = weekday_sessions(23);
        let provider_bars = session_dates
            .iter()
            .map(|date| json!({"t": format!("{date}T20:00:00Z"), "c": 100.0}))
            .collect::<Vec<_>>();
        let spy_bars = provider_bars.clone();
        let payload = json!({
            "bars": {
                "AAPL": provider_bars,
                "SPY": spy_bars,
            },
            "next_page_token": null,
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stock provider");
        let address = listener.local_addr().expect("stock provider address");
        let provider = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route("/v2/stocks/bars", get(bars))
                    .with_state(payload),
            )
            .await
            .expect("serve stock provider");
        });

        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let universe = core
            .store_stock_universe(StoreStockUniverseInput {
                name: "Pinned S&P fixture".to_owned(),
                effective_date: session_dates[0],
                source_url: "https://example.test/constituents.csv".to_owned(),
                source_revision: "fixture-revision".to_owned(),
                source_sha256: "a".repeat(64),
                license_name: "PDDL-1.0".to_owned(),
                license_url: "https://opendatacommons.org/licenses/pddl/1-0/".to_owned(),
                attribution_text: "Recovery test fixture".to_owned(),
                members: vec![StockUniverseMemberInput {
                    ticker: "AAPL".to_owned(),
                    display_name: "Apple".to_owned(),
                    sector: Some("Technology".to_owned()),
                    sub_industry: None,
                }],
            })
            .expect("store pinned universe");
        let pinned_market_date = session_dates[21];
        let run_id = "started-before-market-advanced";
        core.begin_stock_screen_run(StockScreenStartInput {
            run_id: Some(run_id.to_owned()),
            market_date: pinned_market_date,
            universe_snapshot_id: Some(universe.id.clone()),
        })
        .expect("persist started screen");

        let origin = format!("http://{address}");
        let config = StockConfig::for_test(
            &format!("{origin}/v2/"),
            &format!("{origin}/constituents.csv"),
        );
        let data = StockDataClient::new(config.clone()).expect("stock data client");
        let now = Utc::now();
        let claim = SchedulerClaim {
            run_id: run_id.to_owned(),
            job_key: "stock.daily_screen".to_owned(),
            job_kind: "stock.daily_screen".to_owned(),
            scheduled_for: now.to_rfc3339(),
            idempotency_key: format!("stock:{}", now.to_rfc3339()),
            attempt_number: 2,
            max_attempts: 5,
            worker_id: "recovery-worker".to_owned(),
            lease_expires_at: (now + chrono::Duration::minutes(5)).to_rfc3339(),
        };
        let result = run_stock_screen(&core, &data, &config, &OpenAiClient::disabled(), &claim)
            .await
            .expect("recover pinned screen");
        assert_eq!(result["status"], "no_candidates");
        let recovered = core
            .get_stock_screen_run(run_id)
            .expect("read recovered run")
            .expect("recovered run");
        assert_eq!(recovered.market_date, pinned_market_date);
        assert_eq!(
            recovered.universe_snapshot_id.as_deref(),
            Some(universe.id.as_str())
        );
        assert_eq!(recovered.status, "no_candidates");

        provider.abort();
    }

    #[tokio::test]
    async fn ai_crash_before_call_releases_reservation_without_retrying() {
        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let openai = OpenAiClient::disabled();
        let report = seed_ai_report(
            &core,
            "screen-before-call",
            "stock-ai-screen-before-call",
            "stock:screen-before-call",
            &openai,
        );

        let outcome = reconcile_stock_ai_report(&core, &openai, report, "stock:screen-before-call")
            .await
            .expect("reconcile pre-call crash");
        assert_eq!(outcome.status, "failed");
        assert_eq!(outcome.cost_microusd, 0);
        assert_eq!(outcome.openai_calls, 0);
        let settlement = core
            .get_ai_budget_settlement("stock:screen-before-call")
            .expect("read settlement")
            .expect("settled reservation");
        assert_eq!(settlement.actual_cost_microusd, 0);
        assert_eq!(settlement.outcome, "preflight_failed");
    }

    #[tokio::test]
    async fn ai_crash_after_call_marker_reserves_maximum_without_retrying() {
        let directory = tempfile::tempdir().expect("temporary TM home");
        let core = TmCore::open(TmHome::new(directory.path())).expect("open core");
        let openai = OpenAiClient::disabled();
        let report = seed_ai_report(
            &core,
            "screen-after-call",
            "stock-ai-screen-after-call",
            "stock:screen-after-call",
            &openai,
        );
        assert!(
            core.try_mark_stock_ai_report_calling(&report.id)
                .expect("mark AI call")
        );
        let calling = core
            .get_stock_ai_report(&report.id)
            .expect("read calling report")
            .expect("calling report");

        let outcome = reconcile_stock_ai_report(&core, &openai, calling, "stock:screen-after-call")
            .await
            .expect("reconcile post-call crash");
        assert_eq!(outcome.status, "uncertain");
        assert_eq!(outcome.cost_microusd, STOCK_AI_MAXIMUM_COST_MICROUSD);
        assert_eq!(outcome.openai_calls, 0);
        let settlement = core
            .get_ai_budget_settlement("stock:screen-after-call")
            .expect("read settlement")
            .expect("settled reservation");
        assert_eq!(
            settlement.actual_cost_microusd,
            STOCK_AI_MAXIMUM_COST_MICROUSD
        );
        assert_eq!(settlement.outcome, "upstream_cost_estimate");
    }
}
