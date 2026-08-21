use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::{
    Error, Result, TmCore,
    database::new_id,
    error::{invalid, not_found},
};

pub const DEVICE_PAIRING_TTL_SECONDS: i64 = 600;
pub const DEVICE_SESSION_TTL_DAYS: i64 = 90;
pub const DEVICE_LAST_SEEN_WRITE_SECONDS: i64 = 3_600;
const MAX_PENDING_PAIRINGS: i64 = 3;
const MAX_PAIRING_REQUESTS_PER_HOUR: i64 = 10;
const MAX_DEVICE_LABEL_CHARS: usize = 80;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DevicePairing {
    pub id: String,
    pub device_label: String,
    pub status: String,
    pub requested_at: String,
    pub expires_at: String,
    pub approved_at: Option<String>,
    pub completed_at: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredDevice {
    pub id: String,
    pub label: String,
    pub status: String,
    pub created_at: String,
    pub last_seen_at: String,
    pub expires_at: String,
    pub revoked_at: Option<String>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSessionAuthentication {
    pub device: RegisteredDevice,
    pub csrf_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePairingSecrets {
    pub pairing: DevicePairing,
    pub code: String,
    pub polling_secret: String,
}

impl TmCore {
    pub fn start_device_pairing(
        &self,
        device_label: &str,
        code: &str,
        code_sha256: &str,
        polling_secret: &str,
        polling_sha256: &str,
        as_of: DateTime<Utc>,
    ) -> Result<DevicePairingSecrets> {
        let label = validate_device_label(device_label)?;
        validate_code(code)?;
        validate_sha256("pairing code hash", code_sha256)?;
        validate_secret("polling secret", polling_secret)?;
        validate_sha256("polling secret hash", polling_sha256)?;
        let now = timestamp(as_of);
        let expires_at = timestamp(as_of + Duration::seconds(DEVICE_PAIRING_TTL_SECONDS));
        let id = new_id();
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                expire_pairings(transaction, as_of)?;
                let pending: i64 = transaction.query_row(
                    "SELECT count(*) FROM device_pairings WHERE status IN ('pending', 'approved')",
                    [],
                    |row| row.get(0),
                )?;
                if pending >= MAX_PENDING_PAIRINGS {
                    return Err(Error::Conflict(
                        "too many device pairings are already pending".to_owned(),
                    ));
                }
                let hour_start = timestamp(as_of - Duration::hours(1));
                let recent: i64 = transaction.query_row(
                    "SELECT count(*) FROM device_auth_events
                 WHERE event_type = 'pairing_requested' AND created_at >= ?1",
                    [&hour_start],
                    |row| row.get(0),
                )?;
                if recent >= MAX_PAIRING_REQUESTS_PER_HOUR {
                    return Err(Error::Conflict(
                        "device pairing request limit was reached".to_owned(),
                    ));
                }
                transaction.execute(
                    "INSERT INTO device_pairings(
                    id, device_label, code_sha256, polling_sha256, status,
                    requested_at, expires_at, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?5, ?5)",
                    params![id, label, code_sha256, polling_sha256, now, expires_at],
                )?;
                insert_event(
                    transaction,
                    Some(&id),
                    None,
                    "pairing_requested",
                    "mobile",
                    &now,
                )?;
                query_pairing(transaction, &id)
            })
            .map(|pairing| DevicePairingSecrets {
                pairing,
                code: code.to_owned(),
                polling_secret: polling_secret.to_owned(),
            })
    }

    pub fn list_device_pairings(&self, as_of: DateTime<Utc>) -> Result<Vec<DevicePairing>> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                expire_pairings(transaction, as_of)?;
                let mut statement = transaction.prepare(
                    "SELECT id, device_label, status, requested_at, expires_at,
                        approved_at, completed_at, revision
                 FROM device_pairings
                 WHERE status IN ('pending', 'approved')
                 ORDER BY requested_at DESC, id DESC",
                )?;
                let rows = statement.query_map([], map_pairing)?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })
    }

    pub fn approve_device_pairing(
        &self,
        pairing_id: &str,
        code_sha256: &str,
        actor: &str,
        as_of: DateTime<Utc>,
    ) -> Result<DevicePairing> {
        validate_sha256("pairing code hash", code_sha256)?;
        let now = timestamp(as_of);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                expire_pairings(transaction, as_of)?;
                let expected: Option<(String, String)> = transaction
                    .query_row(
                        "SELECT code_sha256, status FROM device_pairings WHERE id = ?1",
                        [pairing_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((expected_hash, status)) = expected else {
                    return Err(not_found("device pairing", pairing_id));
                };
                if status != "pending" {
                    return Err(Error::Conflict("device pairing is not pending".to_owned()));
                }
                if !constant_time_text_equal(&expected_hash, code_sha256) {
                    return Err(invalid("device pairing code did not match"));
                }
                transaction.execute(
                    "UPDATE device_pairings
                 SET status = 'approved', approved_at = ?2, approved_by = ?3,
                     updated_at = ?2, revision = revision + 1
                 WHERE id = ?1 AND status = 'pending'",
                    params![pairing_id, now, actor],
                )?;
                insert_event(
                    transaction,
                    Some(pairing_id),
                    None,
                    "pairing_approved",
                    actor,
                    &now,
                )?;
                query_pairing(transaction, pairing_id)
            })
    }

    pub fn complete_device_pairing(
        &self,
        pairing_id: &str,
        polling_sha256: &str,
        token_sha256: &str,
        csrf_sha256: &str,
        as_of: DateTime<Utc>,
    ) -> Result<RegisteredDevice> {
        validate_sha256("polling secret hash", polling_sha256)?;
        validate_sha256("device token hash", token_sha256)?;
        validate_sha256("CSRF token hash", csrf_sha256)?;
        let now = timestamp(as_of);
        let expires_at = timestamp(as_of + Duration::days(DEVICE_SESSION_TTL_DAYS));
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                expire_pairings(transaction, as_of)?;
                let pairing: Option<(String, String, String)> = transaction
                    .query_row(
                        "SELECT device_label, polling_sha256, status
                     FROM device_pairings WHERE id = ?1",
                        [pairing_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let Some((label, expected_polling_hash, status)) = pairing else {
                    return Err(not_found("device pairing", pairing_id));
                };
                if status != "approved" {
                    return Err(Error::Conflict("device pairing is not approved".to_owned()));
                }
                if !constant_time_text_equal(&expected_polling_hash, polling_sha256) {
                    return Err(invalid("device pairing secret did not match"));
                }
                let device_id = new_id();
                transaction.execute(
                    "INSERT INTO registered_devices(
                    id, pairing_id, label, token_sha256, csrf_sha256, status,
                    created_at, last_seen_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'active', ?6, ?6, ?7)",
                    params![
                        device_id,
                        pairing_id,
                        label,
                        token_sha256,
                        csrf_sha256,
                        now,
                        expires_at
                    ],
                )?;
                transaction.execute(
                    "UPDATE device_pairings
                 SET status = 'completed', completed_at = ?2, updated_at = ?2,
                     revision = revision + 1
                 WHERE id = ?1 AND status = 'approved'",
                    params![pairing_id, now],
                )?;
                insert_event(
                    transaction,
                    Some(pairing_id),
                    Some(&device_id),
                    "pairing_completed",
                    "mobile",
                    &now,
                )?;
                query_device(transaction, &device_id)
            })
    }

    pub fn authenticate_device_session(
        &self,
        token_sha256: &str,
        as_of: DateTime<Utc>,
    ) -> Result<Option<DeviceSessionAuthentication>> {
        validate_sha256("device token hash", token_sha256)?;
        let now = timestamp(as_of);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let row: Option<(RegisteredDevice, String)> = transaction
                    .query_row(
                        "SELECT id, label, status, created_at, last_seen_at, expires_at,
                            revoked_at, revision, csrf_sha256
                     FROM registered_devices WHERE token_sha256 = ?1",
                        [token_sha256],
                        |row| Ok((map_device(row)?, row.get(8)?)),
                    )
                    .optional()?;
                let Some((mut device, csrf_sha256)) = row else {
                    return Ok(None);
                };
                if device.status == "active" && device.expires_at <= now {
                    transaction.execute(
                        "UPDATE registered_devices
                     SET status = 'expired', revision = revision + 1
                     WHERE id = ?1 AND status = 'active'",
                        [&device.id],
                    )?;
                    insert_event(
                        transaction,
                        None,
                        Some(&device.id),
                        "device_expired",
                        "system",
                        &now,
                    )?;
                    return Ok(None);
                }
                if device.status != "active" {
                    return Ok(None);
                }
                let last_seen = DateTime::parse_from_rfc3339(&device.last_seen_at)
                    .map_err(|_| Error::Invariant("invalid device last_seen_at".to_owned()))?
                    .with_timezone(&Utc);
                if as_of - last_seen >= Duration::seconds(DEVICE_LAST_SEEN_WRITE_SECONDS) {
                    transaction.execute(
                        "UPDATE registered_devices SET last_seen_at = ?2 WHERE id = ?1",
                        params![device.id, now],
                    )?;
                    device.last_seen_at = now;
                }
                Ok(Some(DeviceSessionAuthentication {
                    device,
                    csrf_sha256,
                }))
            })
    }

    pub fn list_registered_devices(&self, as_of: DateTime<Utc>) -> Result<Vec<RegisteredDevice>> {
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                expire_devices(transaction, as_of)?;
                let mut statement = transaction.prepare(
                    "SELECT id, label, status, created_at, last_seen_at, expires_at,
                        revoked_at, revision
                 FROM registered_devices ORDER BY created_at DESC, id DESC",
                )?;
                let rows = statement.query_map([], map_device)?;
                Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
            })
    }

    pub fn get_registered_device(&self, device_id: &str) -> Result<RegisteredDevice> {
        let connection = self.database.connect()?;
        query_device(&connection, device_id)
    }

    pub fn revoke_registered_device(
        &self,
        device_id: &str,
        actor: &str,
        event_type: &str,
        as_of: DateTime<Utc>,
    ) -> Result<RegisteredDevice> {
        if !matches!(event_type, "device_revoked" | "device_logout") {
            return Err(invalid("invalid device revocation event"));
        }
        let now = timestamp(as_of);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let changed = transaction.execute(
                    "UPDATE registered_devices
                 SET status = 'revoked', revoked_at = ?2, revoked_by = ?3,
                     revision = revision + 1
                 WHERE id = ?1 AND status = 'active'",
                    params![device_id, now, actor],
                )?;
                if changed == 0 {
                    let existing: Option<String> = transaction
                        .query_row(
                            "SELECT status FROM registered_devices WHERE id = ?1",
                            [device_id],
                            |row| row.get(0),
                        )
                        .optional()?;
                    return match existing {
                        None => Err(not_found("registered device", device_id)),
                        Some(_) => Err(Error::Conflict("device is not active".to_owned())),
                    };
                }
                insert_event(transaction, None, Some(device_id), event_type, actor, &now)?;
                query_device(transaction, device_id)
            })
    }

    pub fn revoke_all_registered_devices(
        &self,
        actor: &str,
        as_of: DateTime<Utc>,
    ) -> Result<usize> {
        let now = timestamp(as_of);
        self.database
            .transaction(TransactionBehavior::Immediate, |transaction| {
                let active_ids = {
                    let mut statement = transaction.prepare(
                        "SELECT id FROM registered_devices WHERE status = 'active' ORDER BY id",
                    )?;
                    statement
                        .query_map([], |row| row.get::<_, String>(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                for id in &active_ids {
                    transaction.execute(
                        "UPDATE registered_devices
                     SET status = 'revoked', revoked_at = ?2, revoked_by = ?3,
                         revision = revision + 1
                     WHERE id = ?1 AND status = 'active'",
                        params![id, now, actor],
                    )?;
                    insert_event(transaction, None, Some(id), "device_revoked", actor, &now)?;
                }
                Ok(active_ids.len())
            })
    }
}

fn expire_pairings(transaction: &Transaction<'_>, as_of: DateTime<Utc>) -> Result<()> {
    let now = timestamp(as_of);
    let ids = {
        let mut statement = transaction.prepare(
            "SELECT id FROM device_pairings
             WHERE status IN ('pending', 'approved') AND expires_at <= ?1",
        )?;
        statement
            .query_map([&now], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for id in ids {
        transaction.execute(
            "UPDATE device_pairings
             SET status = 'expired', updated_at = ?2, revision = revision + 1
             WHERE id = ?1 AND status IN ('pending', 'approved')",
            params![id, now],
        )?;
        insert_event(
            transaction,
            Some(&id),
            None,
            "pairing_expired",
            "system",
            &now,
        )?;
    }
    Ok(())
}

fn expire_devices(transaction: &Transaction<'_>, as_of: DateTime<Utc>) -> Result<()> {
    let now = timestamp(as_of);
    let ids = {
        let mut statement = transaction.prepare(
            "SELECT id FROM registered_devices WHERE status = 'active' AND expires_at <= ?1",
        )?;
        statement
            .query_map([&now], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for id in ids {
        transaction.execute(
            "UPDATE registered_devices SET status = 'expired', revision = revision + 1
             WHERE id = ?1 AND status = 'active'",
            [&id],
        )?;
        insert_event(
            transaction,
            None,
            Some(&id),
            "device_expired",
            "system",
            &now,
        )?;
    }
    Ok(())
}

fn insert_event(
    transaction: &Transaction<'_>,
    pairing_id: Option<&str>,
    device_id: Option<&str>,
    event_type: &str,
    actor: &str,
    created_at: &str,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO device_auth_events(
            id, pairing_id, device_id, event_type, actor, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            new_id(),
            pairing_id,
            device_id,
            event_type,
            actor,
            created_at
        ],
    )?;
    Ok(())
}

fn query_pairing(connection: &Connection, id: &str) -> Result<DevicePairing> {
    connection
        .query_row(
            "SELECT id, device_label, status, requested_at, expires_at,
                    approved_at, completed_at, revision
             FROM device_pairings WHERE id = ?1",
            [id],
            map_pairing,
        )
        .map_err(Into::into)
}

fn query_device(connection: &Connection, id: &str) -> Result<RegisteredDevice> {
    connection
        .query_row(
            "SELECT id, label, status, created_at, last_seen_at, expires_at,
                    revoked_at, revision
             FROM registered_devices WHERE id = ?1",
            [id],
            map_device,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => not_found("registered device", id),
            other => other.into(),
        })
}

fn map_pairing(row: &Row<'_>) -> rusqlite::Result<DevicePairing> {
    Ok(DevicePairing {
        id: row.get(0)?,
        device_label: row.get(1)?,
        status: row.get(2)?,
        requested_at: row.get(3)?,
        expires_at: row.get(4)?,
        approved_at: row.get(5)?,
        completed_at: row.get(6)?,
        revision: row.get(7)?,
    })
}

fn map_device(row: &Row<'_>) -> rusqlite::Result<RegisteredDevice> {
    Ok(RegisteredDevice {
        id: row.get(0)?,
        label: row.get(1)?,
        status: row.get(2)?,
        created_at: row.get(3)?,
        last_seen_at: row.get(4)?,
        expires_at: row.get(5)?,
        revoked_at: row.get(6)?,
        revision: row.get(7)?,
    })
}

fn validate_device_label(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_DEVICE_LABEL_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(invalid(
            "device label must contain 1 to 80 visible characters",
        ));
    }
    Ok(trimmed.to_owned())
}

fn validate_code(value: &str) -> Result<()> {
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid("device pairing code must be exactly six digits"));
    }
    Ok(())
}

fn validate_secret(name: &str, value: &str) -> Result<()> {
    if value.len() < 43
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(invalid(format!("{name} has an invalid format")));
    }
    Ok(())
}

fn validate_sha256(name: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(format!("{name} must be a SHA-256 hex digest")));
    }
    Ok(())
}

fn constant_time_text_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}
