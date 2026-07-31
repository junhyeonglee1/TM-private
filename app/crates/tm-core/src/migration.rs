use std::{collections::BTreeMap, path::Path};

use rusqlite::{Connection, OpenFlags, TransactionBehavior, types::ValueRef};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    BackupArtifact, Error, Result, backup,
    database::{Database, SCHEMA_VERSION, now_utc, validate_schema_semantics},
    export::EXPORTED_TABLES,
};

const MANIFEST_FORMAT: &str = "tm-migration-manifest";
const MANIFEST_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationTableManifest {
    pub row_count: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationManifest {
    pub format: String,
    pub format_version: u32,
    pub schema_version: i64,
    pub migration_versions: Vec<i64>,
    pub generated_at: String,
    pub tables: BTreeMap<String, MigrationTableManifest>,
    pub logical_sha256: String,
}

impl MigrationManifest {
    #[must_use]
    pub fn logically_matches(&self, other: &Self) -> bool {
        self.format == other.format
            && self.format_version == other.format_version
            && self.schema_version == other.schema_version
            && self.migration_versions == other.migration_versions
            && self.tables == other.tables
            && self.logical_sha256 == other.logical_sha256
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationDryRun {
    pub source: MigrationManifest,
    pub snapshot: MigrationManifest,
    pub snapshot_artifact: BackupArtifact,
    pub verified: bool,
}

pub(crate) fn manifest(database: &Database) -> Result<MigrationManifest> {
    database.transaction(TransactionBehavior::Deferred, |transaction| {
        manifest_for_connection(transaction)
    })
}

pub(crate) fn inspect_database(path: &Path) -> Result<MigrationManifest> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|_| Error::InvalidBackup(path.to_path_buf()))?;
    manifest_for_connection(&connection).map_err(|_| Error::InvalidBackup(path.to_path_buf()))
}

pub(crate) fn dry_run(database: &Database) -> Result<MigrationDryRun> {
    let source = manifest(database)?;
    let snapshot_artifact = backup::create_database_backup(
        &database.home().database_path(),
        &database.home().database_backups_dir(),
        "migration-dry-run",
    )?;
    let snapshot = inspect_database(Path::new(&snapshot_artifact.path))?;
    if !source.logically_matches(&snapshot) {
        return Err(Error::Invariant(
            "migration dry-run snapshot does not match the source database".to_owned(),
        ));
    }
    Ok(MigrationDryRun {
        source,
        snapshot,
        snapshot_artifact,
        verified: true,
    })
}

fn manifest_for_connection(connection: &Connection) -> Result<MigrationManifest> {
    validate_connection(connection)?;
    let schema_version = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let migration_versions = migration_versions(connection)?;
    let mut tables = BTreeMap::new();
    for table in EXPORTED_TABLES {
        tables.insert((*table).to_owned(), table_manifest(connection, table)?);
    }
    let logical_sha256 = logical_manifest_hash(schema_version, &migration_versions, &tables);
    Ok(MigrationManifest {
        format: MANIFEST_FORMAT.to_owned(),
        format_version: MANIFEST_FORMAT_VERSION,
        schema_version,
        migration_versions,
        generated_at: now_utc(),
        tables,
        logical_sha256,
    })
}

fn validate_connection(connection: &Connection) -> Result<()> {
    let schema_version: i64 =
        connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if schema_version != SCHEMA_VERSION {
        return Err(Error::Invariant(format!(
            "migration requires schema {SCHEMA_VERSION}, found {schema_version}"
        )));
    }
    validate_schema_semantics(connection, schema_version)?;
    let integrity: String = connection.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if integrity != "ok" {
        return Err(Error::Invariant(format!(
            "database integrity check failed: {integrity}"
        )));
    }
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if rows.next()?.is_some() {
        return Err(Error::Invariant(
            "database contains foreign key violations".to_owned(),
        ));
    }
    Ok(())
}

fn migration_versions(connection: &Connection) -> Result<Vec<i64>> {
    let mut statement =
        connection.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
    let versions = statement
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let expected = (1..=SCHEMA_VERSION).collect::<Vec<_>>();
    if versions != expected {
        return Err(Error::Invariant(format!(
            "schema migration ledger mismatch: expected {expected:?}, found {versions:?}"
        )));
    }
    Ok(versions)
}

fn table_manifest(connection: &Connection, table: &str) -> Result<MigrationTableManifest> {
    let pragma = format!("PRAGMA table_info(\"{table}\")");
    let mut column_statement = connection.prepare(&pragma)?;
    let columns = column_statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        return Err(Error::Invariant(format!(
            "migration table is missing or has no columns: {table}"
        )));
    }

    let query = format!("SELECT * FROM \"{table}\"");
    let mut statement = connection.prepare(&query)?;
    let mut rows = statement.query([])?;
    let mut canonical_rows = Vec::new();
    while let Some(row) = rows.next()? {
        let mut canonical_row = Vec::new();
        for (index, column) in columns.iter().enumerate() {
            push_bytes(&mut canonical_row, column.as_bytes());
            push_value(&mut canonical_row, row.get_ref(index)?);
        }
        canonical_rows.push(canonical_row);
    }
    canonical_rows.sort_unstable();

    let mut hasher = Sha256::new();
    hasher.update(b"tm-table-manifest-v1\0");
    for row in &canonical_rows {
        hasher.update((row.len() as u64).to_be_bytes());
        hasher.update(row);
    }
    Ok(MigrationTableManifest {
        row_count: canonical_rows.len() as u64,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn logical_manifest_hash(
    schema_version: i64,
    migration_versions: &[i64],
    tables: &BTreeMap<String, MigrationTableManifest>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"tm-migration-manifest-v1\0");
    hasher.update(schema_version.to_be_bytes());
    for version in migration_versions {
        hasher.update(version.to_be_bytes());
    }
    for (table, manifest) in tables {
        hasher.update((table.len() as u64).to_be_bytes());
        hasher.update(table.as_bytes());
        hasher.update(manifest.row_count.to_be_bytes());
        hasher.update(manifest.sha256.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn push_value(output: &mut Vec<u8>, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => output.push(0),
        ValueRef::Integer(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_be_bytes());
        }
        ValueRef::Real(value) => {
            output.push(2);
            output.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        ValueRef::Text(value) => {
            output.push(3);
            push_bytes(output, value);
        }
        ValueRef::Blob(value) => {
            output.push(4);
            push_bytes(output, value);
        }
    }
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u64).to_be_bytes());
    output.extend_from_slice(value);
}
