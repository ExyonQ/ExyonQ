#!/usr/bin/env bash
# Deprecated wrapper — use scripts/security-fuzz.sh for all fuzz targets.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
exec bash "$ROOT/scripts/security-fuzz.sh" "$@"
