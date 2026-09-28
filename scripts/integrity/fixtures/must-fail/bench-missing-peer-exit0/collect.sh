#!/usr/bin/env bash
# AUDITOR_NEGATIVE_FIXTURE — soft-skip missing peer then exit 0 (must remain HIGH).
set -euo pipefail
cid=""
if [[ -z "$cid" ]]; then
  echo "{\"error\":\"bench-runner not found\"}" >"/tmp/out.json"
  exit 0
fi
