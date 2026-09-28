#!/usr/bin/env bash
# Orchestrate R3E dual-arch competitive ExyonQ vs OLS from product baseline f0b2d67.
# Syncs source archive + scripts; builds release binaries on evidence hosts; runs remote harness.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_WS="${NETCUP_WS:-/root/exyonq-r3e}"
ORACLE_WS="${ORACLE_WS:-/home/ubuntu/exyonq-r3e}"
SSH_OPTS="${SSH_OPTS:--o BatchMode=yes -o StrictHostKeyChecking=accept-new}"
EXYONQ_REVISION="${EXYONQ_REVISION:-f0b2d67}"
TREE_HEAD="$(git rev-parse --short HEAD)"
LOCAL_EV="${LOCAL_EV:-$ROOT/.exyonq-local/tmp/r3-20260803}"
REMOTE_EV_BASE=".exyonq-local/tmp/r3-20260803"
mkdir -p "$LOCAL_EV"

echo "R3E_ORCHESTRATE baseline=$EXYONQ_REVISION tree_head=$TREE_HEAD"
# Product tree must match baseline (docs-only HEAD drift OK)
if ! git merge-base --is-ancestor "$EXYONQ_REVISION" HEAD; then
  echo "FATAL: $EXYONQ_REVISION is not ancestor of HEAD" >&2
  exit 2
fi
if [[ -n "$(git diff --name-only "$EXYONQ_REVISION" HEAD -- core crates modules cli config module-api packaging wasm 2>/dev/null)" ]]; then
  echo "FATAL: product paths differ from $EXYONQ_REVISION" >&2
  git diff --stat "$EXYONQ_REVISION" HEAD -- core crates modules cli config module-api packaging wasm
  exit 2
fi

ARCHIVE="$LOCAL_EV/exyonq-${EXYONQ_REVISION}.tar"
if [[ ! -f "$ARCHIVE" ]]; then
  echo "creating git archive $ARCHIVE"
  git archive --format=tar "$EXYONQ_REVISION" -o "$ARCHIVE"
fi

sync_host() {
  local host="$1" ws="$2"
  echo "[$(date -u +%H:%M:%S)] sync $host -> $ws"
  ssh $SSH_OPTS "$host" "mkdir -p '$ws' '$ws/$REMOTE_EV_BASE' '$ws/scripts/r3' '$ws/tools/upstream'"
  # Unpack product baseline (idempotent enough: clear src trees that archive owns)
  ssh $SSH_OPTS "$host" "cd '$ws' && tar xf -" <"$ARCHIVE"
  # Overlay newer R3 scripts + upstream + OLS templates (infra; not product)
  rsync -az -e "ssh $SSH_OPTS" \
    --exclude target --exclude '*.log' \
    "$ROOT/scripts/r3/" "$host:$ws/scripts/r3/"
  rsync -az -e "ssh $SSH_OPTS" \
    --exclude target --exclude '*.log' \
    "$ROOT/tools/upstream/" "$host:$ws/tools/upstream/"
}

build_host() {
  local host="$1" ws="$2" arch="$3"
  echo "[$(date -u +%H:%M:%S)] build $host ($arch)"
  ssh $SSH_OPTS "$host" "bash -lc '
    set -euo pipefail
    source \"\$HOME/.cargo/env\" 2>/dev/null || true
    cd \"$ws\"
    export CARGO_TERM_COLOR=never
    # Prefer isolated target to avoid colliding with soak-src
    export CARGO_TARGET_DIR=\"$ws/target\"
    echo TREE_HEAD=$TREE_HEAD > \"$ws/$REMOTE_EV_BASE/${arch}-r3e-build.txt\"
    echo EXYONQ_REVISION=$EXYONQ_REVISION >> \"$ws/$REMOTE_EV_BASE/${arch}-r3e-build.txt\"
    cargo build --release -p exyonq 2>&1 | tee \"$ws/$REMOTE_EV_BASE/${arch}-r3e-build-exyonq.log\"
    # Upstream peer is a standalone workspace; do not inherit product CARGO_TARGET_DIR.
    unset CARGO_TARGET_DIR
    cargo build --release --manifest-path tools/upstream/Cargo.toml 2>&1 | tee \"$ws/$REMOTE_EV_BASE/${arch}-r3e-build-upstream.log\"
    test -x \"$ws/target/release/exyonq\"
    test -x tools/upstream/target/release/exyonq-upstream
    \"$ws/target/release/exyonq\" --version | tee -a \"$ws/$REMOTE_EV_BASE/${arch}-r3e-build.txt\"
  '"
}

run_host() {
  local host="$1" ws="$2" arch="$3" wrk2="$4"
  echo "[$(date -u +%H:%M:%S)] run R3E $host ($arch)"
  ssh $SSH_OPTS "$host" "bash -lc '
    set -euo pipefail
    source \"\$HOME/.cargo/env\" 2>/dev/null || true
    cd \"$ws\"
    export ARCH_LABEL=$arch
    export WRK2_BIN=$wrk2
    export ROOT=$ws
    export EV=$ws/$REMOTE_EV_BASE/${arch}-r3e
    export EXYONQ_REVISION=$EXYONQ_REVISION
    export TREE_HEAD=$TREE_HEAD
    chmod +x scripts/r3/r3e-competitive-remote.sh
    bash scripts/r3/r3e-competitive-remote.sh
  '"
}

echo "[$(date -u +%H:%M:%S)] sync netcup"
sync_host "$NETCUP_HOST" "$NETCUP_WS"
echo "[$(date -u +%H:%M:%S)] sync oracle"
sync_host "$ORACLE_HOST" "$ORACLE_WS"

# Build in parallel
build_host "$NETCUP_HOST" "$NETCUP_WS" amd64 >"$LOCAL_EV/r3e-amd64-build.log" 2>&1 &
BPID_N=$!
build_host "$ORACLE_HOST" "$ORACLE_WS" arm64 >"$LOCAL_EV/r3e-arm64-build.log" 2>&1 &
BPID_O=$!
set +e
wait "$BPID_N"; EC_N=$?
wait "$BPID_O"; EC_O=$?
set -e
echo "$EC_N" >"$LOCAL_EV/r3e-amd64-build-exit.txt"
echo "$EC_O" >"$LOCAL_EV/r3e-arm64-build-exit.txt"
echo "build_exit amd64=$EC_N arm64=$EC_O"
if [[ "$EC_N" -ne 0 || "$EC_O" -ne 0 ]]; then
  echo "BUILD_FAILED — see $LOCAL_EV/r3e-*-build.log" >&2
  exit 3
fi

# Run competitively sequentially per host to reduce noise (hosts independent → parallel OK)
run_host "$NETCUP_HOST" "$NETCUP_WS" amd64 /root/exyonq-bv04/tools/wrk2-44a94c17 \
  >"$LOCAL_EV/r3e-amd64-run.log" 2>&1 &
RPID_N=$!
run_host "$ORACLE_HOST" "$ORACLE_WS" arm64 /home/ubuntu/exyonq-bv04/tools/wrk2-44a94c17 \
  >"$LOCAL_EV/r3e-arm64-run.log" 2>&1 &
RPID_O=$!
set +e
wait "$RPID_N"; REC_N=$?
wait "$RPID_O"; REC_O=$?
set -e
echo "$REC_N" >"$LOCAL_EV/r3e-amd64-run-exit.txt"
echo "$REC_O" >"$LOCAL_EV/r3e-arm64-run-exit.txt"

rsync -az -e "ssh $SSH_OPTS" \
  "$NETCUP_HOST:$NETCUP_WS/$REMOTE_EV_BASE/amd64-r3e/" "$LOCAL_EV/amd64-r3e/" || true
rsync -az -e "ssh $SSH_OPTS" \
  "$ORACLE_HOST:$ORACLE_WS/$REMOTE_EV_BASE/arm64-r3e/" "$LOCAL_EV/arm64-r3e/" || true

echo "R3E_ORCHESTRATE_DONE amd64_exit=$REC_N arm64_exit=$REC_O"
exit $(( REC_N != 0 || REC_O != 0 ? 4 : 0 ))
