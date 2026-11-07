#!/usr/bin/env bash
# Negative control for the artifact guard used by the E2E job.
#
# The artifact-bound E2E runner records both built binary hashes, re-verifies
# them after tests and after each E2E, and rejects a log whose binary_sha256 or
# compat_sha256 does not match. This control uses the same guard with real
# copies of the built binaries: it must accept the intact copies and reject a
# tampered binary, a tampered exyonq log hash and a tampered exyonq-compat log
# hash. If the guard regresses to accepting any tampered artifact, this step
# fails and the E2E job fails with it.
#
# This is a regression control for the guard, not E2E evidence.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
COMPAT="${EXYONQ_COMPAT_BIN:-$ROOT/target/debug/exyonq-compat}"
GUARD="$ROOT/scripts/gates/artifact-integrity.sh"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-artifact-negative.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

last_output=""
fail() {
  echo "ARTIFACT_NEGATIVE_CONTROL=FAIL $*" >&2
  exit 1
}

expect_rc() {
  local expected=$1
  shift
  local output rc
  set +e
  output="$("$@" 2>&1)"
  rc=$?
  set -e
  printf '%s\n' "$output"
  if [[ "$rc" -ne "$expected" ]]; then
    fail "expected rc=$expected, got rc=$rc for: $*"
  fi
  last_output="$output"
}

[[ -f "$BIN" ]] || fail "missing built binary: $BIN"
[[ -f "$COMPAT" ]] || fail "missing built compat binary: $COMPAT"
[[ -f "$GUARD" ]] || fail "missing artifact guard: $GUARD"

STATE="$TMP/artifact-integrity.tsv"
PRISTINE="$TMP/exyonq.pristine"
COMPAT_PRISTINE="$TMP/exyonq-compat.pristine"
TAMPERED="$TMP/exyonq.tampered"
LOG="$TMP/e2e-bound.log"
TAMPERED_COMPAT_LOG="$TMP/e2e-tampered-compat.log"
TAMPERED_BINARY_LOG="$TMP/e2e-tampered-binary.log"
UNBOUND_LOG="$TMP/e2e-unbound.log"
: >"$STATE"

cp "$BIN" "$PRISTINE"
cp "$COMPAT" "$COMPAT_PRISTINE"
if ! cmp -s "$BIN" "$PRISTINE"; then
  fail "pristine exyonq copy differs from the built binary"
fi
if ! cmp -s "$COMPAT" "$COMPAT_PRISTINE"; then
  fail "pristine exyonq-compat copy differs from the built compat binary"
fi

# Positive control: both intact copies must be accepted before tampering.
expect_rc 0 bash "$GUARD" record "$STATE" exyonq "$PRISTINE"
if [[ "$last_output" != *"ARTIFACT_RECORD"* ]]; then
  fail "guard did not record the pristine exyonq copy"
fi
expect_rc 0 bash "$GUARD" record "$STATE" exyonq-compat "$COMPAT_PRISTINE"
if [[ "$last_output" != *"ARTIFACT_RECORD"* ]]; then
  fail "guard did not record the pristine exyonq-compat copy"
fi

ORIGINAL_SHA="$(awk -F'\t' '$1 == "exyonq" { hash = $2 } END { print hash }' "$STATE")"
COMPAT_SHA="$(awk -F'\t' '$1 == "exyonq-compat" { hash = $2 } END { print hash }' "$STATE")"
[[ "$ORIGINAL_SHA" =~ ^[0-9a-f]{64}$ ]] || fail "guard recorded an invalid exyonq sha256: ${ORIGINAL_SHA:-<empty>}"
[[ "$COMPAT_SHA" =~ ^[0-9a-f]{64}$ ]] || fail "guard recorded an invalid exyonq-compat sha256: ${COMPAT_SHA:-<empty>}"
if [[ "$ORIGINAL_SHA" == "$COMPAT_SHA" ]]; then
  fail "test binaries have the same hash; cannot prove per-label log binding"
fi

expect_rc 0 bash "$GUARD" verify "$STATE" exyonq "$PRISTINE"
if [[ "$last_output" != *"ARTIFACT_MATCH"* ]]; then
  fail "guard did not accept the pristine exyonq copy"
fi
expect_rc 0 bash "$GUARD" verify "$STATE" exyonq-compat "$COMPAT_PRISTINE"
if [[ "$last_output" != *"ARTIFACT_MATCH"* ]]; then
  fail "guard did not accept the pristine exyonq-compat copy"
fi

# Tamper with a copy of the exyonq binary. Appending bytes preserves the
# original artifact for comparison while guaranteeing a different digest.
cp "$PRISTINE" "$TAMPERED"
printf 'tampered\n' >>"$TAMPERED"
if cmp -s "$PRISTINE" "$TAMPERED"; then
  fail "binary tamper did not change the copied artifact"
fi
expect_rc 3 bash "$GUARD" verify "$STATE" exyonq "$TAMPERED"
if [[ "$last_output" != *"ARTIFACT_DRIFT"* ]]; then
  fail "guard rejected the tampered binary for the wrong reason"
fi

# One log carries both hashes; each label must bind only its own field.
printf 'binary_sha256=%s\ncompat_sha256=%s\nRESULT=PASS\n' "$ORIGINAL_SHA" "$COMPAT_SHA" >"$LOG"
expect_rc 0 bash "$GUARD" log "$STATE" exyonq "$LOG"
if [[ "$last_output" != *"ARTIFACT_LOG_BOUND"* ]]; then
  fail "guard did not accept the bound exyonq log"
fi
expect_rc 0 bash "$GUARD" log "$STATE" exyonq-compat "$LOG" compat_sha256
if [[ "$last_output" != *"ARTIFACT_LOG_BOUND"* ]]; then
  fail "guard did not accept the bound exyonq-compat log"
fi

FAKE_SHA="ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"

# Tamper only the compat hash: compat must fail, exyonq must still bind.
sed "s/compat_sha256=$COMPAT_SHA/compat_sha256=$FAKE_SHA/" "$LOG" >"$TAMPERED_COMPAT_LOG"
expect_rc 3 bash "$GUARD" log "$STATE" exyonq-compat "$TAMPERED_COMPAT_LOG" compat_sha256
if [[ "$last_output" != *"ARTIFACT_LOG_DRIFT"* ]]; then
  fail "guard rejected the tampered exyonq-compat log for the wrong reason"
fi
expect_rc 0 bash "$GUARD" log "$STATE" exyonq "$TAMPERED_COMPAT_LOG"
if [[ "$last_output" != *"ARTIFACT_LOG_BOUND"* ]]; then
  fail "exyonq log binding changed when only compat_sha256 was tampered"
fi

# Tamper only the exyonq hash: exyonq must fail, compat must still bind.
sed "s/binary_sha256=$ORIGINAL_SHA/binary_sha256=$FAKE_SHA/" "$LOG" >"$TAMPERED_BINARY_LOG"
expect_rc 3 bash "$GUARD" log "$STATE" exyonq "$TAMPERED_BINARY_LOG"
if [[ "$last_output" != *"ARTIFACT_LOG_DRIFT"* ]]; then
  fail "guard rejected the tampered exyonq log for the wrong reason"
fi
expect_rc 0 bash "$GUARD" log "$STATE" exyonq-compat "$TAMPERED_BINARY_LOG" compat_sha256
if [[ "$last_output" != *"ARTIFACT_LOG_BOUND"* ]]; then
  fail "exyonq-compat log binding changed when only binary_sha256 was tampered"
fi

# A log without either binding must also be rejected for both labels.
printf 'RESULT=PASS\n' >"$UNBOUND_LOG"
expect_rc 4 bash "$GUARD" log "$STATE" exyonq "$UNBOUND_LOG"
if [[ "$last_output" != *"ARTIFACT_LOG_UNBOUND"* ]]; then
  fail "guard did not reject the unbound exyonq log"
fi
expect_rc 4 bash "$GUARD" log "$STATE" exyonq-compat "$UNBOUND_LOG" compat_sha256
if [[ "$last_output" != *"ARTIFACT_LOG_UNBOUND"* ]]; then
  fail "guard did not reject the unbound exyonq-compat log"
fi

echo "ARTIFACT_NEGATIVE_CONTROL_BINARY=PASS expected=ARTIFACT_DRIFT rc=3"
echo "ARTIFACT_NEGATIVE_CONTROL_LOG=PASS expected=ARTIFACT_LOG_DRIFT rc=3"
echo "ARTIFACT_NEGATIVE_CONTROL_COMPAT_LOG=PASS expected=ARTIFACT_LOG_DRIFT rc=3"
echo "ARTIFACT_NEGATIVE_CONTROL_UNBOUND_LOG=PASS expected=ARTIFACT_LOG_UNBOUND rc=4"
echo "ARTIFACT_NEGATIVE_CONTROL=PASS"
