#!/usr/bin/env bash
# P1.6-WS4 — candidate artifact documentation consistency.
# Does NOT declare RC, tag, publish, or sign.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

fail=0
pass() { echo "PASS: $*"; }
fail_msg() { echo "FAIL: $*" >&2; fail=1; }

TARGET_RC="0.4.0-rc.1"
DEV_VER="0.4.0"
SRC_HEAD="5182e7988ec3f27b009bd3e0ca52a441f2eb58f3"

echo "=== p16-ws4-verify-release-docs ==="

required=(
  docs/release/p1.6-ws4-source-freeze.md
  docs/release/p1.6-ws4-artifact-matrix.md
  docs/release/p1.6-ws4-rc-candidate-manifest.md
  docs/release/p1.6-ws4-dual-arch-reconciliation.md
)
for f in "${required[@]}"; do
  [[ -f "$f" ]] && pass "exists $f" || fail_msg "missing $f"
done

for f in "${required[@]}"; do
  grep -q "RC_STATUS.*=.*NOT_DECLARED" "$f" && pass "RC_STATUS NOT_DECLARED in $f" \
    || fail_msg "RC_STATUS NOT_DECLARED missing in $f"
  grep -q "PUBLICATION_STATUS.*=.*FORBIDDEN" "$f" && pass "PUBLICATION_STATUS FORBIDDEN in $f" \
    || fail_msg "PUBLICATION_STATUS FORBIDDEN missing in $f"
done

grep -q "$SRC_HEAD" docs/release/p1.6-ws4-source-freeze.md \
  && pass "freeze SOURCE_HEAD" || fail_msg "freeze SOURCE_HEAD mismatch"
grep -q "$SRC_HEAD" docs/release/p1.6-ws4-rc-candidate-manifest.md \
  && pass "candidate SOURCE_HEAD" || fail_msg "candidate SOURCE_HEAD mismatch"
grep -q "TARGET_RC_VERSION.*=.*${TARGET_RC}" docs/release/p1.6-ws4-artifact-matrix.md \
  && pass "matrix TARGET_RC" || fail_msg "matrix TARGET_RC"
grep -q "SOURCE_VERSION.*=.*${DEV_VER}" docs/release/p1.6-ws4-artifact-matrix.md \
  && pass "matrix SOURCE_VERSION" || fail_msg "matrix SOURCE_VERSION"
grep -q "RC_ARTIFACT_MATRIX = COMPLETE" docs/release/p1.6-ws4-artifact-matrix.md \
  && pass "matrix COMPLETE" || fail_msg "matrix not COMPLETE"
grep -q "RC_CANDIDATE_MANIFEST = COMPLETE" docs/release/p1.6-ws4-rc-candidate-manifest.md \
  && pass "candidate COMPLETE" || fail_msg "candidate not COMPLETE"
grep -q "BIT_FOR_BIT_CLAIM = NOT_CLAIMED" docs/release/p1.6-ws4-rc-candidate-manifest.md \
  && pass "bit-for-bit not claimed" || fail_msg "bit-for-bit claim missing"
grep -q "ARM64_NFPM" docs/release/p1.6-ws4-artifact-matrix.md \
  && pass "arm64 nfpm limit documented" || fail_msg "arm64 nfpm limit missing"
grep -q "AUTO_OPEN_WS5 = NO" docs/release/p1.6-ws4-dual-arch-reconciliation.md \
  && pass "WS5 not auto-opened" || fail_msg "WS5 auto-open check"

if [[ "$fail" -eq 0 ]]; then
  echo "RELEASE_DOC_CONSISTENCY_WS4=PASS"
  exit 0
fi
echo "RELEASE_DOC_CONSISTENCY_WS4=FAIL"
exit 1
