use chrono::{Duration, Utc};
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempDir};
use tm_core::{DEFAULT_TM_HOME, Error, Result, TmCore, TmHome};

fn fixture() -> Result<(TempDir, TmCore)> {
    let test_runs = std::path::Path::new(DEFAULT_TM_HOME)
        .join("dist")
        .join("test-runs");
    std::fs::create_dir_all(&test_runs)?;
    let temporary = Builder::new()
        .prefix("tm-device-auth-")
        .tempdir_in(test_runs)?;
    let core = TmCore::open(TmHome::new(temporary.path()))?;
    Ok((temporary, core))
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn register(
    core: &TmCore,
    as_of: chrono::DateTime<Utc>,
    polling_char: char,
) -> Result<(String, String)> {
    let code = "123456";
    let polling = format!("poll_{}", polling_char.to_string().repeat(64));
    let pairing = core.start_device_pairing(
        "My phone",
        code,
        &hash(code),
        &polling,
        &hash(&polling),
        as_of,
    )?;
    core.approve_device_pairing(&pairing.pairing.id, &hash(code), "primary-admin", as_of)?;
    let token_char = char::from_u32(u32::from(polling_char) + 1).unwrap_or('B');
    let csrf_char = char::from_u32(u32::from(polling_char) + 2).unwrap_or('C');
    let token = format!("tm_dev_v1_{}", token_char.to_string().repeat(64));
    let csrf = format!("tm_csrf_v1_{}", csrf_char.to_string().repeat(64));
    let device = core.complete_device_pairing(
        &pairing.pairing.id,
        &hash(&polling),
        &hash(&token),
        &hash(&csrf),
        as_of,
    )?;
    Ok((device.id, token))
}

#[test]
fn pairing_requires_the_exact_code_and_completes_only_once() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let now = Utc::now();
    let code = "654321";
    let polling = format!("poll_{}", "D".repeat(64));
    let pairing = core.start_device_pairing(
        "  Personal iPhone  ",
        code,
        &hash(code),
        &polling,
        &hash(&polling),
        now,
    )?;
    assert_eq!(pairing.pairing.device_label, "Personal iPhone");
    assert!(matches!(
        core.approve_device_pairing(&pairing.pairing.id, &hash("000000"), "primary-admin", now),
        Err(Error::InvalidInput(_))
    ));
    core.approve_device_pairing(&pairing.pairing.id, &hash(code), "primary-admin", now)?;
    let token = format!("tm_dev_v1_{}", "E".repeat(64));
    let csrf = format!("tm_csrf_v1_{}", "F".repeat(64));
    let device = core.complete_device_pairing(
        &pairing.pairing.id,
        &hash(&polling),
        &hash(&token),
        &hash(&csrf),
        now,
    )?;
    assert_eq!(device.status, "active");
    assert!(matches!(
        core.complete_device_pairing(
            &pairing.pairing.id,
            &hash(&polling),
            &hash(&format!("tm_dev_v1_{}", "1".repeat(64))),
            &hash(&csrf),
            now,
        ),
        Err(Error::Conflict(_))
    ));
    let serialized = serde_json::to_string(&core.list_registered_devices(now)?)?;
    assert!(!serialized.contains(&token));
    assert!(!serialized.contains(&hash(&token)));
    assert!(!serialized.contains(&csrf));
    Ok(())
}

#[test]
fn revocation_and_expiry_take_effect_on_the_next_authentication() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let now = Utc::now();
    let (device_id, token) = register(&core, now, 'A')?;
    assert!(
        core.authenticate_device_session(&hash(&token), now)?
            .is_some()
    );
    core.revoke_registered_device(&device_id, "primary-admin", "device_revoked", now)?;
    assert!(
        core.authenticate_device_session(&hash(&token), now + Duration::seconds(1))?
            .is_none()
    );

    let (other_id, other_token) = register(&core, now + Duration::seconds(2), 'D')?;
    assert!(
        core.authenticate_device_session(&hash(&other_token), now + Duration::days(91))?
            .is_none()
    );
    assert_eq!(core.get_registered_device(&other_id)?.status, "expired");
    Ok(())
}

#[test]
fn backup_restore_cannot_revive_a_revoked_device() -> Result<()> {
    let (_temporary, core) = fixture()?;
    let now = Utc::now();
    let (device_id, token) = register(&core, now, 'A')?;
    let active_backup = core.create_backup()?;
    core.revoke_registered_device(
        &device_id,
        "primary-admin",
        "device_revoked",
        now + Duration::seconds(1),
    )?;

    core.restore_backup(&active_backup.path)?;
    assert_eq!(core.get_registered_device(&device_id)?.status, "revoked");
    assert!(
        core.authenticate_device_session(&hash(&token), now + Duration::seconds(2))?
            .is_none()
    );
    Ok(())
}
