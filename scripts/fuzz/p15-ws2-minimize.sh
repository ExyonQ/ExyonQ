#!/usr/bin/env bash
# P1.5-WS2 — minimize a crash artifact.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TOOLCHAIN="${P15_WS2_TOOLCHAIN:-nightly}"

if [[ $# -lt 2 ]]; then
  echo "usage: $0 <target> <artifact-path> [out-path]" >&2
  exit 2
fi
TARGET="$1"
ARTIFACT="$2"
OUT="${3:-$ROOT/tests/fixtures/security/fuzz-regressions/${TARGET}.min}"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "FATAL: cargo-fuzz missing" >&2
  exit 2
fi
[[ -f "$ARTIFACT" ]] || { echo "FATAL: artifact missing: $ARTIFACT" >&2; exit 2; }
mkdir -p "$(dirname "$OUT")"

cd "$ROOT/fuzz"
echo "MINIMIZE target=$TARGET artifact=$ARTIFACT out=$OUT"
RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo fuzz tmin "$TARGET" -- "$ARTIFACT" -exact_artifact_path="$OUT"
echo "MINIMIZED=$OUT"
