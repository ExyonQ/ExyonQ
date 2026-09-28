#!/usr/bin/env bash
# Local/CI gate — Phase 0 + 7-B kernel boundary lint (STRICT).
#
# Usage:
#   bash scripts/architecture/verify-kernel-guards.sh
#   bash scripts/architecture/verify-kernel-guards.sh --selftest
#   bash scripts/architecture/verify-kernel-guards.sh --report study/reports/KERNEL-BOUNDARY-LINT-REPORT.md
#
# Equivalent:
#   EXYONQ_PHASE0_KERNEL_STRICT=1 bash scripts/architecture/verify-phase0-kernel.sh
#
# Contract: docs/architecture/phase7b-runtime-guardrails.md
set -euo pipefail

_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_ROOT="$(cd "$_DIR/../.." && pwd)"

if ! command -v rg >/dev/null 2>&1; then
  echo "verify-kernel-guards: ripgrep (rg) required on PATH" >&2
  exit 2
fi

case "${1:-}" in
  -h | --help)
    sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  --selftest)
    exec bash "$_DIR/verify-phase0-kernel.sh" --selftest
    ;;
esac

export EXYONQ_PHASE0_KERNEL_ROOT="${EXYONQ_PHASE0_KERNEL_ROOT:-$_ROOT}"
export EXYONQ_PHASE0_KERNEL_STRICT=1
exec bash "$_DIR/verify-phase0-kernel.sh" "$@"
