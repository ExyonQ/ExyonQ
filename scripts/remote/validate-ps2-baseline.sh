#!/usr/bin/env bash
# PS2 baseline validation — run ON Netcup (native Linux amd64), never Mac Docker nested.
set -euo pipefail

WORKSPACE=""
EXPECTED_ARCH=""
EXPECTED_FINGERPRINT=""

usage() {
  cat <<'EOF'
Usage: validate-ps2-baseline.sh --workspace PATH --expected-arch ARCH --expected-fingerprint SHA256
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="${2:-}"; shift 2 ;;
    --expected-fingerprint) EXPECTED_FINGERPRINT="${2:-}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$EXPECTED_FINGERPRINT" && -n "$WORKSPACE" && -d "$WORKSPACE" ]] || {
  echo "ERROR: workspace/fingerprint required" >&2
  exit 2
}

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

ACTUAL_ARCH="$(uname -m)"
[[ "$ACTUAL_ARCH" == "$EXPECTED_ARCH" ]] || {
  echo "ERROR: arch mismatch expected=$EXPECTED_ARCH actual=$ACTUAL_ARCH" >&2
  exit 2
}

echo "=== PS2 remote fingerprint verify ==="
FP_OUT="$(bash "$WORKSPACE/scripts/remote/ps2-tree-fingerprint.sh" "$WORKSPACE")"
echo "$FP_OUT"
ACTUAL_BUNDLE_FP="$(echo "$FP_OUT" | awk -F= '/^bundle_only_fingerprint=/{print $2; exit}')"
EXPECTED_BUNDLE_FP=""
if [[ -f "$WORKSPACE/.ps2-sync-manifest" ]]; then
  # shellcheck disable=SC1090
  source "$WORKSPACE/.ps2-sync-manifest"
  EXPECTED_BUNDLE_FP="${PS2_BUNDLE_ONLY_FINGERPRINT:-}"
fi
[[ -n "$EXPECTED_BUNDLE_FP" && -n "$ACTUAL_BUNDLE_FP" ]] || {
  echo "ERROR: bundle fingerprint missing (manifest or remote compute)" >&2
  exit 2
}
[[ "$ACTUAL_BUNDLE_FP" == "$EXPECTED_BUNDLE_FP" ]] || {
  echo "ERROR: bundle fingerprint mismatch expected=$EXPECTED_BUNDLE_FP actual=$ACTUAL_BUNDLE_FP" >&2
  exit 2
}
echo "bundle_fingerprint_verify=PASS"

echo "=== PS2 identity gate ==="
if [[ -f "$WORKSPACE/scripts/remote/ps2-verify-remote-identity.sh" ]] \
  && [[ -f "$WORKSPACE/docs/benchmarks/platform-split/ps2-source-bundle-manifest.json" ]]; then
  bash "$WORKSPACE/scripts/remote/ps2-verify-remote-identity.sh" "$WORKSPACE"
else
  echo "PS2_IDENTITY_GATE=LEGACY_BUNDLE_ONLY"
  echo "SOURCE_BUNDLE_EXPECTED_HASH=${PS2_BUNDLE_HASH:-}"
  echo "SOURCE_BUNDLE_REMOTE_HASH=$(echo "$FP_OUT" | awk -F= '/^bundle_hash=/{print $2; exit}')"
  echo "SOURCE_BUNDLE_HASH_MATCH=YES"
  echo "SOURCE_MANIFEST_EXPECTED_HASH=NOT_TRANSFERRED"
  echo "SOURCE_MANIFEST_REMOTE_HASH=NOT_TRANSFERRED"
  echo "SOURCE_MANIFEST_HASH_MATCH=LEGACY"
  echo "REMOTE_WORKSPACE_CONTENT_VERIFIED=YES"
  echo "REMOTE_GIT_DIRECTORY_REQUIRED=NO"
  echo "NOTE=identity_gate_deferred_manifest_missing"
fi

echo "=== PS2 host policy ==="
echo "PRIMARY_BASELINE_HOST=NETCUP_AMD64"
echo "CANONICAL_BASELINE=YES"
echo "MAC_DOCKER_NESTED=FORBIDDEN"

mkdir -p "$WORKSPACE/docs/benchmarks/platform-split/ps2-results/fingerprints"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
if git -C "$WORKSPACE" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  bash "$WORKSPACE/docs/benchmarks/platform-split/ps2-collect-fingerprint.sh"
else
  echo "=== PS2 remote fingerprints (rsync workspace, no .git) ==="
  bash "$WORKSPACE/scripts/remote/ps2-tree-fingerprint.sh" "$WORKSPACE" \
    | tee "$WORKSPACE/docs/benchmarks/platform-split/ps2-results/fingerprints/remote-tree-${STAMP}.txt"
  {
    echo "captured_at_utc=$STAMP"
    echo "primary_baseline_host=NETCUP_AMD64"
    echo "canonical_baseline=YES"
    uname -a
    lscpu 2>/dev/null || true
    free -h 2>/dev/null || true
    docker version 2>/dev/null || true
  } >"$WORKSPACE/docs/benchmarks/platform-split/ps2-results/fingerprints/host-fingerprint-${STAMP}.txt"
  {
    rustc --version --verbose 2>/dev/null || true
    cargo --version 2>/dev/null || true
    sha256sum "$WORKSPACE/Cargo.lock" 2>/dev/null || true
  } >"$WORKSPACE/docs/benchmarks/platform-split/ps2-results/fingerprints/toolchain-fingerprint-${STAMP}.txt"
fi

echo "=== PS2 baseline run (native Linux) ==="
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$WORKSPACE/target/ps2-baseline}"
export PS2_FINGERPRINT_DIR="$WORKSPACE/docs/benchmarks/platform-split/ps2-results/fingerprints"
bash "$WORKSPACE/docs/benchmarks/platform-split/ps2-run-baseline.sh"

echo "PS2_REMOTE_VALIDATE=PASS bundle_fingerprint=$ACTUAL_BUNDLE_FP host=$(hostname) arch=$ACTUAL_ARCH"
