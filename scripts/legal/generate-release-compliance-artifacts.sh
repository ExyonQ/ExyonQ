#!/usr/bin/env bash
# Generate release compliance artifacts (third-party notices + SBOM).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

bash "$ROOT/scripts/legal/generate-third-party-notices.sh"
bash "$ROOT/scripts/legal/generate-sbom.sh"

echo "release compliance artifacts ready:"
echo "  $ROOT/THIRD_PARTY_NOTICES.md"
echo "  $ROOT/sbom.cdx.json"
