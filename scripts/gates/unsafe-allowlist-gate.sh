#!/usr/bin/env bash
# EXYONQ-UNSAFE-ALLOWLIST-GATE
# ADR-045 §6: `unsafe` is allowed only inside the FFI capsule
# (`crates/exyonq-linux-ffi/**`) and, temporarily, the epoll worker crate
# (`crates/exyonq-platform-linux/**`). Everything else is a ratchet: the
# pre-existing occurrences are recorded in the baseline and the gate fails
# when the set grows. Shrinking the baseline is always allowed (and reported).
#
# Usage:
#   unsafe-allowlist-gate.sh --tree DIR [--baseline FILE] [--strict]
#   unsafe-allowlist-gate.sh --selftest
#
# Exit: 0 pass, 1 new/growing unsafe outside the allowlist (or --strict hit).
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

RULE_ID=EXYONQ-UNSAFE-ALLOWLIST-GATE
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TREE=""
BASELINE=""
STRICT=0
SELFTEST=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tree) TREE="${2:?}"; shift 2 ;;
    --baseline) BASELINE="${2:?}"; shift 2 ;;
    --strict) STRICT=1; shift ;;
    --selftest) SELFTEST=1; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; exit 2 ;;
  esac
done

ALLOW_PREFIXES=(
  "crates/exyonq-linux-ffi/"
  "crates/exyonq-platform-linux/"
)
SKIP_PREFIXES=(
  "target/"
  ".exyonq-local/"
  ".git/"
  "node_modules/"
  "ROADMAP EXYONQ/"
)

scan_tree() {
  local tree="$1"
  python3 - "$tree" "${ALLOW_PREFIXES[@]}" -- "${SKIP_PREFIXES[@]}" <<'PY'
import re, sys, pathlib

argv = sys.argv[1:]
tree = pathlib.Path(argv[0])
sep = argv.index("--")
allow = tuple(argv[1:sep])
skip = tuple(argv[sep + 1:])

pattern = re.compile(r"\bunsafe\s*(?:\{|fn\b|impl\b|trait\b|extern\b)")
counts = {}
for path in tree.rglob("*.rs"):
    rel = str(path.relative_to(tree)).replace("\\", "/")
    if any(rel.startswith(p) for p in skip):
        continue
    if any(rel.startswith(p) for p in allow):
        continue
    n = len(pattern.findall(path.read_text(errors="ignore")))
    if n:
        counts[rel] = n
for rel in sorted(counts):
    print(f"{counts[rel]} {rel}")
PY
}

read_baseline() {
  local file="$1"
  [[ -f "$file" ]] || return 0
  grep -vE '^\s*(#|$)' "$file" | awk '{print $1" "$2}'
}

run_gate() {
  local tree="$1" baseline="$2"
  local actual expected
  actual="$(scan_tree "$tree")"

  expected="$(read_baseline "$baseline")"

  local total=0 files=0
  while read -r n path; do
    [[ -z "${n:-}" ]] && continue
    total=$((total + n)); files=$((files + 1))
  done <<<"$actual"

  echo "RULE_ID=$RULE_ID"
  echo "UNSAFE_OUTSIDE_ALLOWLIST_FILES=$files"
  echo "UNSAFE_OUTSIDE_ALLOWLIST_OCCURRENCES=$total"

  if [[ "$STRICT" == "1" ]]; then
    if [[ "$total" -gt 0 ]]; then
      echo "STRICT_MODE=1"
      echo "UNSAFE_ALLOWLIST_GATE=FAIL"
      echo "UNSAFE_ALLOWLIST_VIOLATIONS=$total"
      while read -r n path; do [[ -n "${n:-}" ]] && echo "VIOLATION $path occurrences=$n"; done <<<"$actual"
      return 1
    fi
    echo "UNSAFE_ALLOWLIST_GATE=PASS"
    return 0
  fi

  local failures=0 stale=0
  # grow: new file or more occurrences than the baseline
  while read -r n path; do
    [[ -z "${n:-}" ]] && continue
    local base
    base="$(awk -v p="$path" '$2 == p {print $1}' <<<"$expected")"
    if [[ -z "$base" ]]; then
      echo "NEW_UNSAFE $path occurrences=$n"
      failures=$((failures + 1))
    elif [[ "$n" -gt "$base" ]]; then
      echo "UNSAFE_GREW $path baseline=$base now=$n"
      failures=$((failures + 1))
    elif [[ "$n" -lt "$base" ]]; then
      echo "BASELINE_STALE $path baseline=$base now=$n (shrink the baseline)"
      stale=$((stale + 1))
    fi
  done <<<"$actual"

  # baseline entries that no longer exist at all are stale too
  while read -r n path; do
    [[ -z "${n:-}" ]] && continue
    if ! grep -qE "^[0-9]+ ${path//\//\\/}$" <<<"$actual"; then
      echo "BASELINE_STALE $path gone (shrink the baseline)"
      stale=$((stale + 1))
    fi
  done <<<"$expected"

  echo "BASELINE_STALE_ENTRIES=$stale"
  if [[ "$failures" -gt 0 ]]; then
    echo "UNSAFE_ALLOWLIST_GATE=FAIL"
    echo "NEW_UNSAFE_OUTSIDE_ALLOWLIST=$failures"
    return 1
  fi
  echo "UNSAFE_ALLOWLIST_GATE=PASS"
  return 0
}

selftest() {
  local tmp status=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN

  mkdir -p "$tmp/clean/crates/exyonq-linux-ffi/src" "$tmp/clean/core/src"
  printf 'pub fn a() { let _ = 1; }\n' >"$tmp/clean/core/src/lib.rs"
  printf 'pub fn b() { unsafe { std::ptr::null::<u8>(); } }\n' \
    >"$tmp/clean/crates/exyonq-linux-ffi/src/lib.rs"
  if ! ( TREE="$tmp/clean"; STRICT=0; BASELINE=""; run_gate "$TREE" "$BASELINE" ) >/dev/null; then
    echo "SELFTEST_FAIL allowlisted_only_must_pass"
    status=1
  else
    echo "SELFTEST_PASS allowlisted_only_must_pass"
  fi

  mkdir -p "$tmp/dirty/core/src"
  printf 'pub fn c() { unsafe { std::ptr::null::<u8>(); } }\n' >"$tmp/dirty/core/src/lib.rs"
  if ( TREE="$tmp/dirty"; STRICT=0; BASELINE=""; run_gate "$TREE" "$BASELINE" ) >/dev/null; then
    echo "SELFTEST_FAIL new_unsafe_must_fail"
    status=1
  else
    echo "SELFTEST_PASS new_unsafe_must_fail"
  fi

  printf '1 core/src/lib.rs\n' >"$tmp/baseline.txt"
  if ! ( TREE="$tmp/dirty"; STRICT=0; BASELINE="$tmp/baseline.txt"; run_gate "$TREE" "$BASELINE" ) >/dev/null; then
    echo "SELFTEST_FAIL baselined_unsafe_must_pass"
    status=1
  else
    echo "SELFTEST_PASS baselined_unsafe_must_pass"
  fi

  printf 'pub fn c() { unsafe { std::ptr::null::<u8>(); } unsafe { std::ptr::null::<u8>(); } }\n' \
    >"$tmp/dirty/core/src/lib.rs"
  if ( TREE="$tmp/dirty"; STRICT=0; BASELINE="$tmp/baseline.txt"; run_gate "$TREE" "$BASELINE" ) >/dev/null; then
    echo "SELFTEST_FAIL growth_over_baseline_must_fail"
    status=1
  else
    echo "SELFTEST_PASS growth_over_baseline_must_fail"
  fi

  if ( TREE="$tmp/dirty"; STRICT=1; BASELINE="$tmp/baseline.txt"; run_gate "$TREE" "$BASELINE" ) >/dev/null; then
    echo "SELFTEST_FAIL strict_must_fail"
    status=1
  else
    echo "SELFTEST_PASS strict_must_fail"
  fi

  echo "UNSAFE_ALLOWLIST_GATE_SELFTEST=$([[ $status -eq 0 ]] && echo PASS || echo FAIL)"
  return "$status"
}

if [[ "$SELFTEST" == "1" ]]; then
  selftest
  exit $?
fi

TREE="${TREE:-$ROOT}"
BASELINE="${BASELINE:-$ROOT/docs/governance/unsafe-baseline.txt}"
run_gate "$TREE" "$BASELINE"
