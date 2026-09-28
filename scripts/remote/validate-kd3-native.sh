#!/usr/bin/env bash
# KD3.2 native Linux validation — run ON remote host after rsync.
set -euo pipefail

WORKSPACE=""
EXPECTED_ARCH=""
EXPECTED_FINGERPRINT=""
RUN_E2E=1

usage() {
  cat <<'EOF'
Usage: validate-kd3-native.sh --workspace PATH --expected-arch ARCH --expected-fingerprint SHA256 [--no-e2e]
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
    --no-e2e)
      RUN_E2E=0
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
[[ -n "${WORKSPACE:-}" && -d "$WORKSPACE" ]] || {
  echo "ERROR: workspace missing: $WORKSPACE" >&2
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

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:/root/.cargo/bin:${PATH:-/usr/bin:/bin}"
export RUST_BACKTRACE=1

for cmd in bash cargo rustc rg python3 curl; do
  command -v "$cmd" >/dev/null 2>&1 || {
    echo "ERROR: required command missing: $cmd" >&2
    exit 2
  }
done

echo "=== KD3.4 native validation $(date -u +%Y-%m-%dT%H:%M:%SZ) ==="
echo "--- preflight ---"
git rev-parse HEAD || true
git status --short | head -20 || true
uname -a
uname -m
rustc --version
cargo --version

FP_SCRIPT="scripts/remote/kd3-tree-fingerprint.sh"
[[ -f "$FP_SCRIPT" ]] || {
  echo "ERROR: fingerprint script missing: $FP_SCRIPT" >&2
  exit 2
}
chmod +x "$FP_SCRIPT" scripts/e2e/suites/proxy-suite.sh 2>/dev/null || true

echo "--- tree fingerprint ---"
manifest="$(bash "$FP_SCRIPT" "$WORKSPACE")"
printf '%s\n' "$manifest"
remote_fp="$(printf '%s\n' "$manifest" | awk -F= '/^fingerprint=/{print $2}')"
[[ -n "$remote_fp" ]] || {
  echo "ERROR: remote fingerprint empty" >&2
  exit 2
}
if [[ "$remote_fp" != "$EXPECTED_FINGERPRINT" ]]; then
  echo "ERROR: fingerprint mismatch expected=$EXPECTED_FINGERPRINT actual=$remote_fp" >&2
  exit 2
fi
echo "fingerprint: OK"

echo "--- cargo fmt --check ---"
cargo fmt --check

echo "--- cargo check ---"
cargo check -p exyonq-module-api
cargo check -p exyonq-mod-tls
cargo check -p exyonq-mod-http3
cargo check -p exyonq-module-pipeline
cargo check -p exyonq-cache
cargo check -p exyonq-mod-proxy
cargo check -p exyonq-core
cargo check -p exyonq

echo "--- cargo test ---"
cargo test -p exyonq-module-api
cargo test -p exyonq-mod-tls
cargo test -p exyonq-mod-http3
cargo test -p exyonq-module-pipeline
cargo test -p exyonq-cache
cargo test -p exyonq-mod-proxy
# Linux epoll/sendfile unit tests abort under parallel lib test scheduling (IO Safety).
cargo test -p exyonq-core -- --test-threads=1

echo "--- architecture guards ---"
bash scripts/architecture/verify-kd3-proxy-guards.sh baseline
bash scripts/architecture/verify-kd3-proxy-guards.sh kd3_3
bash scripts/architecture/verify-kd3-proxy-guards.sh kd3_4
bash scripts/architecture/verify-kd2-static-guards.sh closure
bash scripts/architecture/verify-kd2-ci.sh
bash scripts/architecture/verify-kd1-ci.sh
bash scripts/architecture/verify-kd0-fcgi-guards.sh
bash scripts/architecture/verify-kd4-core-residual-guards.sh baseline
bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_1
bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_2
bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_3
bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_4
bash scripts/architecture/verify-kd4-core-residual-guards.sh kd4_5

if [[ "$RUN_E2E" -eq 1 ]]; then
  echo "--- KD3.4 proxy E2E (wire + WebSocket) ---"
  cargo build -p exyonq --bin exyonq
  bash scripts/e2e/suites/proxy-suite.sh
fi

echo "=== KD3.4 native validation OK arch=$actual_arch fingerprint=$remote_fp ==="
