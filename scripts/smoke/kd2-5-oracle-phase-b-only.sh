#!/usr/bin/env bash
# KD2.5 — Oracle arm64 Phase B only (after Netcup gate passes).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${KD25_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=120}"
ORACLE_SSH="${ORACLE_SSH:-oracle-quasar}"
ORACLE_REPO="${ORACLE_REPO:-/home/ubuntu/exyonq-dev-soak-src}"
RESULTS_DIR="${KD25_RESULTS_DIR:-$ROOT/benchmarks/results-dev/kd2-5-native-linux-20260712T135539Z}"
LOG="$RESULTS_DIR/oracle-aarch64-retry.log"
mkdir -p "$RESULTS_DIR"

write_sync_manifest() {
  local manifest="$ROOT/.kd25-sync-manifest"
  (
    cd "$ROOT"
    echo "head=$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "dirty_lines=$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
    git status --short 2>/dev/null | LC_ALL=C sort | sha256sum
    find core/src module-api/src crates/exyonq-mod-static cli/exyonq \
      scripts/architecture/verify-kd2-static-guards.sh \
      scripts/smoke/kd2-5-static-smoke.sh \
      -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum 2>/dev/null | sha256sum
    echo "sync_time=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  ) >"$manifest"
}

echo "[$(date -u +%H:%M:%S)] rsync -> $ORACLE_SSH:$ORACLE_REPO"
write_sync_manifest
rsync -az --delete \
  --exclude target \
  --exclude .git/objects \
  --exclude .git/logs \
  --exclude benchmarks/results \
  --exclude benchmarks/results-dev \
  -e "ssh $SSH_OPTS" \
  "$ROOT/" "${ORACLE_SSH}:${ORACLE_REPO}/"

echo "[$(date -u +%H:%M:%S)] oracle-aarch64 Phase B remote validation"
ssh $SSH_OPTS "$ORACLE_SSH" "bash -s" -- "$ORACLE_REPO" "oracle-aarch64" "B" >"$LOG" 2>&1 <<'REMOTE'
set -euo pipefail
REPO="$1"
LABEL="$2"
PHASE="$3"
cd "$REPO"
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
export RUST_BACKTRACE=1

echo "=== KD2.5 $LABEL Phase $PHASE $(date -Iseconds) ==="
echo "--- preflight ---"
if [[ -f .kd25-sync-manifest ]]; then
  echo "tree_manifest:"
  cat .kd25-sync-manifest
fi
uname -a
uname -m
rustc --version
cargo --version

echo "--- cargo fmt --check ---"
cargo fmt --check

echo "--- cargo check ---"
cargo check -p exyonq-module-api
cargo check -p exyonq-mod-static
cargo check -p exyonq-core
cargo check -p exyonq

echo "--- cargo test (Phase B parity) ---"
cargo test -p exyonq-module-api -- --test-threads=1
cargo test -p exyonq-mod-static --features test-utils -- --test-threads=1
cargo test -p exyonq-core -- --test-threads=1

echo "--- architecture guards ---"
bash scripts/architecture/verify-kd2-static-guards.sh closure
bash scripts/architecture/verify-kd2-ci.sh
bash scripts/architecture/verify-kd1-ci.sh
bash scripts/architecture/verify-kd0-fcgi-guards.sh

echo "--- static smoke ---"
cargo build -p exyonq --bin exyonq
chmod +x scripts/smoke/kd2-5-static-smoke.sh
bash scripts/smoke/kd2-5-static-smoke.sh

echo "=== KD2.5 $LABEL Phase $PHASE OK ==="
REMOTE

ORACLE_RC=$?
if [[ "$ORACLE_RC" -ne 0 ]]; then
  echo "FAIL: Oracle Phase B (rc=$ORACLE_RC)"
  echo "Log: $LOG"
  tail -40 "$LOG" 2>/dev/null || true
  exit "$ORACLE_RC"
fi

echo "PASS: Oracle Phase B retry"
echo "Log: $LOG"
