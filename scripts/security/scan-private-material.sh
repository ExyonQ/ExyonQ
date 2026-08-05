#!/usr/bin/env bash
# Canonical FAIL_CLOSED private-material scanner.
# RULE_ID=EXYONQ-SEC-PRIVATE-MATERIAL-ZERO
#
# Usage:
#   scan-private-material.sh --tree DIR
#   scan-private-material.sh --git-index [--repo DIR]
#   scan-private-material.sh --git-tree [--repo DIR]
#   scan-private-material.sh --git-diff [--repo DIR] [--base REF]
#   scan-private-material.sh --archive FILE
#   scan-private-material.sh --oci PATH
#   scan-private-material.sh --assets DIR
#   scan-private-material.sh --selftest
#   scan-private-material.sh --regression
#
# Exit != 0 on any private material. Never prints secret bodies.
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
ENGINE="$SCRIPT_DIR/lib/private_material_scan.py"
REGRESSION="$SCRIPT_DIR/tests/test-private-material-scanner.sh"

pick_python() {
  if [[ -n "${EXYONQ_PYTHON3:-}" && -x "${EXYONQ_PYTHON3}" ]]; then
    printf '%s\n' "$EXYONQ_PYTHON3"
    return 0
  fi
  if [[ -x /usr/bin/python3 ]]; then
    printf '%s\n' /usr/bin/python3
    return 0
  fi
  if [[ -x /opt/homebrew/bin/python3 ]]; then
    printf '%s\n' /opt/homebrew/bin/python3
    return 0
  fi
  command -v python3
}

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
}

MODE=""
ROOT=""
REPO="$REPO_ROOT"
BASE="origin/main"
ARCHIVE=""
OCI=""
ASSETS=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tree) MODE=tree; ROOT="${2:-}"; shift 2 ;;
    --git-index) MODE=git-index; shift ;;
    --git-tree) MODE=git-tree; shift ;;
    --git-diff) MODE=git-diff; shift ;;
    --archive) MODE=archive; ARCHIVE="${2:-}"; shift 2 ;;
    --oci) MODE=oci; OCI="${2:-}"; shift 2 ;;
    --assets) MODE=assets; ASSETS="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --base) BASE="${2:-}"; shift 2 ;;
    --root) ROOT="${2:-}"; shift 2 ;; # alias for --tree value when MODE preset
    --selftest) MODE=selftest; shift ;;
    --regression) MODE=regression; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$MODE" ]] || { echo "ERROR: mode required" >&2; usage >&2; exit 2; }

PY="$(pick_python)"
[[ -f "$ENGINE" ]] || { echo "ERROR: missing engine $ENGINE" >&2; exit 2; }

case "$MODE" in
  tree)
    [[ -n "$ROOT" && -d "$ROOT" ]] || { echo "ERROR: --tree DIR required" >&2; exit 2; }
    exec "$PY" "$ENGINE" tree --root "$ROOT"
    ;;
  git-index)
    exec "$PY" "$ENGINE" git-index --repo "$REPO"
    ;;
  git-tree)
    exec "$PY" "$ENGINE" git-tree --repo "$REPO"
    ;;
  git-diff)
    exec "$PY" "$ENGINE" git-diff --repo "$REPO" --base "$BASE"
    ;;
  archive)
    [[ -n "$ARCHIVE" && -f "$ARCHIVE" ]] || { echo "ERROR: --archive FILE required" >&2; exit 2; }
    exec "$PY" "$ENGINE" archive --archive "$ARCHIVE"
    ;;
  oci)
    [[ -n "$OCI" ]] || { echo "ERROR: --oci PATH required" >&2; exit 2; }
    exec "$PY" "$ENGINE" oci --oci "$OCI"
    ;;
  assets)
    [[ -n "$ASSETS" && -d "$ASSETS" ]] || { echo "ERROR: --assets DIR required" >&2; exit 2; }
    exec "$PY" "$ENGINE" assets --assets "$ASSETS"
    ;;
  selftest|regression)
    exec bash "$REGRESSION"
    ;;
  *)
    echo "ERROR: unknown mode: $MODE" >&2
    exit 2
    ;;
esac
