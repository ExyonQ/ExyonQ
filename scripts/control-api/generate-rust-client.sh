#!/usr/bin/env bash
# Cap016 — deterministic openapi-to-rust client generation + drift check.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/crates/exyonq-control-api-client"
CONFIG="$ROOT/crates/exyonq-control-api-client/openapi-to-rust.toml"
OPENAPI="$ROOT/docs/control-api/openapi-v1.yaml"

if [[ ! -f "$OPENAPI" ]]; then
  echo "missing OpenAPI contract: $OPENAPI" >&2
  exit 2
fi
if ! command -v openapi-to-rust >/dev/null 2>&1; then
  echo "openapi-to-rust not installed (cargo install --locked openapi-to-rust)" >&2
  exit 127
fi

MODE="${1:-generate}"
case "$MODE" in
  generate)
    openapi-to-rust generate -c "$CONFIG"
    ;;
  check)
    openapi-to-rust generate -c "$CONFIG" --check
    ;;
  *)
    echo "usage: $0 [generate|check]" >&2
    exit 2
    ;;
esac

echo "OPENAPI_GENERATION_STATUS=PASS"
echo "MODE=$MODE"
