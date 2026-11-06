#!/usr/bin/env bash
# OL-05 end-to-end verification: real OWASP CRS, differential + detection-only +
# per-rule Prometheus metrics.
#
# This is the single reproducible entry point for the local OL-05 evidence. It
# builds the measured binary, pins the CRS revision/corpus/oracle in the
# underlying gate, runs both decision passes and both metrics modes, and fails
# on any mismatch. The "in CI" clause is still separate: this is local execution,
# not GitHub-hosted Actions.
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
EXPECTED_CRS_COMMIT=aabf675fcfc5e489b424b844e6c9f1b39802df69
EXPECTED_CORPUS_SHA=1fb44865af1691e6f92bab41a2690d41c684c4ec9c38f6022edde7309ecb4c54
DEFAULT_REF_IMAGE='owasp/modsecurity-crs@sha256:873ce68a658d2e3b61484a8a24d215f7d15456e7ce5aec5e7114bf000d743365'

die() {
    echo "ol05-crs-verify: $*" >&2
    exit 2
}

CRS_DIR=${EXYONQ_CRS_DIR:-}
if [[ -z "$CRS_DIR" ]]; then
    for candidate in "$ROOT/target-linux/crs-v4.25.0" "$ROOT/.exyonq-local/refs/crs-v4.25.0"; do
        if [[ -f "$candidate/crs-setup.conf.example" && -d "$candidate/rules" ]]; then
            CRS_DIR=$candidate
            break
        fi
    done
fi
[[ -n "$CRS_DIR" ]] || die "set EXYONQ_CRS_DIR to a CRS v4.25.0 checkout"
[[ -f "$CRS_DIR/crs-setup.conf.example" && -d "$CRS_DIR/rules" ]] ||
    die "EXYONQ_CRS_DIR is not a CRS checkout: $CRS_DIR"

ACTUAL_COMMIT=$(git -C "$CRS_DIR" rev-parse HEAD 2>/dev/null) ||
    die "CRS checkout is not a git repository: $CRS_DIR"
[[ "$ACTUAL_COMMIT" == "$EXPECTED_CRS_COMMIT" ]] ||
    die "CRS commit mismatch: expected $EXPECTED_CRS_COMMIT, got $ACTUAL_COMMIT"

CORPUS="$ROOT/scripts/gates/fixtures/crs-v4.25.0-corpus.json"
[[ -f "$CORPUS" ]] || die "corpus not found: $CORPUS"
if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL_CORPUS_SHA=$(sha256sum "$CORPUS" | awk '{print $1}')
else
    ACTUAL_CORPUS_SHA=$(shasum -a 256 "$CORPUS" | awk '{print $1}')
fi
[[ "$ACTUAL_CORPUS_SHA" == "$EXPECTED_CORPUS_SHA" ]] ||
    die "corpus hash mismatch: expected $EXPECTED_CORPUS_SHA, got $ACTUAL_CORPUS_SHA"

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
[[ -n "$PYTHON" ]] || die "Python >= 3.10 is required (set PYTHON)"
export PYTHON

EXYONQ_BIN=${EXYONQ_BIN:-$ROOT/target/debug/exyonq}
export EXYONQ_BIN
export CRS_REF_IMAGE=${CRS_REF_IMAGE:-$DEFAULT_REF_IMAGE}

RUN=${OL05_RUN_DIR:-}
if [[ -z "$RUN" ]]; then
    RUN=$(mktemp -d "$ROOT/.exyonq-local/evidence/ol05-verify-$(date -u +%Y%m%dT%H%M%SZ)-XXXXXX")
else
    mkdir -p "$RUN"
    [[ -z "$(ls -A "$RUN" 2>/dev/null)" ]] || die "OL05_RUN_DIR is not empty: $RUN"
fi

if command -v sha256sum >/dev/null 2>&1; then
    sha256_file() { sha256sum "$1" | awk '{print $1}'; }
else
    sha256_file() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

FAIL=0
run_step() {
    local name=$1
    shift
    local log="$RUN/$name.log"
    printf '=== %s ===\n' "$name" | tee "$log"
    set +e
    "$@" 2>&1 | tee -a "$log"
    local status=${PIPESTATUS[0]}
    set -e
    printf '%s_EXIT=%s\n' "$name" "$status" | tee -a "$RUN/summary.log"
    if [[ "$status" -ne 0 ]]; then
        FAIL=1
    fi
    return "$status"
}

check_log() {
    local log=$1
    local pattern=$2
    local description=$3
    if ! grep -Eq "$pattern" "$RUN/$log"; then
        echo "missing $description in $RUN/$log" >&2
        FAIL=1
    fi
}

if ! run_step build cargo build -p exyonq --bin exyonq --locked; then
    printf 'OL05_VERIFY=FAIL (build)\n' | tee -a "$RUN/summary.log"
    echo "RUN=$RUN"
    exit 1
fi

printf 'binary=%s binary_sha256=%s\n' "$EXYONQ_BIN" "$(sha256_file "$EXYONQ_BIN")" |
    tee -a "$RUN/summary.log"
"$EXYONQ_BIN" --version | tee -a "$RUN/summary.log"

printf 'crs_commit=%s corpus_sha256=%s reference_image=%s\n' \
    "$ACTUAL_COMMIT" "$ACTUAL_CORPUS_SHA" "$CRS_REF_IMAGE" | tee -a "$RUN/summary.log"

run_step block env \
    EXYONQ_CRS_DIR="$CRS_DIR" \
    CRS_DIFF_WORK="$ROOT/target/ol05-crs-block" \
    PYTHON="$PYTHON" \
    bash "$ROOT/scripts/gates/crs-diff-gate.sh" || true

run_step dryrun env \
    CRS_DRY_RUN=1 \
    EXYONQ_CRS_DIR="$CRS_DIR" \
    CRS_DIFF_WORK="$ROOT/target/ol05-crs-dryrun" \
    PYTHON="$PYTHON" \
    bash "$ROOT/scripts/gates/crs-diff-gate.sh" || true

run_step metrics-block env \
    EXYONQ_CRS_DIR="$CRS_DIR" \
    EXYONQ_BIN="$EXYONQ_BIN" \
    bash "$ROOT/scripts/e2e/ol05-waf-metrics-e2e.sh" block || true

run_step metrics-detection-only env \
    EXYONQ_CRS_DIR="$CRS_DIR" \
    EXYONQ_BIN="$EXYONQ_BIN" \
    bash "$ROOT/scripts/e2e/ol05-waf-metrics-e2e.sh" detection-only || true

run_step files-xml env \
    PYTHON="$PYTHON" \
    "$PYTHON" -u "$ROOT/scripts/gates/crs-files-xml-diff.py" || true

check_log block.log '^failures=0/4628$' 'block failures=0/4628'
check_log block.log '^excluded_with_evidence=33/4628$' 'block 33 evidence-backed exclusions'
check_log block.log '^mode=block$' 'block mode marker'
check_log dryrun.log '^failures=0/4628$' 'dry-run failures=0/4628'
check_log dryrun.log '^excluded_with_evidence=0/4628$' 'dry-run zero exclusions'
check_log dryrun.log '^mode=dry-run$' 'dry-run mode marker'
check_log dryrun.log 'violations_942100=[1-9][0-9]*' '942100 per-rule metric'
check_log dryrun.log 'violations_949110=[1-9][0-9]*' '949110 anomaly metric'
check_log dryrun.log 'would_block=[1-9][0-9]*' 'would-block signal'
check_log metrics-block.log '^RESULT=PASS OL-05 WAF metrics mode=block$' 'block metrics PASS'
check_log metrics-block.log '^exyonq_waf_violations_total\{rule_id="942100"\} [1-9][0-9]*$' 'block 942100 metric'
check_log metrics-detection-only.log '^RESULT=PASS OL-05 WAF metrics mode=detection-only$' 'detection-only metrics PASS'
check_log metrics-detection-only.log '^exyonq_waf_violations_total\{rule_id="942100"\} [1-9][0-9]*$' 'detection-only 942100 metric'
check_log files-xml.log '^failures=0$' 'FILES/XML importer differential'

{
    printf 'OL05_CRS_VERIFY summary\n'
    printf 'binary_sha256=%s\n' "$(sha256_file "$EXYONQ_BIN")"
    printf 'crs_commit=%s\n' "$ACTUAL_COMMIT"
    printf 'corpus_sha256=%s\n' "$ACTUAL_CORPUS_SHA"
    printf 'reference_image=%s\n' "$CRS_REF_IMAGE"
    grep -E '^(failures|excluded_with_evidence|mode|dry_run_evidence)=' "$RUN/block.log" "$RUN/dryrun.log" || true
    grep -E '^exyonq_waf_violations_total' "$RUN/metrics-block.log" "$RUN/metrics-detection-only.log" || true
} | tee -a "$RUN/summary.log"

if [[ "$FAIL" -ne 0 ]]; then
    printf 'OL05_VERIFY=FAIL\n' | tee -a "$RUN/summary.log"
    STATUS=1
else
    printf 'OL05_VERIFY=PASS\n' | tee -a "$RUN/summary.log"
    STATUS=0
fi

# Hash after the final status line: hashing summary.log earlier leaves a stale
# digest in the manifest (the exact regression this ordering prevents).
(
    cd "$RUN"
    for file in build.log block.log dryrun.log metrics-block.log metrics-detection-only.log files-xml.log summary.log; do
        if [[ -f "$file" ]]; then
            printf '%s  %s\n' "$(sha256_file "$file")" "$file"
        fi
    done
) | tee "$RUN/SHA256SUMS.txt"

echo "RUN=$RUN"
exit "$STATUS"
