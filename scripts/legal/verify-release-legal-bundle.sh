#!/usr/bin/env bash
# Release gate: license policy + distributable legal texts present or generable.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

for file in LICENSE NOTICE; do
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    exit 1
  fi
done

echo "==> verify deny.toml / about.toml alignment"
bash "$ROOT/scripts/legal/verify-license-policy-alignment.sh"

echo "==> cargo deny check licenses"
cargo deny check licenses

TMP="$(mktemp "${TMPDIR:-/tmp}/third-party-notices.XXXXXX.md")"
trap 'rm -f "$TMP"' EXIT

echo "==> generate third-party notices (smoke)"
bash "$ROOT/scripts/legal/generate-third-party-notices.sh" "$TMP"

if [[ ! -s "$TMP" ]]; then
  echo "generated third-party notices file is empty" >&2
  exit 1
fi

if ! grep -q '^# Third-Party Notices — ExyonQ' "$TMP"; then
  echo "third-party notices missing expected header" >&2
  exit 1
fi

echo "release legal bundle OK"
