#!/usr/bin/env bash
# Build a disposable static-mvp dual-Linux admission transfer package from
# tracked lab sources only. Generated artifacts live under
# /Volumes/Lexar/Cursor/temp/<OUT_NAME>/ — never under the canonical tree.
#
# USAGE:
#   bash scripts/gates/build-static-mvp-admit-package.sh [--out-name NAME]
#
# Outputs (under OUT_ROOT):
#   pkg/candidate.tgz          — source tree for remote build
#   pkg/host-admit.sh          — generated copy of scripts/gates/static-mvp-host-admit.sh
#   local-meta/candidate.manifest
#   local-meta/provenance.env
#   DELIVERY.env (skeleton)
set -euo pipefail
LC_ALL=C
export LC_ALL
export PATH="/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:${PATH}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAB="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUT_NAME=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out-name) OUT_NAME="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

OUT_NAME="${OUT_NAME:-exyonq-static-mvp-admit-$(date -u +%Y%m%dT%H%M%SZ)}"
OUT_ROOT="/Volumes/Lexar/Cursor/temp/${OUT_NAME}"
if [[ -e "$OUT_ROOT" ]]; then
  echo "ERROR: OUT_ROOT already exists: $OUT_ROOT" >&2
  exit 2
fi

HOST_ADMIT_SRC="$LAB/scripts/gates/static-mvp-host-admit.sh"
PACKAGE_BUILDER="$LAB/scripts/gates/build-static-mvp-admit-package.sh"
[[ -f "$HOST_ADMIT_SRC" ]] || { echo "ERROR: missing tracked host-admit source"; exit 2; }
[[ -f "$PACKAGE_BUILDER" ]] || { echo "ERROR: missing tracked package builder"; exit 2; }

sha256() { /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'; }

# Product candidate files (must match prior PRODUCT_CANDIDATE_DIFF_SHA256 for the
# three modified tracked .rs files; new files included as CANDIDATE rows).
PRODUCT_PATHS=(
  cli/exyonqctl/src/main.rs
  compat/nginx/src/lib.rs
  compat/nginx/src/map_ir.rs
  compat/nginx/src/static_mvp.rs
  compat/nginx/tests/static_mvp.rs
  compat/nginx/NGINX_STATIC_IMPORT_MVP.md
)
HARNESS_PATHS=(
  scripts/gates/core-regression-gate.sh
  scripts/gates/static-mvp-host-admit.sh
  scripts/gates/build-static-mvp-admit-package.sh
  scripts/gates/deploy-static-mvp-admit.sh
  scripts/e2e/03b-wordpress-profile.sh
  scripts/e2e/04-operations.sh
)

for p in "${PRODUCT_PATHS[@]}" "${HARNESS_PATHS[@]}"; do
  [[ -f "$LAB/$p" ]] || { echo "ERROR: missing required path $p"; exit 2; }
done

DIFF_SHA="$(
  cd "$LAB"
  git diff -- cli/exyonqctl/src/main.rs compat/nginx/src/lib.rs compat/nginx/src/map_ir.rs \
    | /usr/bin/shasum -a 256 | /usr/bin/awk '{print $1}'
)"
EXPECTED_DIFF_SHA="400a7b7a02404cebaef4891cd2be48e8f8c67c20af19ed47621c6701676ba470"
if [[ "$DIFF_SHA" != "$EXPECTED_DIFF_SHA" ]]; then
  echo "ERROR: product diff SHA drift want=$EXPECTED_DIFF_SHA got=$DIFF_SHA" >&2
  exit 2
fi

BASE_HEAD="$(cd "$LAB" && git rev-parse HEAD)"
BASE_TREE="$(cd "$LAB" && git rev-parse 'HEAD^{tree}')"
UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

mkdir -p "$OUT_ROOT"/{pkg,local-meta,staging,remote}

# Stage full tree from HEAD, then overlay working-tree candidate/harness paths.
# Temp-only staging — not a publication export.
(
  cd "$LAB"
  git archive --format=tar HEAD
) | tar -xf - -C "$OUT_ROOT/staging"

# Overlay product + harness paths from the live worktree (includes untracked).
for p in "${PRODUCT_PATHS[@]}" "${HARNESS_PATHS[@]}"; do
  mkdir -p "$OUT_ROOT/staging/$(dirname "$p")"
  cp -p "$LAB/$p" "$OUT_ROOT/staging/$p"
done

# Generated runner entrypoint: byte-identical copy of tracked host-admit source.
cp -p "$HOST_ADMIT_SRC" "$OUT_ROOT/pkg/host-admit.sh"
chmod +x "$OUT_ROOT/pkg/host-admit.sh"

TRACKED_HOST_ADMIT_SHA="$(sha256 "$HOST_ADMIT_SRC")"
GENERATED_HOST_ADMIT_SHA="$(sha256 "$OUT_ROOT/pkg/host-admit.sh")"
if [[ "$TRACKED_HOST_ADMIT_SHA" != "$GENERATED_HOST_ADMIT_SHA" ]]; then
  echo "ERROR: generated host-admit diverged from tracked source" >&2
  exit 2
fi

# Prove isolated-socket logic is present in generated runner.
grep -q 'EXYONQ_CONTROL_SOCKET' "$OUT_ROOT/pkg/host-admit.sh"
grep -q 'exyonq-smvp-ctrl-' "$OUT_ROOT/pkg/host-admit.sh"
grep -q 'unset EXYONQ_CONTROL_SOCKET' "$OUT_ROOT/pkg/host-admit.sh"

(
  cd "$OUT_ROOT/staging"
  /usr/bin/tar -czf "$OUT_ROOT/pkg/candidate.tgz" .
)
TGZ_SHA="$(sha256 "$OUT_ROOT/pkg/candidate.tgz")"

{
  echo "BASE_HEAD=$BASE_HEAD"
  echo "BASE_TREE=$BASE_TREE"
  echo "CREATED_UTC=$UTC"
  echo "GENERATOR=scripts/gates/build-static-mvp-admit-package.sh"
  echo "HOST_ADMIT_TRACKED_SOURCE=scripts/gates/static-mvp-host-admit.sh"
  echo "HOST_ADMIT_GENERATED_FROM_TRACKED_SOURCE=PASS"
  echo "TEMP_PKG_CANONICAL_DEPENDENCY=NO"
  for p in "${PRODUCT_PATHS[@]}" "${HARNESS_PATHS[@]}"; do
    echo "CANDIDATE $(sha256 "$OUT_ROOT/staging/$p") $p"
  done
  echo "CANDIDATE_DIFF_SHA256=$DIFF_SHA"
  echo "CANDIDATE_TGZ_SHA256=$TGZ_SHA"
  echo "HOST_ADMIT_SHA256=$GENERATED_HOST_ADMIT_SHA"
  echo "TRACKED_HOST_ADMIT_SHA256=$TRACKED_HOST_ADMIT_SHA"
} >"$OUT_ROOT/local-meta/candidate.manifest"
MAN_SHA="$(sha256 "$OUT_ROOT/local-meta/candidate.manifest")"
echo "CANDIDATE_MANIFEST_SHA256=$MAN_SHA" >>"$OUT_ROOT/local-meta/candidate.manifest"

{
  echo "OUT_ROOT=$OUT_ROOT"
  echo "BASE_HEAD=$BASE_HEAD"
  echo "BASE_TREE=$BASE_TREE"
  echo "PRODUCT_CANDIDATE_DIFF_SHA256=$DIFF_SHA"
  echo "REBUILT_PACKAGE_MANIFEST_SHA256=$MAN_SHA"
  echo "GENERATED_HOST_ADMIT_SHA256=$GENERATED_HOST_ADMIT_SHA"
  echo "TRACKED_SOURCE_PATHS=scripts/gates/static-mvp-host-admit.sh"
  echo "TRACKED_GENERATOR_PATHS=scripts/gates/build-static-mvp-admit-package.sh"
  echo "HOST_ADMIT_GENERATED_FROM_TRACKED_SOURCE=PASS"
  echo "TEMP_PKG_CANONICAL_DEPENDENCY=NO"
  echo "CANDIDATE_TGZ_SHA256=$TGZ_SHA"
  echo "GENERATION_COMMAND=bash scripts/gates/build-static-mvp-admit-package.sh --out-name $OUT_NAME"
} | tee "$OUT_ROOT/local-meta/provenance.env"

echo "OUT_ROOT=$OUT_ROOT"
echo "CANDIDATE_MANIFEST_SHA256=$MAN_SHA"
exit 0
