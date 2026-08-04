#!/usr/bin/env bash
# AUTHORITATIVE HTTP/3 protocol E2E entrypoint.
# NO-SMOKE: smoke scripts are legacy and are not validation evidence.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux
TARGET="$ROOT/scripts/e2e/http3-post-body-e2e.sh"
if [[ ! -f "$TARGET" ]]; then
  echo "FAIL: missing $TARGET"
  exit 1
fi
echo "http3-protocol-e2e: AUTHORITATIVE NO-SMOKE → http3-post-body-e2e"
exec bash "$TARGET"
