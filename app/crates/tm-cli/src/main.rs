use std::io;
use std::process::ExitCode;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::Parser;
use serde_json::Value;
use tm_cli::{Cli, CommandService, DigestKind, execute};
use tm_core::TmCore;

struct CoreService {
    core: TmCore,
}

impl CoreService {
    fn open() -> Result<Self> {
        Ok(Self {
            core: TmCore::open_from_env().context("failed to open TM data store")?,
        })
    }
}

impl CommandService for CoreService {
    fn digest_prepare(&mut self, kind: DigestKind, date: NaiveDate) -> Result<Value> {
        let kind = match kind {
            DigestKind::Morning => tm_core::DigestKind::Morning,
            DigestKind::Evening => tm_core::DigestKind::Evening,
        };
        let preparation = self
            .core
            .prepare_digest(kind, date)
            .context("failed to prepare digest")?;
        serde_json::to_value(preparation).context("failed to serialize digest preparation")
    }

    fn digest_complete(&mut self, delivery_key: &str, slack_ref: &str) -> Result<Value> {
        let delivery = self
            .core
            .complete_digest(delivery_key, slack_ref)
            .context("failed to complete digest delivery")?;
        serde_json::to_value(delivery).context("failed to serialize digest delivery")
    }

    fn digest_fail(&mut self, delivery_key: &str, reason: &str) -> Result<Value> {
        let delivery = self
            .core
            .fail_digest(delivery_key, reason)
            .context("failed to mark digest delivery as failed")?;
        serde_json::to_value(delivery).context("failed to serialize digest delivery")
    }

    fn digest_status(&mut self) -> Result<Value> {
        let deliveries = self
            .core
            .digest_status()
            .context("failed to read digest status")?;
        serde_json::to_value(deliveries).context("failed to serialize digest status")
    }

    fn changes_prepare(&mut self, worker: &str) -> Result<Value> {
        let request = self
            .core
            .claim_next_change_request(worker)
            .context("failed to claim the next change request")?;
        serde_json::to_value(request).context("failed to serialize change request claim")
    }

    fn changes_complete(
        &mut self,
        request_id: &str,
        claim_key: &str,
        summary: &str,
        patch_ref: &str,
        worker: &str,
    ) -> Result<Value> {
        let request = self
            .core
            .complete_change_request(request_id, claim_key, summary, Some(patch_ref), worker)
            .context("failed to complete change request")?;
        serde_json::to_value(request).context("failed to serialize completed change request")
    }

    fn changes_fail(
        &mut self,
        request_id: &str,
        claim_key: &str,
        reason: &str,
        worker: &str,
    ) -> Result<Value> {
        let request = self
            .core
            .fail_change_request(request_id, claim_key, reason, worker)
            .context("failed to mark change request as failed")?;
        serde_json::to_value(request).context("failed to serialize failed change request")
    }

    fn changes_status(&mut self) -> Result<Value> {
        let requests = self
            .core
            .list_change_requests()
            .context("failed to list change requests")?;
        serde_json::to_value(requests).context("failed to serialize change request status")
    }

    fn backup_create(&mut self) -> Result<Value> {
        let artifact = self
            .core
            .create_backup()
            .context("failed to create database backup")?;
        serde_json::to_value(artifact).context("failed to serialize backup result")
    }

    fn backup_source(&mut self) -> Result<Value> {
        let artifact = self
            .core
            .create_source_snapshot()
            .context("failed to create source snapshot")?;
        serde_json::to_value(artifact).context("failed to serialize source snapshot result")
    }

    fn migration_manifest(&mut self) -> Result<Value> {
        let manifest = self
            .core
            .migration_manifest()
            .context("failed to build migration manifest")?;
        serde_json::to_value(manifest).context("failed to serialize migration manifest")
    }

    fn migration_dry_run(&mut self) -> Result<Value> {
        let result = self
            .core
            .migration_dry_run()
            .context("failed to run migration dry-run")?;
        serde_json::to_value(result).context("failed to serialize migration dry-run")
    }

    fn migration_inspect(&mut self, path: &std::path::Path) -> Result<Value> {
        let manifest = TmCore::inspect_migration_database(path)
            .context("failed to inspect migration database")?;
        serde_json::to_value(manifest).context("failed to serialize inspected manifest")
    }

    fn health(&mut self) -> Result<Value> {
        let health = self.core.health().context("health check failed")?;
        serde_json::to_value(health).context("failed to serialize health report")
    }

    fn export(&mut self) -> Result<Value> {
        self.core.export_json().context("JSON export failed")
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut service = CoreService::open()?;
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    execute(cli, &mut service, &mut stdout)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tm-cli: {error:#}");
            ExitCode::FAILURE
        }
    }
}
