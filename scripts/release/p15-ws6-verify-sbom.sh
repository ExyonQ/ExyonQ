#!/usr/bin/env bash
# P1.5-WS6 — Verify SBOM exists, is valid JSON, has components/metadata; hash match.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-verify-sbom.sh --artifact-dir PATH [--sbom PATH] [--hash-file PATH]
EOF
}

ARTIFACT_DIR=""
SBOM=""
HASH_FILE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifact-dir) ARTIFACT_DIR="$2"; shift 2 ;;
    --sbom) SBOM="$2"; shift 2 ;;
    --hash-file) HASH_FILE="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$ARTIFACT_DIR" && -d "$ARTIFACT_DIR" ]] || {
  usage >&2
  exit 2
}

ARTIFACT_DIR="$(cd "$ARTIFACT_DIR" && pwd)"
[[ -n "$SBOM" ]] || SBOM="$ARTIFACT_DIR/sbom.cdx.json"
[[ -n "$HASH_FILE" ]] || HASH_FILE="$ARTIFACT_DIR/sbom.sha256"

VERDICT="PASS"
NOTES=()

if [[ ! -f "$SBOM" ]]; then
  VERDICT="FAIL"
  NOTES+=("sbom_missing")
elif [[ ! -s "$SBOM" ]]; then
  VERDICT="FAIL"
  NOTES+=("sbom_empty")
else
  python3 - "$SBOM" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    data = json.load(f)
if not isinstance(data, dict):
    raise SystemExit("sbom_not_object")
has_components = bool(data.get("components"))
has_metadata = bool(data.get("metadata"))
if not has_components and not has_metadata:
    raise SystemExit("sbom_missing_components_and_metadata")
print("ok")
PY
  if [[ $? -ne 0 ]]; then
    VERDICT="FAIL"
    NOTES+=("sbom_invalid_json_or_structure")
  fi
fi

if [[ -f "$HASH_FILE" && -f "$SBOM" ]]; then
  expected="$(tr -d '[:space:]' <"$HASH_FILE")"
  actual="$(ws6_sha256_file "$SBOM")"
  if [[ "$expected" != "$actual" ]]; then
    VERDICT="FAIL"
    NOTES+=("hash_mismatch")
  else
    NOTES+=("hash_match")
  fi
elif [[ -f "$SBOM" ]]; then
  NOTES+=("hash_file_missing_skip_hash_check")
fi

NOTES_STR="${NOTES[*]}"
SUMMARY_JSON="$(python3 - "$VERDICT" "$SBOM" "$HASH_FILE" "$NOTES_STR" <<'PY'
import json, sys
verdict, sbom, hash_file, notes_raw = sys.argv[1:5]
notes = [n for n in notes_raw.split() if n]
print(json.dumps({
  "script": "p15-ws6-verify-sbom.sh",
  "verdict": verdict,
  "sbom": sbom,
  "hash_file": hash_file,
  "notes": notes,
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=$VERDICT
SBOM=$SBOM
NOTES=${NOTES[*]}"

ws6_write_summary_pair "$ARTIFACT_DIR" "$SUMMARY_TXT" "$SUMMARY_JSON"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"
[[ "$VERDICT" == "PASS" ]]
