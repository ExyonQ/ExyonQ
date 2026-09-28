#!/usr/bin/env bash
# KD2.5 — Native Linux validation on Netcup (amd64) then Oracle (arm64).
# Mac: preflight only. No Docker substitute.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SSH_OPTS="${KD25_SSH_OPTS:--o BatchMode=yes -o ServerAliveInterval=30 -o ServerAliveCountMax=120}"
RUN_ID="${KD25_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
RESULTS_DIR="${KD25_RESULTS_DIR:-$ROOT/benchmarks/results-dev/kd2-5-native-linux-$RUN_ID}"
mkdir -p "$RESULTS_DIR"

NETCUP_SSH="${NETCUP_SSH:-netcup-bench}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
ORACLE_SSH="${ORACLE_SSH:-oracle-quasar}"
ORACLE_REPO="${ORACLE_REPO:-/home/ubuntu/exyonq-dev-soak-src}"

tree_fingerprint() {
  local dir="$1"
  (
    cd "$dir"
    echo "head=$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "dirty_lines=$(git status --short 2>/dev/null | wc -l | tr -d ' ')"
    git status --short 2>/dev/null | LC_ALL=C sort | sha256sum
    {
      find core/src module-api/src crates/exyonq-mod-static cli/exyonq \
        scripts/architecture/verify-kd2-static-guards.sh \
        scripts/smoke/kd2-5-static-smoke.sh \
        -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum 2>/dev/null
    } | sha256sum
  )
}

write_sync_manifest() {
  local manifest="$ROOT/.kd25-sync-manifest"
  {
    tree_fingerprint "$ROOT"
    echo "sync_time=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "sync_host=$(hostname 2>/dev/null || echo unknown)"
  } >"$manifest"
}

rsync_tree() {
  local ssh_host="$1" repo="$2"
  write_sync_manifest
  echo "[$(date -u +%H:%M:%S)] rsync -> $ssh_host:$repo"
  rsync -az --delete \
    --exclude target \
    --exclude .git/objects \
    --exclude .git/logs \
    --exclude benchmarks/results \
    --exclude benchmarks/results-dev \
    -e "ssh $SSH_OPTS" \
    "$ROOT/" "${ssh_host}:${repo}/"
  chmod +x "$ROOT/scripts/smoke/kd2-5-static-smoke.sh" 2>/dev/null || true
  chmod +x "$ROOT/scripts/architecture/"*.sh 2>/dev/null || true
}

run_remote_validation() {
  local label="$1" ssh_host="$2" repo="$3" log="$4" phase="$5"
  echo "[$(date -u +%H:%M:%S)] $label Phase $phase remote validation"
  ssh $SSH_OPTS "$ssh_host" "bash -s" -- "$repo" "$label" "$phase" >>"$log" 2>&1 <<'REMOTE'
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
else
  git rev-parse HEAD 2>/dev/null || echo "HEAD=unknown"
  git status --short 2>/dev/null | wc -l | xargs echo "dirty_lines=" || echo "dirty_lines=0"
fi
uname -a
uname -m
rustc --version
cargo --version

echo "--- tree fingerprint (remote) ---"
if [[ -f .kd25-sync-manifest ]]; then
  cat .kd25-sync-manifest | tee "/tmp/kd25-fingerprint-${LABEL}.txt"
else
  {
    echo "head=$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "dirty_lines=$(git status --short 2>/dev/null | wc -l | tr -d ' ' || echo 0)"
    git status --short 2>/dev/null | LC_ALL=C sort | sha256sum || true
    find core/src module-api/src crates/exyonq-mod-static cli/exyonq \
      scripts/architecture/verify-kd2-static-guards.sh \
      scripts/smoke/kd2-5-static-smoke.sh \
      -type f 2>/dev/null | LC_ALL=C sort | xargs sha256sum 2>/dev/null | sha256sum
  } | tee "/tmp/kd25-fingerprint-${LABEL}.txt"
fi

echo "--- cargo fmt --check ---"
cargo fmt --check

echo "--- cargo check ---"
cargo check -p exyonq-module-api
cargo check -p exyonq-mod-static
cargo check -p exyonq-core
cargo check -p exyonq

if [[ "$PHASE" == "A" ]]; then
  echo "--- cargo test (full) ---"
  cargo test -p exyonq-module-api -- --test-threads=1
  cargo test -p exyonq-mod-static --features test-utils -- --test-threads=1
  cargo test -p exyonq-core -- --test-threads=1
else
  echo "--- cargo test (Phase B parity) ---"
  cargo test -p exyonq-module-api -- --test-threads=1
  cargo test -p exyonq-mod-static --features test-utils -- --test-threads=1
  cargo test -p exyonq-core -- --test-threads=1
fi

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
}

echo "KD2.5 native Linux validation run_id=$RUN_ID"
echo "Results: $RESULTS_DIR"

tree_fingerprint "$ROOT" | tee "$RESULTS_DIR/tree-fingerprint-mac.txt"

# Phase A — Netcup amd64 (gate principal)
rsync_tree "$NETCUP_SSH" "$NETCUP_REPO"
NETCUP_LOG="$RESULTS_DIR/netcup-amd64.log"
NETCUP_RC=0
run_remote_validation "netcup-amd64" "$NETCUP_SSH" "$NETCUP_REPO" "$NETCUP_LOG" "A" || NETCUP_RC=$?

if [[ "$NETCUP_RC" -ne 0 ]]; then
  echo "FAIL: Netcup Phase A (rc=$NETCUP_RC). Oracle skipped per maintainer policy."
  echo "Log: $NETCUP_LOG"
  tail -40 "$NETCUP_LOG" 2>/dev/null || true
  exit "$NETCUP_RC"
fi

echo "PASS: Netcup Phase A"

# Phase B — Oracle arm64 (solo si Netcup verde)
rsync_tree "$ORACLE_SSH" "$ORACLE_REPO"
ORACLE_LOG="$RESULTS_DIR/oracle-aarch64.log"
ORACLE_RC=0
run_remote_validation "oracle-aarch64" "$ORACLE_SSH" "$ORACLE_REPO" "$ORACLE_LOG" "B" || ORACLE_RC=$?

if [[ "$ORACLE_RC" -ne 0 ]]; then
  echo "FAIL: Oracle Phase B (rc=$ORACLE_RC)"
  echo "Log: $ORACLE_LOG"
  tail -40 "$ORACLE_LOG" 2>/dev/null || true
  exit "$ORACLE_RC"
fi

echo "PASS: Oracle Phase B"
echo "KD2.5 native Linux validation complete: $RESULTS_DIR"
exit 0
