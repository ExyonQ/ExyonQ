#!/usr/bin/env bash
# P1.5-WS2 — reproduce a crash artifact deterministically.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN="${P15_WS2_TOOLCHAIN:-nightly}"

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <target> <artifact-path>" >&2
  exit 2
fi
TARGET="$1"
ARTIFACT="$2"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "FATAL: cargo-fuzz missing" >&2
  exit 2
fi
[[ -f "$ARTIFACT" ]] || { echo "FATAL: artifact missing: $ARTIFACT" >&2; exit 2; }

cd "$ROOT/fuzz"
echo "REPRO target=$TARGET artifact=$ARTIFACT"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo fuzz run "$TARGET" -- "$ARTIFACT"
