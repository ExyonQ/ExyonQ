#!/usr/bin/env bash
# KD2 native Linux validation — run ON remote host after rsync.
# Usage:
#   bash validate-kd2-native.sh \
#     --workspace /path/to/exyonq \
#     --expected-arch x86_64|aarch64 \
#     --expected-fingerprint <sha256>
set -euo pipefail

WORKSPACE=""
EXPECTED_ARCH=""
EXPECTED_FINGERPRINT=""
RUN_SMOKE=1

usage() {
  cat <<'EOF'
Usage: validate-kd2-native.sh --workspace PATH --expected-arch ARCH --expected-fingerprint SHA256 [--no-smoke]

  --workspace PATH              Remote checkout root (must exist)
  --expected-arch ARCH          x86_64 (Netcup) or aarch64 (Oracle)
  --expected-fingerprint SHA  Non-empty bundle tree fingerprint from Mac sync
  --no-smoke                    Skip functional smoke (check/test/guards only)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace)
      WORKSPACE="${2:-}"
      shift 2
      ;;
    --expected-arch)
      EXPECTED_ARCH="${2:-}"
      shift 2
      ;;
    --expected-fingerprint)
      EXPECTED_FINGERPRINT="${2:-}"
      shift 2
      ;;
    --no-smoke)
      RUN_SMOKE=0
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

[[ -n "${EXPECTED_FINGERPRINT:-}" ]] || {
  echo "ERROR: expected fingerprint is empty" >&2
  exit 2
}

[[ -n "${WORKSPACE:-}" ]] || {
  echo "ERROR: --workspace is required" >&2
  exit 2
}

[[ -d "$WORKSPACE" ]] || {
  echo "ERROR: workspace does not exist: $WORKSPACE" >&2
  exit 2
}

[[ -n "${EXPECTED_ARCH:-}" ]] || {
  echo "ERROR: --expected-arch is required" >&2
  exit 2
}

actual_arch="$(uname -m)"
case "$EXPECTED_ARCH" in
  x86_64 | amd64)
    [[ "$actual_arch" == "x86_64" ]] || {
      echo "ERROR: arch mismatch: expected x86_64 got $actual_arch" >&2
      exit 2
    }
    ;;
  aarch64 | arm64)
    [[ "$actual_arch" == "aarch64" ]] || {
      echo "ERROR: arch mismatch: expected aarch64 got $actual_arch" >&2
      exit 2
    }
    ;;
  *)
    echo "ERROR: unsupported --expected-arch: $EXPECTED_ARCH" >&2
    exit 2
    ;;
esac

require_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "ERROR: required command missing: $cmd" >&2
    exit 2
  }
}

require_cmd bash
require_cmd cargo
require_cmd rustc
require_cmd rg
require_cmd python3
require_cmd curl

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH:-/usr/bin:/bin}"
export RUST_BACKTRACE=1

FP_SCRIPT="scripts/remote/kd2-tree-fingerprint.sh"
[[ -x "$FP_SCRIPT" ]] || chmod +x "$FP_SCRIPT" 2>/dev/null || true
[[ -f "$FP_SCRIPT" ]] || {
  echo "ERROR: fingerprint script missing: $FP_SCRIPT" >&2
  exit 2
}

echo "=== KD2 native validation $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
echo "--- preflight manifest ---"
manifest="$(bash "$FP_SCRIPT" "$WORKSPACE")"
printf '%s\n' "$manifest"
remote_fp="$(printf '%s\n' "$manifest" | awk -F= '/^fingerprint=/{print $2}')"

[[ -n "$remote_fp" ]] || {
  echo "ERROR: remote fingerprint computation returned empty" >&2
  exit 2
}

if [[ "$remote_fp" != "$EXPECTED_FINGERPRINT" ]]; then
  echo "ERROR: fingerprint mismatch" >&2
  echo "  expected=$EXPECTED_FINGERPRINT" >&2
  echo "  actual=$remote_fp" >&2
  exit 2
fi
echo "fingerprint: OK"

echo "--- cargo fmt --check ---"
cargo fmt --check

echo "--- cargo check ---"
cargo check -p exyonq-module-api
cargo check -p exyonq-mod-static
cargo check -p exyonq-core
cargo check -p exyonq

echo "--- cargo test (full, --test-threads=1) ---"
cargo test -p exyonq-module-api -- --test-threads=1
cargo test -p exyonq-mod-static --features test-utils -- --test-threads=1
cargo test -p exyonq-core -- --test-threads=1

echo "--- architecture guards ---"
bash scripts/architecture/verify-kd2-static-guards.sh closure
bash scripts/architecture/verify-kd2-ci.sh
bash scripts/architecture/verify-kd1-ci.sh
bash scripts/architecture/verify-kd0-fcgi-guards.sh

if [[ "$RUN_SMOKE" -eq 1 ]]; then
  echo "--- build exyonq + static smoke ---"
  cargo build -p exyonq --bin exyonq
  chmod +x scripts/smoke/kd2-5-static-smoke.sh
  bash scripts/smoke/kd2-5-static-smoke.sh
fi

echo "=== KD2 native validation OK arch=$actual_arch fingerprint=$remote_fp ==="
