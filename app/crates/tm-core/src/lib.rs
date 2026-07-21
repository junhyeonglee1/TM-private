//! Local-first domain and persistence core shared by the TM desktop app and CLI.

mod ai_budget;
mod assistant_action;
mod backup;
mod change_request;
mod core;
mod database;
mod desktop_api;
mod digest;
mod error;
mod export;
mod home;
mod memory;
mod migration;
mod model;
mod mutation;
mod scheduler;

pub use assistant_action::*;
pub use backup::BackupArtifact;
pub use core::TmCore;
pub use desktop_api::{DesktopCommand, execute_desktop_command};
pub use digest::{DigestDelivery, DigestFacts, DigestKind, DigestPreparation};
pub use error::{Error, Result};
pub use home::{DEFAULT_TM_HOME, TmHome};
pub use memory::*;
pub use migration::{MigrationDryRun, MigrationManifest, MigrationTableManifest};
pub use model::*;
pub use mutation::*;
pub use scheduler::*;
