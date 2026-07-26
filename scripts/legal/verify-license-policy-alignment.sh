#!/usr/bin/env bash
# Ensure scripts/legal/about.toml accepted licenses cover deny.toml [licenses].allow.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
python3 - "$ROOT/deny.toml" "$ROOT/scripts/legal/about.toml" <<'PY'
import re
import sys
from pathlib import Path

deny_path, about_path = map(Path, sys.argv[1:3])

def licenses_from_deny(text: str) -> set[str]:
    block = text.split("[licenses]", 1)[1]
    block = block.split("\n[", 1)[0]
    return set(re.findall(r'"([^"]+)"', block))

def licenses_from_about(text: str) -> set[str]:
    block = text.split("accepted = [", 1)[1]
    block = block.split("]", 1)[0]
    return set(re.findall(r'"([^"]+)"', block))

deny = licenses_from_deny(deny_path.read_text())
about = licenses_from_about(about_path.read_text())

missing = sorted(deny - about)
if missing:
    print("license policy drift: deny.toml allows licenses missing from about.toml:", file=sys.stderr)
    for lic in missing:
        print(f"  - {lic}", file=sys.stderr)
    print("Keep scripts/legal/about.toml aligned with deny.toml [licenses].allow.", file=sys.stderr)
    sys.exit(1)

print(f"license policy aligned ({len(deny)} deny licenses covered by about.toml)")

PY
