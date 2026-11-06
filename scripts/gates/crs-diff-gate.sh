#!/usr/bin/env bash
# OL-05 differential gate: ExyonQ vs ModSecurity+CRS.
#
# Pinned reference:
#   CRS v4.25.0, commit aabf675fcfc5e489b424b844e6c9f1b39802df69
#   corpus SHA-256 1fb44865af1691e6f92bab41a2690d41c684c4ec9c38f6022edde7309ecb4c54
#
# The CRS checkout is not vendored in this repository. Point EXYONQ_CRS_DIR at
# a clean checkout of that exact commit. The corpus is vendored under
# scripts/gates/fixtures so the requests do not drift with the upstream repo.
#
# Current status (2026-11-04): the gate is green on native Netcup amd64 and
# Oracle arm64 over CRS v4.25.0: failures=0/4628 with 33 evidence-backed
# exclusions. The "in CI" clause remains open because
# .github/workflows/crs-diff.yml has not run in GitHub Actions; it requires a
# push to a configured upstream. The workflow obtains the pinned tree with
# scripts/gates/crs-fetch.sh (repository + tag + commit, fail-closed) and pins
# the reference engine image by digest, so the differential cannot drift with a
# floating tag. The local default is pinned to the same digest. The exclusions
# cover parser/reference differences only; do not add a product defect to that
# list.
#
# Set CRS_DRY_RUN=1 for the monitor-mode pass: ExyonQ must not block any of the
# 4628 requests (no 403/429) and must still record real CRS rule hits; the
# block-mode pass above is what compares the decision against ModSecurity.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
EXPECTED_CRS_COMMIT=aabf675fcfc5e489b424b844e6c9f1b39802df69
EXPECTED_CORPUS_SHA=1fb44865af1691e6f92bab41a2690d41c684c4ec9c38f6022edde7309ecb4c54
DEFAULT_REF_IMAGE='owasp/modsecurity-crs@sha256:873ce68a658d2e3b61484a8a24d215f7d15456e7ce5aec5e7114bf000d743365'
REF_IMAGE=${CRS_REF_IMAGE:-$DEFAULT_REF_IMAGE}
CRS_DRY_RUN=${CRS_DRY_RUN:-0}
CORPUS=${1:-"$ROOT/scripts/gates/fixtures/crs-v4.25.0-corpus.json"}
CRS_DIR=${EXYONQ_CRS_DIR:-}
EXCLUSIONS=${EXCLUSIONS:-"$ROOT/scripts/gates/fixtures/ol05-exclusions.json"}

die() {
    echo "crs-diff-gate: $*" >&2
    exit 2
}

case "$CRS_DRY_RUN" in
    0|1) ;;
    *) die "CRS_DRY_RUN must be 0 or 1" ;;
esac
if [[ "$REF_IMAGE" != *@sha256:* ]]; then
    die "CRS_REF_IMAGE must be pinned by digest"
fi
export CRS_REF_IMAGE="$REF_IMAGE"

if [[ -z "$CRS_DIR" ]]; then
    die "set EXYONQ_CRS_DIR to a CRS v4.25.0 checkout"
fi
if [[ ! -f "$CRS_DIR/crs-setup.conf.example" || ! -d "$CRS_DIR/rules" ]]; then
    die "EXYONQ_CRS_DIR is not a CRS checkout: $CRS_DIR"
fi
if [[ ! -f "$CORPUS" ]]; then
    die "corpus not found: $CORPUS"
fi
if [[ ! -f "$EXCLUSIONS" ]]; then
    die "exclusions not found: $EXCLUSIONS"
fi

ACTUAL_COMMIT=$(git -C "$CRS_DIR" rev-parse HEAD 2>/dev/null) || die "CRS checkout is not a git repository"
if [[ "$ACTUAL_COMMIT" != "$EXPECTED_CRS_COMMIT" ]]; then
    die "CRS commit mismatch: expected $EXPECTED_CRS_COMMIT, got $ACTUAL_COMMIT"
fi

ACTUAL_SHA=$(python3 - "$CORPUS" <<'PY'
import hashlib
import sys

print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())
PY
)
if [[ "$ACTUAL_SHA" != "$EXPECTED_CORPUS_SHA" ]]; then
    die "corpus hash mismatch: expected $EXPECTED_CORPUS_SHA, got $ACTUAL_SHA"
fi

PYTHON=${PYTHON:-}
if [[ -z "$PYTHON" ]]; then
    for candidate in python3.13 python3.12 python3.11 python3.10 python3; do
        if command -v "$candidate" >/dev/null 2>&1 &&
            "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)' 2>/dev/null; then
            PYTHON=$candidate
            break
        fi
    done
fi
if [[ -z "$PYTHON" ]]; then
    die "Python >= 3.10 is required (set PYTHON)"
fi

BIN="$ROOT/target/debug/exyonq"
[[ -x "$BIN" ]] || die "missing measured binary: $BIN"
if command -v sha256sum >/dev/null 2>&1; then
    BIN_SHA256="$(sha256sum "$BIN" | awk '{print $1}')"
else
    BIN_SHA256="$(shasum -a 256 "$BIN" | awk '{print $1}')"
fi
echo "binary=$BIN binary_sha256=$BIN_SHA256"

ARGS=(
    --crs "$CRS_DIR"
    --corpus "$CORPUS"
    --exclusions "$EXCLUSIONS"
    --threshold 5
)
if [[ "$CRS_DRY_RUN" == "1" ]]; then
    ARGS+=(--dry-run)
fi
echo "crs_diff mode=$([[ "$CRS_DRY_RUN" == "1" ]] && echo dry-run || echo block) crs_commit=$ACTUAL_COMMIT corpus_sha256=$ACTUAL_SHA reference_image=$REF_IMAGE"
exec "$PYTHON" -u "$ROOT/scripts/gates/crs-diff.py" "${ARGS[@]}"
