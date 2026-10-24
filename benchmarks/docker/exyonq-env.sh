#!/usr/bin/env bash
# Resolve EXYONQ_ACCEPT_WORKERS=auto to nproc (Linux bench containers).
# Product defaults already use available_parallelism when unset; this only
# expands the explicit "auto" token for shell/compose wrappers.
set -eu
if [ "${EXYONQ_ACCEPT_WORKERS:-}" = "auto" ]; then
  if command -v nproc >/dev/null 2>&1; then
    export EXYONQ_ACCEPT_WORKERS="$(nproc)"
  else
    export EXYONQ_ACCEPT_WORKERS=1
  fi
fi
exec "$@"
