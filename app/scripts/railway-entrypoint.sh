#!/bin/sh
set -eu

: "${TM_SERVER_HOME:?TM_SERVER_HOME must be set}"

mkdir -p "$TM_SERVER_HOME"

if [ "${TM_BACKUP_ENABLED:-false}" = "true" ]; then
    mkdir -p "$TM_SERVER_HOME/backups/remote"
fi

# The backup directory is created as root before the application starts. On a
# fresh volume this also creates its parent as root, which prevents the tm user
# from creating the sibling backup directories required by TmHome.
chown -R tm:tm "$TM_SERVER_HOME"

if [ "${TM_BACKUP_ENABLED:-false}" = "true" ]; then
    gosu tm:tm /usr/local/bin/railway-backup-loop &
fi

exec gosu tm:tm /usr/local/bin/tm-server
