#!/usr/bin/env bash
# Verify OSS module boundary imports (ADR-026).
#
# Phase 0–1: warn-only (exit 0 with warnings).
# Phase 2+:  set EXYONQ_OSS_BOUNDARIES_STRICT=1 for CI-blocking mode.
#
# Usage:
#   bash scripts/verify-oss-boundaries.sh
#   EXYONQ_OSS_BOUNDARIES_STRICT=1 bash scripts/verify-oss-boundaries.sh
#   bash scripts/verify-oss-boundaries.sh --selftest
#
# Strict-mode negative smoke test (--selftest):
#   Builds a temp tree with `use exyonq_core::...` under modules/, runs this script
#   with EXYONQ_OSS_BOUNDARIES_ROOT pointing at it and STRICT=1 (expect exit 1).
#   Then runs STRICT=1 on the real repo (expect exit 0). Does not modify the repo.
#
# Manual strict-mode probe (optional):
#   EXYONQ_OSS_BOUNDARIES_ROOT=/tmp/exyonq-boundary-probe EXYONQ_OSS_BOUNDARIES_STRICT=1 \
#     bash scripts/verify-oss-boundaries.sh
#   (create /tmp/exyonq-boundary-probe/modules/x/src/lib.rs with a forbidden import first)
set -euo pipefail

_VERIFY_SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
_VERIFY_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

ROOT="${EXYONQ_OSS_BOUNDARIES_ROOT:-$_VERIFY_REPO_ROOT}"
cd "$ROOT"

STRICT="${EXYONQ_OSS_BOUNDARIES_STRICT:-0}"
VIOLATIONS=0

warn() {
  echo "verify-oss-boundaries: WARNING: $*" >&2
  VIOLATIONS=$((VIOLATIONS + 1))
}

# module / official addon source trees (current + future paths)
MOD_GLOBS=(
  "modules"
  "crates/exyonq-mod-*"
)

# config / contract trees that must stay independent of core
CONFIG_GLOBS=(
  "config"
  "crates/exyonq-config*"
  "addon-api"
  "crates/exyonq-core-api"
  "addon-sdk"
  "crates/exyonq-module-sdk"
  "crates/exyonq-rule-engine"
  "crates/exyonq-runtime-plan"
)

# Resolve a named glob array to existing paths only.
# Usage: resolve_globs ARRAY_NAME  → prints paths (one per line).
resolve_globs() {
  local array_name="$1"
  # shellcheck disable=SC2178,SC2128
  eval "local globs=(\"\${${array_name}[@]}\")"
  local g
  for g in "${globs[@]}"; do
    if [[ -e "$g" ]]; then
      printf '%s\n' "$g"
      continue
    fi
    local match
    for match in $g; do
      [[ -e "$match" ]] && printf '%s\n' "$match"
    done
  done
}

# Scan Rust sources under paths from a glob set for ripgrep patterns.
# Usage: scan_paths "label" GLOB_ARRAY_NAME pattern [pattern ...]
scan_paths() {
  local label="$1"
  local glob_array_name="$2"
  shift 2
  local patterns=("$@")

  local paths=()
  local p
  while IFS= read -r p; do
    [[ -n "$p" ]] && paths+=("$p")
  done < <(resolve_globs "$glob_array_name" | sort -u)

  if [[ ${#paths[@]} -eq 0 ]]; then
    return 0
  fi

  local pat matches
  for pat in "${patterns[@]}"; do
    matches="$(rg -n --type rust "$pat" "${paths[@]}" 2>/dev/null || true)"
    if [[ -n "$matches" ]]; then
      warn "$label (pattern: $pat)"
      echo "$matches" >&2
    fi
  done
}

scan_compat() {
  [[ -d compat ]] || return 0
  local matches
  matches="$(rg -n --type rust 'exyonq_core' compat/ 2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    warn "compat/ imports exyonq-core (offline-only layer)"
    echo "$matches" >&2
  fi
}

scan_module_cargo_manifests() {
  local manifest
  for manifest in modules/*/Cargo.toml crates/exyonq-mod-*/Cargo.toml; do
    [[ -f "$manifest" ]] || continue
    if grep -qE 'exyonq-core|path = "\.\./core"' "$manifest"; then
      warn "module crate forbidden Cargo dependency in $manifest"
      grep -nE 'exyonq-core|path = "\.\./core"' "$manifest" >&2 || true
    fi
  done
}

run_selftest() {
  echo "verify-oss-boundaries: --selftest (strict negative smoke) ..."
  local tmp strict_rc clean_strict_rc
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN

  mkdir -p "$tmp/modules/probe/src"
  printf '%s\n' 'use exyonq_core::server::probe;' >"$tmp/modules/probe/src/lib.rs"

  strict_rc=0
  EXYONQ_OSS_BOUNDARIES_ROOT="$tmp" EXYONQ_OSS_BOUNDARIES_STRICT=1 \
    bash "$_VERIFY_SCRIPT" >/dev/null 2>&1 || strict_rc=$?
  if [[ "$strict_rc" -ne 1 ]]; then
    echo "verify-oss-boundaries: --selftest FAILED: expected exit 1 for synthetic violation under STRICT (got $strict_rc)" >&2
    return 1
  fi

  clean_strict_rc=0
  EXYONQ_OSS_BOUNDARIES_STRICT=1 bash "$_VERIFY_SCRIPT" >/dev/null 2>&1 || clean_strict_rc=$?
  if [[ "$clean_strict_rc" -ne 0 ]]; then
    echo "verify-oss-boundaries: --selftest FAILED: clean repo must exit 0 under STRICT (got $clean_strict_rc)" >&2
    return 1
  fi

  _PHASE0_DIR="$(cd "$(dirname "$_VERIFY_SCRIPT")/architecture" && pwd)"
  if [[ -f "$_PHASE0_DIR/verify-phase0-kernel.sh" ]]; then
    phase0_selftest_rc=0
    bash "$_PHASE0_DIR/verify-phase0-kernel.sh" --selftest >/dev/null 2>&1 || phase0_selftest_rc=$?
    if [[ "$phase0_selftest_rc" -ne 0 ]]; then
      echo "verify-oss-boundaries: --selftest FAILED: phase0 kernel selftest" >&2
      return 1
    fi
  fi

  echo "verify-oss-boundaries: --selftest OK (synthetic tree fails strict; clean repo passes strict)"
  return 0
}

if [[ "${1:-}" == "--selftest" ]]; then
  run_selftest
  exit $?
fi

echo "verify-oss-boundaries: scanning (strict=${STRICT}) ..."

scan_paths "module crate imports exyonq-core" MOD_GLOBS \
  'use exyonq_core::' \
  'exyonq_core::(server|proxy|static_files|snapshot|reload|router)::'

scan_paths "config/contract imports exyonq-core" CONFIG_GLOBS \
  'exyonq_core'

scan_compat
scan_module_cargo_manifests

# Phase 0 kernel lint (PR-0 guardrails)
_PHASE0="$(cd "$(dirname "$_VERIFY_SCRIPT")/architecture" && pwd)/verify-phase0-kernel.sh"
if [[ -f "$_PHASE0" ]]; then
  _PHASE0_STRICT=0
  [[ "$STRICT" == "1" ]] && _PHASE0_STRICT=1
  if ! EXYONQ_PHASE0_KERNEL_ROOT="$ROOT" EXYONQ_PHASE0_KERNEL_STRICT="$_PHASE0_STRICT" bash "$_PHASE0"; then
    VIOLATIONS=$((VIOLATIONS + 1))
  fi
fi

if [[ "$VIOLATIONS" -eq 0 ]]; then
  echo "verify-oss-boundaries: OK (no violations)"
  exit 0
fi

echo "verify-oss-boundaries: found ${VIOLATIONS} violation group(s)" >&2
if [[ "$STRICT" == "1" ]]; then
  echo "verify-oss-boundaries: STRICT mode — failing" >&2
  exit 1
fi

echo "verify-oss-boundaries: warn-only mode — passing (set EXYONQ_OSS_BOUNDARIES_STRICT=1 to fail)" >&2
exit 0
