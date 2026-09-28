#!/usr/bin/env bash
# PS2-R5B — compare frozen r5 source identity vs current tree (Mac or remote).
set -euo pipefail

WORKSPACE="${1:-.}"
R5_FREEZE="${2:-$WORKSPACE/docs/benchmarks/platform-split/ps2-results/netcup-amd64-r5-remediation/artifact-freeze.json}"

cd "$WORKSPACE"
[[ -f "$R5_FREEZE" ]] || {
  echo "ERROR: missing r5 freeze: $R5_FREEZE" >&2
  exit 2
}

R5_BUNDLE="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['bundle_hash'])")"
R5_FP="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['bundle_only_fingerprint'])")"
R5_HEAD="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['head'])")"
R5_STATUS="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['status_hash'])")"

FP_OUT="$(bash "$WORKSPACE/scripts/remote/ps2-tree-fingerprint.sh" "$WORKSPACE")"
R5B_BUNDLE="$(echo "$FP_OUT" | awk -F= '/^bundle_hash=/{print $2; exit}')"
R5B_FP="$(echo "$FP_OUT" | awk -F= '/^bundle_only_fingerprint=/{print $2; exit}')"
R5B_HEAD="$(echo "$FP_OUT" | awk -F= '/^head=/{print $2; exit}')"
R5B_STATUS="$(echo "$FP_OUT" | awk -F= '/^status_hash=/{print $2; exit}')"

MATCH=NO
[[ "$R5_BUNDLE" == "$R5B_BUNDLE" && "$R5_FP" == "$R5B_FP" && "$R5_HEAD" == "$R5B_HEAD" && "$R5_STATUS" == "$R5B_STATUS" ]] && MATCH=YES

R5_CARGO_LOCK="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['cargo_lock_sha256'])")"
R5_CARGO_TOML="$(python3 -c "import json; print(json.load(open('$R5_FREEZE'))['identity_evidence_r5']['cargo_toml_sha256'])")"
R5B_CARGO_LOCK="$(echo "$FP_OUT" | awk -F= '/^cargo_lock_sha256=/{print $2; exit}')"
R5B_CARGO_TOML="$(echo "$FP_OUT" | awk -F= '/^cargo_toml_sha256=/{print $2; exit}')"
PRODUCT_MATCH=NO
[[ "$R5_HEAD" == "$R5B_HEAD" && "$R5_STATUS" == "$R5B_STATUS" && "$R5_CARGO_LOCK" == "$R5B_CARGO_LOCK" && "$R5_CARGO_TOML" == "$R5B_CARGO_TOML" ]] && PRODUCT_MATCH=YES

echo "R5_SOURCE_HASH=$R5_BUNDLE"
echo "R5B_SOURCE_HASH=$R5B_BUNDLE"
echo "R5_BUNDLE_ONLY_FINGERPRINT=$R5_FP"
echo "R5B_BUNDLE_ONLY_FINGERPRINT=$R5B_FP"
echo "R5_HEAD=$R5_HEAD"
echo "R5B_HEAD=$R5B_HEAD"
echo "R5_CARGO_LOCK_SHA256=$R5_CARGO_LOCK"
echo "R5B_CARGO_LOCK_SHA256=$R5B_CARGO_LOCK"
echo "R5_PRODUCT_IDENTITY=$PRODUCT_MATCH"
echo "R5_R5B_SOURCE_IDENTITY=$MATCH"
echo "IDENTITY_DRIFT_NOTE=strict_bundle_mismatch_expected_after_R5A_harness_script_edits"

if [[ "$MATCH" != "YES" ]]; then
  echo "PS2_R5B=BLOCKED_BY_SOURCE_MISMATCH"
  exit 1
fi

echo "PS2_R5B_SOURCE_IDENTITY=PASS"
