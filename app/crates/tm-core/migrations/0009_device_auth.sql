CREATE TABLE device_pairings (
    id TEXT PRIMARY KEY,
    device_label TEXT NOT NULL,
    code_sha256 TEXT NOT NULL CHECK (length(code_sha256) = 64),
    polling_sha256 TEXT NOT NULL UNIQUE CHECK (length(polling_sha256) = 64),
    status TEXT NOT NULL CHECK (
        status IN ('pending', 'approved', 'completed', 'expired', 'cancelled')
    ),
    requested_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    approved_at TEXT,
    completed_at TEXT,
    approved_by TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0)
) STRICT;

CREATE INDEX idx_device_pairings_status_expiry
    ON device_pairings(status, expires_at, requested_at DESC);

CREATE TABLE registered_devices (
    id TEXT PRIMARY KEY,
    pairing_id TEXT NOT NULL UNIQUE REFERENCES device_pairings(id),
    label TEXT NOT NULL,
    token_sha256 TEXT NOT NULL UNIQUE CHECK (length(token_sha256) = 64),
    csrf_sha256 TEXT NOT NULL CHECK (length(csrf_sha256) = 64),
    status TEXT NOT NULL CHECK (status IN ('active', 'revoked', 'expired')),
    created_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    revoked_at TEXT,
    revoked_by TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    CHECK (
        (status = 'revoked' AND revoked_at IS NOT NULL AND revoked_by IS NOT NULL)
        OR (status <> 'revoked' AND revoked_at IS NULL AND revoked_by IS NULL)
    )
) STRICT;

CREATE INDEX idx_registered_devices_status_expiry
    ON registered_devices(status, expires_at, created_at DESC);

CREATE TABLE device_auth_events (
    id TEXT PRIMARY KEY,
    pairing_id TEXT REFERENCES device_pairings(id),
    device_id TEXT REFERENCES registered_devices(id),
    event_type TEXT NOT NULL CHECK (
        event_type IN (
            'pairing_requested', 'pairing_approved', 'pairing_completed',
            'pairing_expired', 'device_revoked', 'device_expired', 'device_logout'
        )
    ),
    actor TEXT NOT NULL,
    created_at TEXT NOT NULL,
    CHECK (pairing_id IS NOT NULL OR device_id IS NOT NULL)
) STRICT;

CREATE INDEX idx_device_auth_events_created
    ON device_auth_events(created_at DESC, id DESC);

CREATE TRIGGER device_pairings_no_delete
BEFORE DELETE ON device_pairings
BEGIN
    SELECT RAISE(ABORT, 'device pairings cannot be deleted');
END;

CREATE TRIGGER registered_devices_no_delete
BEFORE DELETE ON registered_devices
BEGIN
    SELECT RAISE(ABORT, 'registered devices cannot be deleted');
END;

CREATE TRIGGER registered_devices_identity_immutable
BEFORE UPDATE ON registered_devices
WHEN NEW.pairing_id <> OLD.pairing_id
  OR NEW.label <> OLD.label
  OR NEW.token_sha256 <> OLD.token_sha256
  OR NEW.csrf_sha256 <> OLD.csrf_sha256
  OR NEW.created_at <> OLD.created_at
  OR NEW.expires_at <> OLD.expires_at
BEGIN
    SELECT RAISE(ABORT, 'registered device identity is immutable');
END;

CREATE TRIGGER device_auth_events_no_update
BEFORE UPDATE ON device_auth_events
BEGIN
    SELECT RAISE(ABORT, 'device auth events are append-only');
END;

CREATE TRIGGER device_auth_events_no_delete
BEFORE DELETE ON device_auth_events
BEGIN
    SELECT RAISE(ABORT, 'device auth events cannot be deleted');
END;
