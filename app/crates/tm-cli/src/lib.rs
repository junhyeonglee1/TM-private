use std::{
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::Value;

/// Command-line interface for the TM local-first data store.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "tm-cli", version, about = "TM local task-management CLI")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Commands {
    /// Prepare or update Slack digest delivery state.
    Digest(DigestArgs),
    /// Claim and resolve manually approved change requests.
    Changes(ChangesArgs),
    /// Create a point-in-time database backup.
    Backup(BackupArgs),
    /// Prepare and inspect data for a one-time cloud cutover.
    Migration(MigrationArgs),
    /// Check paths, database access, and schema state.
    Health(JsonArgs),
    /// Export the complete data model as JSON.
    Export(JsonArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct DigestArgs {
    #[command(subcommand)]
    pub command: DigestCommands,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum DigestCommands {
    /// Atomically claim and prepare facts for a digest delivery.
    Prepare(DigestPrepareArgs),
    /// Mark a claimed digest as delivered.
    Complete(DigestCompleteArgs),
    /// Mark a claimed digest attempt as failed.
    Fail(DigestFailArgs),
    /// Show digest delivery state.
    Status(JsonArgs),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum DigestKind {
    Morning,
    Evening,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct DigestPrepareArgs {
    #[arg(long, value_enum)]
    pub kind: DigestKind,
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub date: NaiveDate,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct DigestCompleteArgs {
    #[arg(long, value_name = "KEY")]
    pub delivery_key: String,
    #[arg(long, value_name = "REF")]
    pub slack_ref: String,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct DigestFailArgs {
    #[arg(long, value_name = "KEY")]
    pub delivery_key: String,
    #[arg(long, value_name = "TEXT")]
    pub reason: String,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChangesArgs {
    #[command(subcommand)]
    pub command: ChangesCommands,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ChangesCommands {
    /// Atomically claim the next pending change request.
    Prepare(ChangesPrepareArgs),
    /// Complete a claimed change request.
    Complete(ChangesCompleteArgs),
    /// Mark a claimed change request as failed.
    Fail(ChangesFailArgs),
    /// Show change request state.
    Status(ChangesStatusArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChangesPrepareArgs {
    #[arg(long, required = true)]
    pub json: bool,
    #[arg(long, value_name = "ID", default_value = "codex-local")]
    pub worker: String,
}

#[derive(Clone, Copy, Debug, Args, PartialEq, Eq)]
pub struct ChangesStatusArgs {
    #[arg(long, required = true)]
    pub json: bool,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChangesCompleteArgs {
    #[arg(long, value_name = "ID")]
    pub request_id: String,
    #[arg(long, value_name = "KEY")]
    pub claim_key: String,
    #[arg(long, value_name = "TEXT")]
    pub summary: String,
    #[arg(long, value_name = "REF")]
    pub patch_ref: String,
    #[arg(long, value_name = "ID", default_value = "codex-local")]
    pub worker: String,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ChangesFailArgs {
    #[arg(long, value_name = "ID")]
    pub request_id: String,
    #[arg(long, value_name = "KEY")]
    pub claim_key: String,
    #[arg(long, value_name = "TEXT")]
    pub reason: String,
    #[arg(long, value_name = "ID", default_value = "codex-local")]
    pub worker: String,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommands,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum BackupCommands {
    /// Create a new timestamped database backup.
    Create,
    /// Create a new timestamped source ZIP snapshot.
    Source,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct MigrationArgs {
    #[command(subcommand)]
    pub command: MigrationCommands,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum MigrationCommands {
    /// Build a content-free logical manifest for the current database.
    Manifest(JsonArgs),
    /// Create a consistent snapshot and verify it against the current database.
    DryRun(JsonArgs),
    /// Inspect a candidate SQLite snapshot without modifying it.
    Inspect(MigrationInspectArgs),
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct MigrationInspectArgs {
    #[arg(long, value_name = "PATH")]
    pub path: PathBuf,
    #[arg(long)]
    pub json: bool,
}

#[derive(Clone, Copy, Debug, Args, PartialEq, Eq)]
pub struct JsonArgs {
    #[arg(long)]
    pub json: bool,
}

/// Small boundary kept in this crate so argument and stdout behavior can be
/// tested without opening a real database.
pub trait CommandService {
    fn digest_prepare(&mut self, kind: DigestKind, date: NaiveDate) -> Result<Value>;
    fn digest_complete(&mut self, delivery_key: &str, slack_ref: &str) -> Result<Value>;
    fn digest_fail(&mut self, delivery_key: &str, reason: &str) -> Result<Value>;
    fn digest_status(&mut self) -> Result<Value>;
    fn changes_prepare(&mut self, worker: &str) -> Result<Value>;
    fn changes_complete(
        &mut self,
        request_id: &str,
        claim_key: &str,
        summary: &str,
        patch_ref: &str,
        worker: &str,
    ) -> Result<Value>;
    fn changes_fail(
        &mut self,
        request_id: &str,
        claim_key: &str,
        reason: &str,
        worker: &str,
    ) -> Result<Value>;
    fn changes_status(&mut self) -> Result<Value>;
    fn backup_create(&mut self) -> Result<Value>;
    fn backup_source(&mut self) -> Result<Value>;
    fn migration_manifest(&mut self) -> Result<Value>;
    fn migration_dry_run(&mut self) -> Result<Value>;
    fn migration_inspect(&mut self, path: &Path) -> Result<Value>;
    fn health(&mut self) -> Result<Value>;
    fn export(&mut self) -> Result<Value>;
}

/// Execute one parsed command. Successful command output is always exactly one
/// JSON value followed by a newline; diagnostic logging belongs on stderr.
pub fn execute(cli: Cli, service: &mut impl CommandService, stdout: &mut impl Write) -> Result<()> {
    let value = match cli.command {
        Commands::Digest(args) => match args.command {
            DigestCommands::Prepare(args) => service.digest_prepare(args.kind, args.date)?,
            DigestCommands::Complete(args) => {
                service.digest_complete(&args.delivery_key, &args.slack_ref)?
            }
            DigestCommands::Fail(args) => service.digest_fail(&args.delivery_key, &args.reason)?,
            DigestCommands::Status(_) => service.digest_status()?,
        },
        Commands::Changes(args) => match args.command {
            ChangesCommands::Prepare(args) => service.changes_prepare(&args.worker)?,
            ChangesCommands::Complete(args) => service.changes_complete(
                &args.request_id,
                &args.claim_key,
                &args.summary,
                &args.patch_ref,
                &args.worker,
            )?,
            ChangesCommands::Fail(args) => service.changes_fail(
                &args.request_id,
                &args.claim_key,
                &args.reason,
                &args.worker,
            )?,
            ChangesCommands::Status(_) => service.changes_status()?,
        },
        Commands::Backup(args) => match args.command {
            BackupCommands::Create => service.backup_create()?,
            BackupCommands::Source => service.backup_source()?,
        },
        Commands::Migration(args) => match args.command {
            MigrationCommands::Manifest(_) => service.migration_manifest()?,
            MigrationCommands::DryRun(_) => service.migration_dry_run()?,
            MigrationCommands::Inspect(args) => service.migration_inspect(&args.path)?,
        },
        Commands::Health(_) => service.health()?,
        Commands::Export(_) => service.export()?,
    };

    serde_json::to_writer(&mut *stdout, &value)?;
    stdout.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::{Result, bail};
    use chrono::NaiveDate;
    use clap::{CommandFactory, Parser};
    use serde_json::{Value, json};

    use super::{
        BackupCommands, ChangesArgs, ChangesCommands, Cli, CommandService, Commands, DigestArgs,
        DigestCommands, DigestKind, JsonArgs, MigrationArgs, MigrationCommands, execute,
    };

    #[derive(Default)]
    struct FakeService {
        call: Option<String>,
        fail: bool,
    }

    impl FakeService {
        fn response(&mut self, call: String) -> Result<Value> {
            self.call = Some(call.clone());
            if self.fail {
                bail!("test failure");
            }
            Ok(json!({"call": call, "ok": true}))
        }
    }

    impl CommandService for FakeService {
        fn digest_prepare(&mut self, kind: DigestKind, date: NaiveDate) -> Result<Value> {
            self.response(format!("prepare:{kind:?}:{date}"))
        }

        fn digest_complete(&mut self, delivery_key: &str, slack_ref: &str) -> Result<Value> {
            self.response(format!("complete:{delivery_key}:{slack_ref}"))
        }

        fn digest_fail(&mut self, delivery_key: &str, reason: &str) -> Result<Value> {
            self.response(format!("fail:{delivery_key}:{reason}"))
        }

        fn digest_status(&mut self) -> Result<Value> {
            self.response("status".to_owned())
        }

        fn changes_prepare(&mut self, worker: &str) -> Result<Value> {
            self.response(format!("changes:prepare:{worker}"))
        }

        fn changes_complete(
            &mut self,
            request_id: &str,
            claim_key: &str,
            summary: &str,
            patch_ref: &str,
            worker: &str,
        ) -> Result<Value> {
            self.response(format!(
                "changes:complete:{request_id}:{claim_key}:{summary}:{patch_ref}:{worker}"
            ))
        }

        fn changes_fail(
            &mut self,
            request_id: &str,
            claim_key: &str,
            reason: &str,
            worker: &str,
        ) -> Result<Value> {
            self.response(format!(
                "changes:fail:{request_id}:{claim_key}:{reason}:{worker}"
            ))
        }

        fn changes_status(&mut self) -> Result<Value> {
            self.response("changes:status".to_owned())
        }

        fn backup_create(&mut self) -> Result<Value> {
            self.response("backup".to_owned())
        }

        fn backup_source(&mut self) -> Result<Value> {
            self.response("source-backup".to_owned())
        }

        fn migration_manifest(&mut self) -> Result<Value> {
            self.response("migration:manifest".to_owned())
        }

        fn migration_dry_run(&mut self) -> Result<Value> {
            self.response("migration:dry-run".to_owned())
        }

        fn migration_inspect(&mut self, path: &std::path::Path) -> Result<Value> {
            self.response(format!("migration:inspect:{}", path.display()))
        }

        fn health(&mut self) -> Result<Value> {
            self.response("health".to_owned())
        }

        fn export(&mut self) -> Result<Value> {
            self.response("export".to_owned())
        }
    }

    #[test]
    fn clap_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_digest_prepare() -> Result<()> {
        let cli = Cli::try_parse_from([
            "tm-cli",
            "digest",
            "prepare",
            "--kind",
            "morning",
            "--date",
            "2026-07-12",
            "--json",
        ])?;

        let Commands::Digest(DigestArgs {
            command: DigestCommands::Prepare(args),
        }) = cli.command
        else {
            bail!("unexpected command");
        };
        assert_eq!(args.kind, DigestKind::Morning);
        assert_eq!(
            args.date,
            NaiveDate::from_ymd_opt(2026, 7, 12)
                .ok_or_else(|| anyhow::anyhow!("invalid test date"))?
        );
        assert!(args.json);
        Ok(())
    }

    #[test]
    fn rejects_invalid_digest_kind_and_date() {
        let invalid_kind = Cli::try_parse_from([
            "tm-cli",
            "digest",
            "prepare",
            "--kind",
            "night",
            "--date",
            "2026-07-12",
            "--json",
        ]);
        assert!(invalid_kind.is_err());

        let invalid_date = Cli::try_parse_from([
            "tm-cli",
            "digest",
            "prepare",
            "--kind",
            "evening",
            "--date",
            "2026-02-30",
            "--json",
        ]);
        assert!(invalid_date.is_err());
    }

    #[test]
    fn parses_changes_prepare_with_default_worker() -> Result<()> {
        let cli = Cli::try_parse_from(["tm-cli", "changes", "prepare", "--json"])?;

        let Commands::Changes(ChangesArgs {
            command: ChangesCommands::Prepare(args),
        }) = cli.command
        else {
            bail!("unexpected command");
        };
        assert!(args.json);
        assert_eq!(args.worker, "codex-local");
        Ok(())
    }

    #[test]
    fn changes_complete_and_fail_require_all_claim_arguments() {
        assert!(Cli::try_parse_from(["tm-cli", "changes", "prepare"]).is_err());
        assert!(Cli::try_parse_from(["tm-cli", "changes", "status"]).is_err());
        assert!(
            Cli::try_parse_from([
                "tm-cli",
                "changes",
                "complete",
                "--request-id",
                "request-1",
                "--claim-key",
                "claim-1",
                "--summary",
                "done",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "tm-cli",
                "changes",
                "fail",
                "--request-id",
                "request-1",
                "--claim-key",
                "claim-1",
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_changes_transitions_with_default_worker() -> Result<()> {
        let complete = Cli::try_parse_from([
            "tm-cli",
            "changes",
            "complete",
            "--request-id",
            "request-1",
            "--claim-key",
            "claim-1",
            "--summary",
            "implemented",
            "--patch-ref",
            "patch-0003",
        ])?;
        let Commands::Changes(ChangesArgs {
            command: ChangesCommands::Complete(complete),
        }) = complete.command
        else {
            bail!("unexpected complete command");
        };
        assert_eq!(complete.request_id, "request-1");
        assert_eq!(complete.claim_key, "claim-1");
        assert_eq!(complete.summary, "implemented");
        assert_eq!(complete.patch_ref, "patch-0003");
        assert_eq!(complete.worker, "codex-local");

        let failed = Cli::try_parse_from([
            "tm-cli",
            "changes",
            "fail",
            "--request-id",
            "request-2",
            "--claim-key",
            "claim-2",
            "--reason",
            "blocked",
        ])?;
        let Commands::Changes(ChangesArgs {
            command: ChangesCommands::Fail(failed),
        }) = failed.command
        else {
            bail!("unexpected fail command");
        };
        assert_eq!(failed.request_id, "request-2");
        assert_eq!(failed.claim_key, "claim-2");
        assert_eq!(failed.reason, "blocked");
        assert_eq!(failed.worker, "codex-local");
        Ok(())
    }

    #[test]
    fn parses_all_required_command_shapes() {
        let commands: &[&[&str]] = &[
            &[
                "tm-cli",
                "digest",
                "complete",
                "--delivery-key",
                "k",
                "--slack-ref",
                "r",
            ],
            &[
                "tm-cli",
                "digest",
                "fail",
                "--delivery-key",
                "k",
                "--reason",
                "network",
            ],
            &["tm-cli", "digest", "status", "--json"],
            &[
                "tm-cli", "changes", "prepare", "--json", "--worker", "worker-1",
            ],
            &[
                "tm-cli",
                "changes",
                "complete",
                "--request-id",
                "request-1",
                "--claim-key",
                "claim-1",
                "--summary",
                "completed",
                "--patch-ref",
                "patch-0003",
                "--worker",
                "worker-1",
            ],
            &[
                "tm-cli",
                "changes",
                "fail",
                "--request-id",
                "request-1",
                "--claim-key",
                "claim-1",
                "--reason",
                "blocked",
                "--worker",
                "worker-1",
            ],
            &["tm-cli", "changes", "status", "--json"],
            &["tm-cli", "backup", "create"],
            &["tm-cli", "backup", "source"],
            &["tm-cli", "migration", "manifest", "--json"],
            &["tm-cli", "migration", "dry-run", "--json"],
            &[
                "tm-cli",
                "migration",
                "inspect",
                "--path",
                "C:\\tm\\snapshot.sqlite3",
                "--json",
            ],
            &["tm-cli", "health", "--json"],
            &["tm-cli", "export", "--json"],
        ];

        for command in commands {
            assert!(Cli::try_parse_from(*command).is_ok());
        }
    }

    #[test]
    fn execute_writes_only_compact_json_to_stdout() -> Result<()> {
        let cli = Cli {
            command: Commands::Health(JsonArgs { json: true }),
        };
        let mut service = FakeService::default();
        let mut stdout = Vec::new();

        execute(cli, &mut service, &mut stdout)?;

        assert_eq!(stdout, b"{\"call\":\"health\",\"ok\":true}\n");
        assert_eq!(service.call.as_deref(), Some("health"));
        Ok(())
    }

    #[test]
    fn execute_does_not_write_partial_success_on_service_error() {
        let cli = Cli {
            command: Commands::Backup(super::BackupArgs {
                command: BackupCommands::Create,
            }),
        };
        let mut service = FakeService {
            call: None,
            fail: true,
        };
        let mut stdout = Vec::new();

        let result = execute(cli, &mut service, &mut stdout);

        assert!(result.is_err());
        assert!(stdout.is_empty());
        assert_eq!(service.call.as_deref(), Some("backup"));
    }

    #[test]
    fn execute_routes_source_snapshot_and_writes_json() -> Result<()> {
        let cli = Cli {
            command: Commands::Backup(super::BackupArgs {
                command: BackupCommands::Source,
            }),
        };
        let mut service = FakeService::default();
        let mut stdout = Vec::new();

        execute(cli, &mut service, &mut stdout)?;

        assert_eq!(stdout, b"{\"call\":\"source-backup\",\"ok\":true}\n");
        assert_eq!(service.call.as_deref(), Some("source-backup"));
        Ok(())
    }

    #[test]
    fn execute_routes_migration_commands_without_row_contents() -> Result<()> {
        let commands = [
            (
                Cli {
                    command: Commands::Migration(MigrationArgs {
                        command: MigrationCommands::Manifest(JsonArgs { json: true }),
                    }),
                },
                "migration:manifest",
            ),
            (
                Cli {
                    command: Commands::Migration(MigrationArgs {
                        command: MigrationCommands::DryRun(JsonArgs { json: true }),
                    }),
                },
                "migration:dry-run",
            ),
            (
                Cli {
                    command: Commands::Migration(MigrationArgs {
                        command: MigrationCommands::Inspect(super::MigrationInspectArgs {
                            path: std::path::PathBuf::from("snapshot.sqlite3"),
                            json: true,
                        }),
                    }),
                },
                "migration:inspect:snapshot.sqlite3",
            ),
        ];

        for (cli, expected_call) in commands {
            let mut service = FakeService::default();
            let mut stdout = Vec::new();

            execute(cli, &mut service, &mut stdout)?;

            assert_eq!(service.call.as_deref(), Some(expected_call));
            assert_eq!(
                serde_json::from_slice::<Value>(&stdout)?,
                json!({"call": expected_call, "ok": true})
            );
        }
        Ok(())
    }

    #[test]
    fn execute_routes_changes_commands_with_every_argument() -> Result<()> {
        let commands = [
            (
                Cli {
                    command: Commands::Changes(ChangesArgs {
                        command: ChangesCommands::Prepare(super::ChangesPrepareArgs {
                            json: true,
                            worker: "worker-a".to_owned(),
                        }),
                    }),
                },
                "changes:prepare:worker-a",
            ),
            (
                Cli {
                    command: Commands::Changes(ChangesArgs {
                        command: ChangesCommands::Complete(super::ChangesCompleteArgs {
                            request_id: "request-1".to_owned(),
                            claim_key: "claim-1".to_owned(),
                            summary: "implemented".to_owned(),
                            patch_ref: "patch-0003".to_owned(),
                            worker: "worker-b".to_owned(),
                        }),
                    }),
                },
                "changes:complete:request-1:claim-1:implemented:patch-0003:worker-b",
            ),
            (
                Cli {
                    command: Commands::Changes(ChangesArgs {
                        command: ChangesCommands::Fail(super::ChangesFailArgs {
                            request_id: "request-2".to_owned(),
                            claim_key: "claim-2".to_owned(),
                            reason: "blocked".to_owned(),
                            worker: "worker-c".to_owned(),
                        }),
                    }),
                },
                "changes:fail:request-2:claim-2:blocked:worker-c",
            ),
            (
                Cli {
                    command: Commands::Changes(ChangesArgs {
                        command: ChangesCommands::Status(super::ChangesStatusArgs { json: true }),
                    }),
                },
                "changes:status",
            ),
        ];

        for (cli, expected_call) in commands {
            let mut service = FakeService::default();
            let mut stdout = Vec::new();

            execute(cli, &mut service, &mut stdout)?;

            assert_eq!(service.call.as_deref(), Some(expected_call));
            assert_eq!(
                serde_json::from_slice::<Value>(&stdout)?,
                json!({"call": expected_call, "ok": true})
            );
        }
        Ok(())
    }
}
