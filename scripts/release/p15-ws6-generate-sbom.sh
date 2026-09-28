#!/usr/bin/env bash
# P1.5-WS6 — Generate CycloneDX SBOM into artifact out-dir; record SHA256.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-generate-sbom.sh --workspace PATH --out-dir PATH
EOF
}

WORKSPACE=""
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$OUT_DIR" ]] || {
  usage >&2
  exit 2
}

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
SBOM_OUT="$OUT_DIR/sbom.cdx.json"
HASH_OUT="$OUT_DIR/sbom.sha256"

GEN="$ROOT/scripts/legal/generate-sbom.sh"
[[ -x "$GEN" || -f "$GEN" ]] || {
  echo "ERROR: missing $GEN" >&2
  exit 1
}

VERDICT="PASS"
NOTE=""

if ! command -v cargo-cyclonedx >/dev/null 2>&1; then
  VERDICT="SKIP_TOOLING"
  NOTE="cargo-cyclonedx not installed"
  ws6_log "$NOTE"
else
  cd "$WORKSPACE"
  if ! bash "$GEN" "$SBOM_OUT"; then
    VERDICT="FAIL"
    NOTE="generate-sbom.sh failed"
  elif [[ ! -s "$SBOM_OUT" ]]; then
    VERDICT="FAIL"
    NOTE="empty sbom output"
  else
    ws6_sha256_file "$SBOM_OUT" >"$HASH_OUT"
    NOTE="wrote $SBOM_OUT"
  fi
fi

SUMMARY_JSON="$(python3 - <<PY
import json, pathlib
sbom = pathlib.Path("$SBOM_OUT")
print(json.dumps({
  "script": "p15-ws6-generate-sbom.sh",
  "verdict": "$VERDICT",
  "sbom_path": str(sbom) if sbom.exists() else None,
  "sbom_sha256": pathlib.Path("$HASH_OUT").read_text().strip() if pathlib.Path("$HASH_OUT").exists() else None,
  "note": "$NOTE",
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=$VERDICT
SBOM=$SBOM_OUT
NOTE=$NOTE"

ws6_write_summary_pair "$OUT_DIR" "$SUMMARY_TXT" "$SUMMARY_JSON"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"

case "$VERDICT" in
  PASS) exit 0 ;;
  SKIP_TOOLING) exit 0 ;;
  *) exit 1 ;;
esac
