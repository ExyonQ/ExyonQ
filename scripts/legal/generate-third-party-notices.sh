#!/usr/bin/env bash
# Generate THIRD_PARTY_NOTICES.md from Cargo.lock via cargo-about.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
CONFIG="$ROOT/scripts/legal/about.toml"
TEMPLATE="$ROOT/scripts/legal/about.hbs"
OUTPUT="${1:-$ROOT/THIRD_PARTY_NOTICES.md}"

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "cargo-about not found; install with: cargo install cargo-about --locked --features cli" >&2
  exit 1
fi

cd "$ROOT"
cargo about generate "$TEMPLATE" \
  --config "$CONFIG" \
  --workspace \
  --locked \
  --fail \
  --output-file "$OUTPUT"

echo "wrote $OUTPUT"
