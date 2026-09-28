#!/usr/bin/env bash
# Report pinned vs latest Wasmtime and surface advisories (signal mode).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PINNED_LINE="$(grep -E '^wasmtime =' Cargo.toml | head -1 || true)"
PINNED_VER="$(echo "$PINNED_LINE" | sed -E 's/.*"=?([0-9]+\.[0-9]+\.[0-9]+)".*/\1/' | head -1)"

LATEST_LINE="$(cargo search wasmtime --limit 1 2>/dev/null | head -1 || true)"
LATEST_VER="$(echo "$LATEST_LINE" | awk '{print $3}' | tr -d '"')"

echo "=== Wasmtime watch ==="
echo "Pinned (Cargo.toml): ${PINNED_LINE:-<not found>}"
echo "Latest (crates.io):  ${LATEST_VER:-unknown}"
if [[ -n "$PINNED_VER" && -n "$LATEST_VER" && "$PINNED_VER" != "$LATEST_VER" ]]; then
  echo "WARNING: version drift — update docs/dependencies/wasmtime-watch.md after reviewing RELEASES.md"
fi

echo ""
echo "=== Direct consumers ==="
cargo tree -i wasmtime 2>/dev/null || echo "(cargo tree unavailable)"

echo ""
echo "=== Advisories (wasmtime) ==="
if command -v cargo-audit >/dev/null 2>&1; then
  cargo audit 2>/dev/null | rg -i wasmtime || echo "No wasmtime advisories in audit output"
else
  echo "cargo-audit not installed — skip (install for full check)"
fi

echo ""
echo "Next: record findings in docs/dependencies/wasmtime-watch.md"
