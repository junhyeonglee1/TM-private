#!/bin/sh
set -eu

: "${TM_SERVER_HOME:?TM_SERVER_HOME must be set}"
: "${TM_BACKUP_S3_ENDPOINT:?TM_BACKUP_S3_ENDPOINT must be set}"
: "${TM_BACKUP_S3_BUCKET:?TM_BACKUP_S3_BUCKET must be set}"
: "${TM_BACKUP_S3_ACCESS_KEY_ID:?TM_BACKUP_S3_ACCESS_KEY_ID must be set}"
: "${TM_BACKUP_S3_SECRET_ACCESS_KEY:?TM_BACKUP_S3_SECRET_ACCESS_KEY must be set}"
: "${TM_BACKUP_S3_REGION:?TM_BACKUP_S3_REGION must be set}"
: "${TM_BACKUP_REPOSITORY_PASSWORD:?TM_BACKUP_REPOSITORY_PASSWORD must be set}"

prefix="${TM_BACKUP_REPOSITORY_PREFIX:-tm-production}"
case "$prefix" in
    *[!a-zA-Z0-9._/-]*|'')
        echo "event=tm_backup_failed reason=invalid_repository_prefix" >&2
        exit 1
        ;;
esac

database="$TM_SERVER_HOME/data/tm.sqlite3"
if [ ! -s "$database" ]; then
    echo "event=tm_backup_skipped reason=database_missing" >&2
    exit 1
fi

status_directory="$TM_SERVER_HOME/backups/remote"
mkdir -p "$status_directory"
work_directory="$(mktemp -d /tmp/tm-remote-backup.XXXXXX)"
trap 'rm -rf "$work_directory"' EXIT HUP INT TERM
snapshot_directory="$work_directory/snapshot"
snapshot="$snapshot_directory/tm.sqlite3"
manifest="$snapshot_directory/manifest.json"
mkdir -p "$snapshot_directory"

sqlite3 "$database" ".timeout 15000" ".backup '$snapshot'"
integrity="$(sqlite3 -readonly "$snapshot" 'PRAGMA integrity_check;')"
if [ "$integrity" != "ok" ]; then
    echo "event=tm_backup_failed reason=sqlite_integrity" >&2
    exit 1
fi
if [ -n "$(sqlite3 -readonly "$snapshot" 'PRAGMA foreign_key_check;')" ]; then
    echo "event=tm_backup_failed reason=sqlite_foreign_key" >&2
    exit 1
fi
schema_version="$(sqlite3 -readonly "$snapshot" 'PRAGMA user_version;')"
case "$schema_version" in
    ''|*[!0-9]*)
        echo "event=tm_backup_failed reason=invalid_schema_version" >&2
        exit 1
        ;;
esac
if [ "$schema_version" -lt 1 ] || [ "$schema_version" -gt 5 ]; then
    echo "event=tm_backup_failed reason=unsupported_schema_version" >&2
    exit 1
fi

created_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
database_sha256="$(sha256sum "$snapshot" | awk '{print $1}')"
database_bytes="$(wc -c < "$snapshot" | tr -d ' ')"
jq -n \
    --arg createdAt "$created_at" \
    --arg sha256 "$database_sha256" \
    --argjson byteSize "$database_bytes" \
    --argjson schemaVersion "$schema_version" \
    '{createdAt:$createdAt,sha256:$sha256,byteSize:$byteSize,schemaVersion:$schemaVersion}' \
    > "$manifest"

endpoint="${TM_BACKUP_S3_ENDPOINT%/}"
export AWS_ACCESS_KEY_ID="$TM_BACKUP_S3_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$TM_BACKUP_S3_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION="$TM_BACKUP_S3_REGION"
export RESTIC_PASSWORD="$TM_BACKUP_REPOSITORY_PASSWORD"
export RESTIC_REPOSITORY="s3:$endpoint/$TM_BACKUP_S3_BUCKET/$prefix"

restic_command() {
    restic -o s3.bucket-lookup=dns "$@"
}

if ! restic_command cat config >/dev/null 2>&1; then
    if ! restic_command init >/dev/null; then
        restic_command cat config >/dev/null
    fi
fi

backup_output="$work_directory/restic-backup.jsonl"
restic_command backup --json --tag tm-sqlite "$snapshot_directory" > "$backup_output"
snapshot_id="$(
    jq -r 'select(.message_type == "summary") | .snapshot_id // empty' "$backup_output" |
        tail -n 1
)"
if [ -z "$snapshot_id" ]; then
    echo "event=tm_backup_failed reason=snapshot_id_missing" >&2
    exit 1
fi

restic_command forget \
    --tag tm-sqlite \
    --keep-daily 7 \
    --keep-weekly 4 \
    --keep-monthly 12 \
    --prune >/dev/null
restic_command check --read-data >/dev/null

status_tmp="$status_directory/status.json.tmp"
jq -n \
    --arg checkedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    --arg snapshotId "$snapshot_id" \
    --arg sha256 "$database_sha256" \
    --argjson byteSize "$database_bytes" \
    --argjson schemaVersion "$schema_version" \
    '{status:"succeeded",checkedAt:$checkedAt,snapshotId:$snapshotId,databaseSha256:$sha256,databaseByteSize:$byteSize,schemaVersion:$schemaVersion,retention:{daily:7,weekly:4,monthly:12},integrityCheck:"ok"}' \
    > "$status_tmp"
chmod 0600 "$status_tmp"
mv "$status_tmp" "$status_directory/status.json"
printf '%s\n' "$(date -u +%Y-%m-%d)" > "$status_directory/last-success-date"
chmod 0600 "$status_directory/last-success-date"
echo "event=tm_backup_succeeded schema_version=$schema_version byte_size=$database_bytes snapshot_id=$snapshot_id"
