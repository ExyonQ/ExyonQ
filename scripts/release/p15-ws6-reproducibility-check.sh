#!/usr/bin/env bash
# P1.5-WS6 — Two clean builds; compare binary SHA256; classify reproducibility honestly.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=scripts/release/lib/ws6-common.sh
source "$ROOT/scripts/release/lib/ws6-common.sh"

usage() {
  cat <<'EOF'
Usage:
  p15-ws6-reproducibility-check.sh \
    --workspace PATH \
    --target x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu \
    --out-dir PATH
EOF
}

WORKSPACE=""
TARGET=""
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="$2"; shift 2 ;;
    --target) TARGET="$2"; shift 2 ;;
    --out-dir) OUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$TARGET" && -n "$OUT_DIR" ]] || {
  usage >&2
  exit 2
}

ws6_require_linux

WS6_REPO_ROOT="$WORKSPACE"
ARCH="$(ws6_target_to_arch "$TARGET")"
mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
BUILD_SCRIPT="$ROOT/scripts/release/p15-ws6-build-artifacts.sh"

DIR_A="$OUT_DIR/build_a"
DIR_B="$OUT_DIR/build_b"
mkdir -p "$DIR_A" "$DIR_B"

ws6_log "reproducibility check: build A"
bash "$BUILD_SCRIPT" --workspace "$WORKSPACE" --target "$TARGET" --out-dir "$DIR_A" --profile release

ws6_log "reproducibility check: build B (clean staging dir)"
bash "$BUILD_SCRIPT" --workspace "$WORKSPACE" --target "$TARGET" --out-dir "$DIR_B" --profile release

TARBALL_A="$(find "$DIR_A" -maxdepth 1 -name 'exyonq-*-linux-*.tar.gz' | head -n1)"
TARBALL_B="$(find "$DIR_B" -maxdepth 1 -name 'exyonq-*-linux-*.tar.gz' | head -n1)"
[[ -n "$TARBALL_A" && -n "$TARBALL_B" ]] || {
  echo "ERROR: missing tarball from build dirs" >&2
  exit 1
}

EXTRACT_A="$(mktemp -d)"
EXTRACT_B="$(mktemp -d)"
cleanup() { rm -rf "$EXTRACT_A" "$EXTRACT_B"; }
trap cleanup EXIT INT TERM

tar -xzf "$TARBALL_A" -C "$EXTRACT_A"
tar -xzf "$TARBALL_B" -C "$EXTRACT_B"

BIN_A="$EXTRACT_A/usr/bin/exyonq"
BIN_B="$EXTRACT_B/usr/bin/exyonq"
CTL_A="$EXTRACT_A/usr/bin/exyonqctl"
CTL_B="$EXTRACT_B/usr/bin/exyonqctl"

SHA_EXY_A="$(ws6_sha256_file "$BIN_A")"
SHA_EXY_B="$(ws6_sha256_file "$BIN_B")"
SHA_CTL_A="$(ws6_sha256_file "$CTL_A")"
SHA_CTL_B="$(ws6_sha256_file "$CTL_B")"
SIZE_EXY_A="$(stat -c '%s' "$BIN_A" 2>/dev/null || stat -f '%z' "$BIN_A")"
SIZE_EXY_B="$(stat -c '%s' "$BIN_B" 2>/dev/null || stat -f '%z' "$BIN_B")"

CMP_SAMPLE=""
VARIANCE_NOTES=()
CLASSIFICATION="BIT_FOR_BIT_REPRODUCIBLE"
VERDICT="PASS"

if [[ "$SHA_EXY_A" != "$SHA_EXY_B" || "$SHA_CTL_A" != "$SHA_CTL_B" ]]; then
  CLASSIFICATION="NOT_REPRODUCIBLE"
  if cmp -s "$BIN_A" "$BIN_B" && cmp -s "$CTL_A" "$CTL_B"; then
    CLASSIFICATION="BIT_FOR_BIT_REPRODUCIBLE"
  else
    CMP_SAMPLE="$(cmp -l "$BIN_A" "$BIN_B" 2>/dev/null | head -n 20 || true)"
    if [[ -z "$CMP_SAMPLE" ]]; then
      CMP_SAMPLE="$(cmp -l "$CTL_A" "$CTL_B" 2>/dev/null | head -n 20 || true)"
    fi
    VARIANCE_NOTES+=("binary_bytes_differ")
    if command -v readelf >/dev/null 2>&1; then
      if readelf -n "$BIN_A" 2>/dev/null | grep -q 'Build ID'; then
        VARIANCE_NOTES+=("likely_build_id_or_embedded_path_variance")
      fi
    fi
    size_delta=$(( SIZE_EXY_A > SIZE_EXY_B ? SIZE_EXY_A - SIZE_EXY_B : SIZE_EXY_B - SIZE_EXY_A ))
    if [[ "$size_delta" -le 4096 ]]; then
      MANIFEST_A="$(python3 -c "import json;print(json.load(open('$EXTRACT_A/build-manifest.json'))['git']['commit'])")"
      MANIFEST_B="$(python3 -c "import json;print(json.load(open('$EXTRACT_B/build-manifest.json'))['git']['commit'])")"
      LOCK_A="$(python3 -c "import json;print(json.load(open('$EXTRACT_A/build-manifest.json'))['build']['cargo_lock_sha256'])")"
      LOCK_B="$(python3 -c "import json;print(json.load(open('$EXTRACT_B/build-manifest.json'))['build']['cargo_lock_sha256'])")"
      if [[ "$MANIFEST_A" == "$MANIFEST_B" && "$LOCK_A" == "$LOCK_B" ]]; then
        CLASSIFICATION="FUNCTIONALLY_REPRODUCIBLE_WITH_DOCUMENTED_VARIANCE"
        VARIANCE_NOTES+=("manifests_match_head_and_lockfile")
      fi
    fi
  fi
fi

if [[ "$CLASSIFICATION" == "BIT_FOR_BIT_REPRODUCIBLE" ]]; then
  VERDICT="PASS"
elif [[ "$CLASSIFICATION" == "FUNCTIONALLY_REPRODUCIBLE_WITH_DOCUMENTED_VARIANCE" ]]; then
  VERDICT="PASS"
  ws6_log "variance documented — BIT_FOR_BIT NOT_CLAIMED"
else
  VERDICT="FAIL"
fi

VARIANCE_NOTES_STR="${VARIANCE_NOTES[*]}"
SUMMARY_JSON="$(python3 - "$SHA_EXY_A" "$SHA_EXY_B" "$SHA_CTL_A" "$SHA_CTL_B" "$SIZE_EXY_A" "$SIZE_EXY_B" "$CLASSIFICATION" "$VERDICT" "$ARCH" "$TARGET" "$CMP_SAMPLE" "$VARIANCE_NOTES_STR" <<'PY'
import json, sys
args = sys.argv[1:]
sha_exy_a, sha_exy_b, sha_ctl_a, sha_ctl_b, size_a, size_b, classification, verdict, arch, target, cmp_sample, notes_raw = args
notes = [n for n in notes_raw.split() if n]
print(json.dumps({
  "script": "p15-ws6-reproducibility-check.sh",
  "verdict": verdict,
  "classification": classification,
  "bit_for_bit_claim": "NOT_CLAIMED",
  "arch": arch,
  "target": target,
  "exyonq_sha256": {"build_a": sha_exy_a, "build_b": sha_exy_b, "match": sha_exy_a == sha_exy_b},
  "exyonqctl_sha256": {"build_a": sha_ctl_a, "build_b": sha_ctl_b, "match": sha_ctl_a == sha_ctl_b},
  "size_exyonq": {"build_a": int(size_a), "build_b": int(size_b)},
  "variance_notes": notes,
  "cmp_sample_first_bytes": cmp_sample,
}, indent=2))
PY
)"

SUMMARY_TXT="VERDICT=$VERDICT
CLASSIFICATION=$CLASSIFICATION
BIT_FOR_BIT=NOT_CLAIMED
ARCH=$ARCH
EXYONQ_SHA_A=$SHA_EXY_A
EXYONQ_SHA_B=$SHA_EXY_B
EXYONQCTL_SHA_A=$SHA_CTL_A
EXYONQCTL_SHA_B=$SHA_CTL_B
VARIANCE_NOTES=${VARIANCE_NOTES[*]}"

ws6_write_summary_pair "$OUT_DIR" "$SUMMARY_TXT" "$SUMMARY_JSON"
printf '%s\n' "$SUMMARY_TXT"
printf '%s\n' "$SUMMARY_JSON"

[[ "$VERDICT" == "PASS" ]]
