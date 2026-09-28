#!/usr/bin/env bash
# UNSAFE_ALLOWLIST_GATE — ADR-045: fail if `unsafe {` appears outside allowlisted FFI paths.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:-}"

usage() {
  cat <<'EOF'
Usage:
  scripts/gates/unsafe-allowlist-gate.sh --selftest
  scripts/gates/unsafe-allowlist-gate.sh --tree <dir>
EOF
}

# Allowlisted roots (relative to tree). Platform-linux temporary until epoll migrates fully.
ALLOW_GLOBS=(
  "crates/exyonq-linux-ffi/"
  "crates/exyonq-platform-linux/"
)

# Paths that may still contain residual unsafe during migration — counted as REVIEW not FAIL
# until Phase B completes. Empty = strict.
RESIDUAL_REVIEW_GLOBS=(
  "crates/exyonq-mod-static/"
  "crates/exyonq-cfd-dataplane/"
  "crates/exyonq-ops-runtime/"
  "core/src/"
)

is_under() {
  local path="$1"
  local prefix="$2"
  case "$path" in
    "$prefix"*) return 0 ;;
    *) return 1 ;;
  esac
}

classify_path() {
  local rel="$1"
  local g
  for g in "${ALLOW_GLOBS[@]}"; do
    if is_under "$rel" "$g"; then
      echo ALLOW
      return
    fi
  done
  for g in "${RESIDUAL_REVIEW_GLOBS[@]}"; do
    if is_under "$rel" "$g"; then
      echo REVIEW
      return
    fi
  done
  echo FORBIDDEN
}

scan_tree() {
  local tree="$1"
  local tmp
  tmp="$(mktemp)"
  # Actual unsafe blocks only (not the word in comments alone when possible).
  (cd "$tree" && rg -n --glob '!target/**' --glob '!**/.exyonq-local/**' --glob '!**/scripts/gates/fixtures/**' --glob '!**/scripts/integrity/fixtures/**' '\bunsafe\s*\{' \
    core crates module-api modules config cli 2>/dev/null || true) >"$tmp"

  local forbid=0
  local review=0
  local allow=0
  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "$line" ]] && continue
    local file="${line%%:*}"
    local kind
    kind="$(classify_path "$file")"
    case "$kind" in
      ALLOW) allow=$((allow + 1)) ;;
      REVIEW)
        review=$((review + 1))
        echo "REVIEW unsafe: $line" >&2
        ;;
      FORBIDDEN)
        forbid=$((forbid + 1))
        echo "FORBIDDEN unsafe: $line" >&2
        ;;
    esac
  done <"$tmp"
  rm -f "$tmp"

  echo "UNSAFE_ALLOWLIST_GATE allow=$allow review=$review forbid=$forbid"
  if [[ "$forbid" -gt 0 ]]; then
    echo "UNSAFE_ALLOWLIST_GATE=FAIL" >&2
    return 1
  fi
  if [[ "$review" -gt 0 ]]; then
    echo "UNSAFE_ALLOWLIST_GATE=PASS_WITH_REVIEW"
    return 0
  fi
  echo "UNSAFE_ALLOWLIST_GATE=PASS"
  return 0
}

selftest() {
  local d
  d="$(mktemp -d)"
  mkdir -p "$d/crates/exyonq-linux-ffi/src" "$d/crates/exyonq-mod-proxy/src" "$d/core/src"
  printf 'fn x() { unsafe { } }\n' >"$d/crates/exyonq-linux-ffi/src/x.rs"
  printf 'fn y() { unsafe { } }\n' >"$d/crates/exyonq-mod-proxy/src/y.rs"
  if scan_tree "$d"; then
    echo "selftest expected FAIL for mod-proxy unsafe" >&2
    rm -rf "$d"
    return 1
  fi
  rm -f "$d/crates/exyonq-mod-proxy/src/y.rs"
  mkdir -p "$d/crates/exyonq-mod-static/src"
  printf 'fn z() { unsafe { } }\n' >"$d/crates/exyonq-mod-static/src/z.rs"
  scan_tree "$d" >/dev/null
  rm -rf "$d"
  echo "unsafe-allowlist-gate selftest OK"
}

case "$MODE" in
  --selftest) selftest ;;
  --tree)
    TREE="${2:-}"
    [[ -n "$TREE" ]] || { usage; exit 2; }
    scan_tree "$TREE"
    ;;
  *) usage; exit 2 ;;
esac
