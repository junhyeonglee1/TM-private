#!/bin/sh
set -u

: "${TM_SERVER_HOME:?TM_SERVER_HOME must be set}"

status_directory="$TM_SERVER_HOME/backups/remote"
database="$TM_SERVER_HOME/data/tm.sqlite3"
mkdir -p "$status_directory"

write_failure_status() {
    status_tmp="$status_directory/status.json.tmp"
    jq -n \
        --arg checkedAt "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        '{status:"failed",checkedAt:$checkedAt,reason:"backup_job_failed",retryAfterSeconds:300}' \
        > "$status_tmp"
    chmod 0600 "$status_tmp"
    mv "$status_tmp" "$status_directory/status.json"
}

sleep 60
while :; do
    today="$(date -u +%Y-%m-%d)"
    last_success=""
    last_success_schema=""
    current_schema=""
    if [ -r "$status_directory/last-success-date" ]; then
        last_success="$(sed -n '1p' "$status_directory/last-success-date")"
    fi
    if [ -r "$status_directory/status.json" ]; then
        last_success_schema="$(jq -r '.schemaVersion // empty' "$status_directory/status.json" 2>/dev/null || true)"
    fi
    if [ -s "$database" ]; then
        current_schema="$(sqlite3 -readonly "$database" 'PRAGMA user_version;' 2>/dev/null || true)"
    fi

    if [ "$last_success" = "$today" ] \
        && [ -n "$current_schema" ] \
        && [ "$last_success_schema" = "$current_schema" ]; then
        sleep 3600
        continue
    fi

    if /usr/local/bin/railway-backup-once; then
        sleep 3600
    else
        write_failure_status
        echo "event=tm_backup_retry_scheduled retry_after_seconds=300" >&2
        sleep 300
    fi
done
