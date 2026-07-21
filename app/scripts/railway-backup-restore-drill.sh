#!/bin/sh
set -eu

: "${TM_BACKUP_S3_ENDPOINT:?TM_BACKUP_S3_ENDPOINT must be set}"
: "${TM_BACKUP_S3_BUCKET:?TM_BACKUP_S3_BUCKET must be set}"
: "${TM_BACKUP_S3_ACCESS_KEY_ID:?TM_BACKUP_S3_ACCESS_KEY_ID must be set}"
: "${TM_BACKUP_S3_SECRET_ACCESS_KEY:?TM_BACKUP_S3_SECRET_ACCESS_KEY must be set}"
: "${TM_BACKUP_S3_REGION:?TM_BACKUP_S3_REGION must be set}"
: "${TM_BACKUP_REPOSITORY_PASSWORD:?TM_BACKUP_REPOSITORY_PASSWORD must be set}"

prefix="${TM_BACKUP_REPOSITORY_PREFIX:-tm-production}"
case "$prefix" in
    *[!a-zA-Z0-9._/-]*|'')
        echo "event=tm_restore_drill_failed reason=invalid_repository_prefix" >&2
        exit 1
        ;;
esac

endpoint="${TM_BACKUP_S3_ENDPOINT%/}"
export AWS_ACCESS_KEY_ID="$TM_BACKUP_S3_ACCESS_KEY_ID"
export AWS_SECRET_ACCESS_KEY="$TM_BACKUP_S3_SECRET_ACCESS_KEY"
export AWS_DEFAULT_REGION="$TM_BACKUP_S3_REGION"
export RESTIC_PASSWORD="$TM_BACKUP_REPOSITORY_PASSWORD"
export RESTIC_REPOSITORY="s3:$endpoint/$TM_BACKUP_S3_BUCKET/$prefix"

target="$(mktemp -d /tmp/tm-restore-drill.XXXXXX)"
trap 'rm -rf "$target"' EXIT HUP INT TERM
restic -o s3.bucket-lookup=dns restore latest --tag tm-sqlite --target "$target" >/dev/null

database="$(find "$target" -type f -name tm.sqlite3 -print -quit)"
manifest="$(find "$target" -type f -name manifest.json -print -quit)"
if [ -z "$database" ] || [ -z "$manifest" ]; then
    echo "event=tm_restore_drill_failed reason=artifact_missing" >&2
    exit 1
fi

expected_sha256="$(jq -er '.sha256' "$manifest")"
expected_schema="$(jq -er '.schemaVersion' "$manifest")"
actual_sha256="$(sha256sum "$database" | awk '{print $1}')"
actual_schema="$(sqlite3 -readonly "$database" 'PRAGMA user_version;')"
integrity="$(sqlite3 -readonly "$database" 'PRAGMA integrity_check;')"
foreign_keys="$(sqlite3 -readonly "$database" 'PRAGMA foreign_key_check;')"

if [ "$expected_sha256" != "$actual_sha256" ]; then
    echo "event=tm_restore_drill_failed reason=checksum_mismatch" >&2
    exit 1
fi
if [ "$expected_schema" != "$actual_schema" ]; then
    echo "event=tm_restore_drill_failed reason=schema_mismatch" >&2
    exit 1
fi
if [ "$integrity" != "ok" ] || [ -n "$foreign_keys" ]; then
    echo "event=tm_restore_drill_failed reason=database_integrity" >&2
    exit 1
fi

database_bytes="$(wc -c < "$database" | tr -d ' ')"
echo "event=tm_restore_drill_succeeded schema_version=$actual_schema byte_size=$database_bytes sha256=$actual_sha256"
