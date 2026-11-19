#!/usr/bin/env bash
# Q-13: every TOML example in the tracked docs must validate with the real
# binary. A doc that shows an example the validator rejects is not copyable.
#
# Usage: bash scripts/gates/q13-docs-audit.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/release/exyonq}"

if [[ ! -x "$EXYONQ_BIN" ]]; then
  echo "FAIL: missing $EXYONQ_BIN (build it with: cargo build --release -p exyonq --bin exyonq --locked)"
  echo "Q13_DOCS_AUDIT_EXIT=2"
  exit 2
fi

cd "$ROOT"
EXYONQ_BIN="$EXYONQ_BIN" python3 scripts/gates/q13-docs-audit.py
rc=$?
echo "Q13_DOCS_AUDIT_EXIT=$rc"
exit "$rc"
