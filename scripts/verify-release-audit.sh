#!/usr/bin/env bash
# Verify release audit artifact exists and has no open blockers.
set -euo pipefail

VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: verify-release-audit.sh X.Y.Z" >&2
  exit 2
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AUDIT="$ROOT/docs/security/audit-v${VERSION}.md"

if [[ ! -f "$AUDIT" ]]; then
  echo "missing audit file: $AUDIT" >&2
  exit 1
fi

if ! grep -Fq "**Version** | ${VERSION}" "$AUDIT"; then
  echo "audit version mismatch: expected ${VERSION} in $AUDIT" >&2
  exit 1
fi

blockers_section="$(awk '/^## Blockers/{flag=1; next} /^## / && flag{exit} flag' "$AUDIT")"
if echo "$blockers_section" | grep -qE '^### |^- \*\*'; then
  echo "audit has open blockers under ## Blockers" >&2
  exit 1
fi
trimmed="$(echo "$blockers_section" | sed '/^[[:space:]]*$/d')"
if [[ -n "$trimmed" ]] && ! echo "$trimmed" | grep -qiE '^(none|ninguno)\b'; then
  echo "audit ## Blockers must be empty or state 'None'" >&2
  exit 1
fi

echo "Release audit OK: audit-v${VERSION}.md"
