#!/usr/bin/env bash
# AUTHORITATIVE reverse-proxy functional E2E.
# Controlled HTTP peer (Python) is required by the proxy contract — not a product path substitute.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
e2e_ensure_bins
echo "proxy-e2e: AUTHORITATIVE → suites/proxy-suite.sh"
exec bash "$ROOT/scripts/e2e/suites/proxy-suite.sh"
