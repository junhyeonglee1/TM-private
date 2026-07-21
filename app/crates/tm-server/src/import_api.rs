use std::{env, fs, io::Write, process::Command};

use axum::{
    Extension, Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderName, StatusCode},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tm_core::{Error as CoreError, MigrationManifest, TmCore};
use uuid::Uuid;

use super::{ApiEnvelope, ApiError, AppState, RequestId};

const CONFIRM_IMPORT_HEADER: HeaderName = HeaderName::from_static("x-tm-confirm-import");
const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ImportResult {
    imported: bool,
    byte_size: usize,
    file_sha256: String,
    pre_import_backup_file: String,
    manifest: MigrationManifest,
}

pub(super) fn body_limit() -> axum::extract::DefaultBodyLimit {
    axum::extract::DefaultBodyLimit::max(MAX_IMPORT_BYTES)
}

pub(super) async fn import_database(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ApiEnvelope<ImportResult>>, ApiError> {
    if body.is_empty() {
        return Err(import_error(
            StatusCode::BAD_REQUEST,
            "IMPORT_DATABASE_EMPTY",
            "database import body is empty",
            &request_id,
        ));
    }
    let expected_logical_sha256 = headers
        .get(&CONFIRM_IMPORT_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| {
            import_error(
                StatusCode::PRECONDITION_REQUIRED,
                "IMPORT_CONFIRMATION_REQUIRED",
                "x-tm-confirm-import must contain the source logical SHA-256",
                &request_id,
            )
        })?
        .to_owned();
    let byte_size = body.len();
    let file_sha256 = format!("{:x}", Sha256::digest(&body));
    let error_request_id = request_id.0.clone();
    let result = tokio::task::spawn_blocking(move || {
        perform_import(
            &state.core,
            &body,
            &expected_logical_sha256,
            byte_size,
            file_sha256,
        )
    })
    .await
    .map_err(|_| ApiError {
        status: StatusCode::SERVICE_UNAVAILABLE,
        code: "IMPORT_WORKER_FAILED",
        message: "database import worker failed".to_owned(),
        request_id: error_request_id.clone(),
    })?
    .map_err(|error| map_import_error(error, error_request_id))?;

    tracing::info!(
        event = "tm_database_import_succeeded",
        request_id = %request_id.0,
        byte_size = result.byte_size,
        schema_version = result.manifest.schema_version,
        logical_sha256 = %result.manifest.logical_sha256,
    );
    Ok(Json(ApiEnvelope {
        request_id: request_id.0,
        data: result,
    }))
}

fn perform_import(
    core: &TmCore,
    body: &[u8],
    expected_logical_sha256: &str,
    byte_size: usize,
    file_sha256: String,
) -> tm_core::Result<ImportResult> {
    let import_dir = core.home().data_dir().join("imports");
    fs::create_dir_all(&import_dir)?;
    let candidate_path = import_dir.join(format!("{}.sqlite3", Uuid::now_v7()));
    let result = (|| {
        let mut candidate = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate_path)?;
        candidate.write_all(body)?;
        candidate.sync_all()?;
        drop(candidate);

        let source_manifest = TmCore::inspect_migration_database(&candidate_path)?;
        if source_manifest.logical_sha256 != expected_logical_sha256 {
            return Err(CoreError::Conflict(
                "source manifest does not match import confirmation".to_owned(),
            ));
        }
        let pre_import_backup = core.create_backup()?;
        core.restore_backup(&candidate_path)?;
        let active_manifest = core.migration_manifest()?;
        if !source_manifest.logically_matches(&active_manifest) {
            tracing::error!(
                event = "tm_database_import_manifest_mismatch",
                source_logical_sha256 = %source_manifest.logical_sha256,
                active_logical_sha256 = %active_manifest.logical_sha256,
                "rolling back database import"
            );
            core.restore_backup(&pre_import_backup.path)?;
            return Err(CoreError::Invariant(
                "active database manifest did not match the import source".to_owned(),
            ));
        }
        if let Err(error) = create_remote_backup_if_enabled() {
            tracing::error!(
                event = "tm_database_import_remote_backup_failed",
                "rolling back database import after remote backup failure"
            );
            core.restore_backup(&pre_import_backup.path)?;
            return Err(error);
        }
        let pre_import_backup_file = std::path::Path::new(&pre_import_backup.path)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                CoreError::Invariant("pre-import backup filename is invalid".to_owned())
            })?
            .to_owned();
        Ok(ImportResult {
            imported: true,
            byte_size,
            file_sha256,
            pre_import_backup_file,
            manifest: active_manifest,
        })
    })();
    if let Err(error) = fs::remove_file(&candidate_path)
        && result.is_ok()
    {
        return Err(error.into());
    }
    result
}

fn create_remote_backup_if_enabled() -> tm_core::Result<()> {
    if env::var("TM_BACKUP_ENABLED").as_deref() != Ok("true") {
        return Ok(());
    }
    let status = Command::new("/usr/local/bin/railway-backup-once").status()?;
    if !status.success() {
        return Err(CoreError::Io(std::io::Error::other(
            "encrypted remote backup failed after database import",
        )));
    }
    Ok(())
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn map_import_error(error: CoreError, request_id: String) -> ApiError {
    match error {
        CoreError::InvalidBackup(_) | CoreError::InvalidInput(_) | CoreError::Invariant(_) => {
            ApiError {
                status: StatusCode::BAD_REQUEST,
                code: "IMPORT_DATABASE_REJECTED",
                message: "database import failed validation and was not activated".to_owned(),
                request_id,
            }
        }
        CoreError::Conflict(_) => ApiError {
            status: StatusCode::CONFLICT,
            code: "IMPORT_CONFIRMATION_MISMATCH",
            message: "database import confirmation did not match the source".to_owned(),
            request_id,
        },
        _ => {
            tracing::error!(%request_id, "database import failed internally");
            ApiError {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                code: "IMPORT_DATABASE_FAILED",
                message: "database import failed without activating the candidate".to_owned(),
                request_id,
            }
        }
    }
}

fn import_error(
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: &RequestId,
) -> ApiError {
    ApiError {
        status,
        code,
        message: message.to_owned(),
        request_id: request_id.0.clone(),
    }
}
