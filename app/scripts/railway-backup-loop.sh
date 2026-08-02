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
    last_probe_present=""
    last_probe_sha256=""
    current_schema=""
    current_probe_present=""
    current_probe_sha256=""
    probe_state_matches=false
    if [ -r "$status_directory/last-success-date" ]; then
        last_success="$(sed -n '1p' "$status_directory/last-success-date")"
    fi
    if [ -r "$status_directory/status.json" ]; then
        last_success_schema="$(jq -r '.schemaVersion // empty' "$status_directory/status.json" 2>/dev/null || true)"
        last_probe_present="$(jq -r 'if .expenseCryptoProbePresent == null then "" else (.expenseCryptoProbePresent|tostring) end' "$status_directory/status.json" 2>/dev/null || true)"
        last_probe_sha256="$(jq -r '.expenseCryptoProbeSha256 // empty' "$status_directory/status.json" 2>/dev/null || true)"
    fi
    if [ -s "$database" ]; then
        current_schema="$(sqlite3 -readonly "$database" 'PRAGMA user_version;' 2>/dev/null || true)"
        if [ "$current_schema" = "15" ]; then
            current_probe_count="$(
                sqlite3 -readonly "$database" \
                    "SELECT COUNT(*) FROM expense_crypto_metadata
                     WHERE singleton_key = 'expense-data-key-probe' AND key_version = 1;" \
                    2>/dev/null || true
            )"
            if [ "$current_probe_count" = "0" ]; then
                current_probe_present=false
            elif [ "$current_probe_count" = "1" ]; then
                current_probe_present=true
                current_probe_sha256="$(
                    sqlite3 -readonly -noheader "$database" \
                        "SELECT singleton_key || '|' || key_version || '|' || nonce || '|' ||
                                ciphertext || '|' || aad
                         FROM expense_crypto_metadata
                         WHERE singleton_key = 'expense-data-key-probe' AND key_version = 1;" \
                        2>/dev/null |
                        tr -d '\r\n' |
                        sha256sum |
                        awk '{print $1}'
                )"
            fi
        fi
    fi
    if [ -n "$current_schema" ] && [ "$current_schema" -lt 15 ] 2>/dev/null; then
        probe_state_matches=true
    elif [ "$current_schema" = "15" ] \
        && [ -n "$current_probe_present" ] \
        && [ "$last_probe_present" = "$current_probe_present" ] \
        && [ "$last_probe_sha256" = "$current_probe_sha256" ]; then
        probe_state_matches=true
    fi

    if [ "$last_success" = "$today" ] \
        && [ -n "$current_schema" ] \
        && [ "$last_success_schema" = "$current_schema" ] \
        && [ "$probe_state_matches" = "true" ]; then
        sleep 300
        continue
    fi

    if /usr/local/bin/railway-backup-once; then
        sleep 60
    else
        write_failure_status
        echo "event=tm_backup_retry_scheduled retry_after_seconds=300" >&2
        sleep 300
    fi
done
