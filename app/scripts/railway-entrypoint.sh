#!/bin/sh
set -eu

: "${TM_SERVER_HOME:?TM_SERVER_HOME must be set}"

mkdir -p "$TM_SERVER_HOME"
chown tm:tm "$TM_SERVER_HOME"

exec gosu tm:tm /usr/local/bin/tm-server
