use std::time::Duration;

use chrono::Utc;
use tm_core::{SCHEDULER_POLL_SECONDS, TmCore};
use tokio::{task::JoinHandle, time::MissedTickBehavior};
use uuid::Uuid;

pub fn spawn(core: TmCore) -> JoinHandle<()> {
    let worker_id = format!("tm-server-{}", Uuid::now_v7());
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(SCHEDULER_POLL_SECONDS));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let cycle_core = core.clone();
            let cycle_worker_id = worker_id.clone();
            match tokio::task::spawn_blocking(move || {
                cycle_core.run_scheduler_cycle(&cycle_worker_id, Utc::now())
            })
            .await
            {
                Ok(Ok(report)) => {
                    if report.scheduled > 0
                        || report.skipped > 0
                        || report.lease_recoveries > 0
                        || report.succeeded > 0
                        || report.failed > 0
                    {
                        tracing::info!(
                            worker_id,
                            scheduled = report.scheduled,
                            skipped = report.skipped,
                            lease_recoveries = report.lease_recoveries,
                            succeeded = report.succeeded,
                            failed = report.failed,
                            "scheduler cycle completed"
                        );
                    }
                }
                Ok(Err(error)) => {
                    tracing::error!(worker_id, %error, "scheduler cycle failed");
                }
                Err(error) => {
                    tracing::error!(worker_id, %error, "scheduler worker task failed");
                }
            }
        }
    })
}
