#!/usr/bin/env bash
# AUTHORITATIVE reverse-proxy functional E2E.
# Controlled HTTP peer (Python) is required by the proxy contract — not a product mock substitute.
# Body migrated from scripts/smoke/kd3-proxy-smoke.sh (class C audit); this path is the closer.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins
echo "proxy-e2e: AUTHORITATIVE entry → kd3 proxy functional suite (controlled peer)"
exec bash "$ROOT/scripts/smoke/kd3-proxy-smoke.sh"
