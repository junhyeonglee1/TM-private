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
if [ "$schema_version" -lt 1 ] || [ "$schema_version" -gt 14 ]; then
    echo "event=tm_backup_failed reason=unsupported_schema_version" >&2
    exit 1
fi
migration_summary="$(
    sqlite3 -readonly "$snapshot" \
        "SELECT COUNT(*) || '|' || COALESCE(MIN(version), 0) || '|' ||
                COALESCE(MAX(version), 0) || '|' || COUNT(DISTINCT version)
         FROM schema_migrations;"
)"
expected_migration_summary="$schema_version|1|$schema_version|$schema_version"
if [ "$migration_summary" != "$expected_migration_summary" ]; then
    echo "event=tm_backup_failed reason=incomplete_migration_ledger" >&2
    exit 1
fi
if [ "$schema_version" -ge 13 ]; then
    required_tables='schema_migrations
projects
tasks
checklist_items
tags
task_tags
task_day_entries
task_events
work_sessions
session_tasks
worklogs
notes
calendar_events
stock_watchlist_items
stock_universe_snapshots
stock_universe_members
stock_market_data_batches
stock_market_sessions
stock_daily_bars
stock_screen_runs
stock_screen_results
stock_ai_reports
task_report_runs
task_report_feedback
entity_links
attachments
digest_deliveries
change_requests
change_request_events
mutation_idempotency_records
mutation_audit_events
ai_budget_ledger
assistant_action_requests
assistant_action_events
assistant_memories
assistant_memory_sources
assistant_memory_events
scheduler_jobs
scheduler_runs
scheduler_attempts
scheduler_effects
device_pairings
registered_devices
device_auth_events
app_state'
    for table in $required_tables; do
        table_exists="$(
            sqlite3 -readonly "$snapshot" \
                "SELECT COUNT(*) FROM sqlite_schema
                 WHERE type = 'table' AND name = '$table';"
        )"
        if [ "$table_exists" != "1" ]; then
            echo "event=tm_backup_failed reason=required_table_missing table=$table" >&2
            exit 1
        fi
    done
fi
schema_semantics_validated=null
if [ "$schema_version" -ge 14 ]; then
    schema_semantic_summary="$(
        sqlite3 -readonly "$snapshot" "
            WITH objects AS (
                SELECT type,
                       name,
                       lower(
                           replace(replace(replace(replace(sql, ' ', ''), char(9), ''), char(10), ''), char(13), '')
                       ) AS normalized_sql
                FROM sqlite_schema
                WHERE sql IS NOT NULL
            )
            SELECT
                (SELECT COUNT(*) FROM projects WHERE system_key = 'uncategorized') || '|' ||
                (SELECT COUNT(*) FROM projects
                 WHERE system_key = 'uncategorized'
                   AND name = '기타'
                   AND archived_at IS NULL
                   AND deleted_at IS NULL) || '|' ||
                (SELECT COUNT(*) FROM projects
                 WHERE system_key IS NOT NULL AND system_key <> 'uncategorized') || '|' ||
                (SELECT COUNT(*)
                 FROM tasks
                 LEFT JOIN projects ON projects.id = tasks.project_id
                 WHERE tasks.project_id IS NULL OR projects.id IS NULL) || '|' ||
                (SELECT COUNT(*) FROM objects
                 WHERE type = 'table'
                   AND name = 'projects'
                   AND normalized_sql LIKE '%system_keytext%'
                   AND normalized_sql LIKE '%check(system_keyisnullorsystem_key=''uncategorized'')%') || '|' ||
                (SELECT COUNT(*) FROM objects
                 WHERE type = 'index'
                   AND name = 'idx_projects_system_key'
                   AND normalized_sql LIKE '%createuniqueindexidx_projects_system_key%'
                   AND normalized_sql LIKE '%onprojects(system_key)%'
                   AND normalized_sql LIKE '%wheresystem_keyisnotnull%') || '|' ||
                (SELECT COUNT(*) FROM objects
                 WHERE type = 'trigger'
                   AND (
                       (name = 'tasks_project_required_insert'
                        AND normalized_sql LIKE '%beforeinsertontasks%'
                        AND normalized_sql LIKE '%new.project_idisnull%'
                        AND normalized_sql LIKE '%raise(abort,''taskprojectisrequired'')%')
                       OR
                       (name = 'tasks_project_required_update'
                        AND normalized_sql LIKE '%beforeupdateofproject_idontasks%'
                        AND normalized_sql LIKE '%new.project_idisnull%'
                        AND normalized_sql LIKE '%raise(abort,''taskprojectisrequired'')%')
                       OR
                       (name = 'projects_uncategorized_protect_update'
                        AND normalized_sql LIKE '%beforeupdateonprojects%'
                        AND normalized_sql LIKE '%old.system_key=''uncategorized''%'
                        AND normalized_sql LIKE '%new.system_keyisnotold.system_key%'
                        AND normalized_sql LIKE '%new.nameisnotold.name%'
                        AND normalized_sql LIKE '%new.archived_atisnotold.archived_at%'
                        AND normalized_sql LIKE '%new.deleted_atisnotold.deleted_at%'
                        AND normalized_sql LIKE '%raise(abort,''uncategorizedprojectidentityandactivestateareimmutable'')%')
                       OR
                       (name = 'projects_uncategorized_protect_delete'
                        AND normalized_sql LIKE '%beforedeleteonprojects%'
                        AND normalized_sql LIKE '%old.system_key=''uncategorized''%'
                        AND normalized_sql LIKE '%raise(abort,''uncategorizedprojectcannotbedeleted'')%')
                       OR
                       (name = 'projects_uncategorized_name_reserved_insert'
                        AND normalized_sql LIKE '%beforeinsertonprojects%'
                        AND normalized_sql LIKE '%new.system_keyisnull%'
                        AND normalized_sql LIKE '%trim(new.name)=''기타''%'
                        AND normalized_sql LIKE '%raise(abort,''projectname기타isreserved'')%')
                       OR
                       (name = 'projects_uncategorized_name_reserved_update'
                        AND normalized_sql LIKE '%beforeupdateofname,system_keyonprojects%'
                        AND normalized_sql LIKE '%new.system_keyisnull%'
                        AND normalized_sql LIKE '%trim(new.name)=''기타''%'
                        AND normalized_sql LIKE '%trim(old.name)<>''기타''%'
                        AND normalized_sql LIKE '%raise(abort,''projectname기타isreserved'')%')
                   ));"
    )"
    if [ "$schema_semantic_summary" != "1|1|0|0|1|1|6" ]; then
        echo "event=tm_backup_failed reason=schema_semantics summary=$schema_semantic_summary" >&2
        exit 1
    fi
    schema_semantics_validated=true
fi

created_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
database_sha256="$(sha256sum "$snapshot" | awk '{print $1}')"
database_bytes="$(wc -c < "$snapshot" | tr -d ' ')"
jq -n \
    --arg createdAt "$created_at" \
    --arg sha256 "$database_sha256" \
    --argjson byteSize "$database_bytes" \
    --argjson schemaVersion "$schema_version" \
    --argjson migrationLedgerComplete true \
    --argjson requiredTablesComplete true \
    --argjson schemaSemanticsValidated "$schema_semantics_validated" \
    '{createdAt:$createdAt,sha256:$sha256,byteSize:$byteSize,schemaVersion:$schemaVersion,migrationLedgerComplete:$migrationLedgerComplete,requiredTablesComplete:$requiredTablesComplete,schemaSemanticsValidated:$schemaSemanticsValidated}' \
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
    --argjson migrationLedgerComplete true \
    --argjson requiredTablesComplete true \
    --argjson schemaSemanticsValidated "$schema_semantics_validated" \
    '{status:"succeeded",checkedAt:$checkedAt,snapshotId:$snapshotId,databaseSha256:$sha256,databaseByteSize:$byteSize,schemaVersion:$schemaVersion,retention:{daily:7,weekly:4,monthly:12},integrityCheck:"ok",migrationLedgerComplete:$migrationLedgerComplete,requiredTablesComplete:$requiredTablesComplete,schemaSemanticsValidated:$schemaSemanticsValidated}' \
    > "$status_tmp"
chmod 0600 "$status_tmp"
mv "$status_tmp" "$status_directory/status.json"
printf '%s\n' "$(date -u +%Y-%m-%d)" > "$status_directory/last-success-date"
chmod 0600 "$status_directory/last-success-date"
echo "event=tm_backup_succeeded schema_version=$schema_version byte_size=$database_bytes snapshot_id=$snapshot_id"
