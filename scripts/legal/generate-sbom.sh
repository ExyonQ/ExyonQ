#!/usr/bin/env bash
# Generate CycloneDX SBOM for the exyonq release binary (cli/exyonq).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUTPUT="${1:-$ROOT/sbom.cdx.json}"
OUTPUT="$(cd "$(dirname "$OUTPUT")" && pwd)/$(basename "$OUTPUT")"
MANIFEST="$ROOT/cli/exyonq/Cargo.toml"
STAGING="$ROOT/cli/exyonq/sbom.cdx.json"

if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
  echo "cargo-cyclonedx not found; install with: cargo install cargo-cyclonedx --locked" >&2
  exit 1
fi

cd "$ROOT"
rm -f "$STAGING"
cargo cyclonedx \
  --manifest-path "$MANIFEST" \
  --format json \
  --override-filename sbom.cdx \
  --spec-version 1.5 \
  --no-build-deps

if [[ ! -f "$STAGING" ]]; then
  echo "expected SBOM at $STAGING" >&2
  exit 1
fi

mv "$STAGING" "$OUTPUT"

# cargo-cyclonedx may leave transient *.cdx.json in workspace crate dirs; keep root artifact only.
while IFS= read -r -d '' stray; do
  rm -f "$stray"
done < <(find "$ROOT" -name '*.cdx.json' -not -path '*/target/*' ! -path "$OUTPUT" -print0)

echo "wrote $OUTPUT"
