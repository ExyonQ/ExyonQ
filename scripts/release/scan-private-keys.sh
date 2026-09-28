#!/usr/bin/env bash
# Scan a tree for private-key material (EXYONQ-SEC-PRIVATE-MATERIAL-ZERO).
# No TLS fixture allowlist — private PEM under tests/fixtures is forbidden.
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/release/lib/p14sign-common.sh
source "$SCRIPT_DIR/lib/p14sign-common.sh"

usage() {
  cat <<'EOF'
Usage:
  scan-private-keys.sh --root DIR

Exit 0 if no private keys found.
Exit non-zero if private key material is detected.
Does not print key contents.
EOF
}

ROOT=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --root) p14sign_require_arg --root "${2:-}"; ROOT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) p14sign_die "unknown argument: $1" ;;
  esac
done

p14sign_require_arg --root "$ROOT"
[[ -d "$ROOT" ]] || p14sign_die "root not a directory: $ROOT"
# Prefer dedicated rule script when present (same fail-closed semantics).
ZERO="$SCRIPT_DIR/../test-tls/exyonq-sec-private-material-zero.sh"
if [[ -x "$ZERO" ]]; then
  bash "$ZERO" --root "$ROOT"
else
  p14sign_scan_path_for_private_keys "$ROOT"
  echo "PRIVATE_KEY_SCAN=PASS" >&2
fi
