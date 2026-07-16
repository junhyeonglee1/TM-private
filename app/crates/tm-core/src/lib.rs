//! Local-first domain and persistence core shared by the TM desktop app and CLI.

mod backup;
mod change_request;
mod core;
mod database;
mod digest;
mod error;
mod export;
mod home;
mod migration;
mod model;

pub use backup::BackupArtifact;
pub use core::TmCore;
pub use digest::{DigestDelivery, DigestFacts, DigestKind, DigestPreparation};
pub use error::{Error, Result};
pub use home::{DEFAULT_TM_HOME, TmHome};
pub use migration::{MigrationDryRun, MigrationManifest, MigrationTableManifest};
pub use model::*;
