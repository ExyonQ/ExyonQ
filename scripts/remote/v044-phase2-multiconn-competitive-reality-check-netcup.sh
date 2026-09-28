#!/usr/bin/env bash
# V044_PHASE2_MULTI_CONNECTION_COMPETITIVE_REALITY_CHECK
# READ_ONLY product. Harness-only. PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN.
# Netcup AMD64 multi-conn P4-like compare: Phase2 CFD vs current ExyonQ vs NGINX vs OLS vs HAProxy.
set -euo pipefail

ROOT="${ROOT:-/Volumes/Lexar/Cursor/exyonq-laboratorio}"
HOST="${PCR_HOST:-netcup-bench}"
REMOTE_WS="${PCR_REMOTE_WS:-/root/exyonq-cfd-phase2}"
RUN_ID="${PCR_RUN_ID:-20260828-173023}"
LOCAL_EV="$ROOT/.exyonq-local/evidence/phase2-multiconn-competitive-reality/${RUN_ID}"
STACK="${PCR_STACK:-v044pxdp-reality-20260826-223445}"
NET="${STACK}_default"
CONC="${PCR_CONC:-100}"
THREADS="${PCR_THREADS:-2}"
WARMUP="${PCR_WARMUP:-20}"
MEASURE="${PCR_MEASURE:-30}"
REPS="${PCR_REPS:-7}"
CPUSET="${PCR_CPUSET:-0-7}"
PATH_P4="/api/"

mkdir -p "$LOCAL_EV"/{configs,correctness-precheck,raw-results,cpu,memory,latency,scheduler,errors,geometry}

echo "[pcr] sync PHASE1+2 crates only -> ${HOST}:${REMOTE_WS}"
ssh -o BatchMode=yes -o ConnectTimeout=20 "$HOST" "mkdir -p '$REMOTE_WS'"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=20' \
  --exclude target --exclude .git --exclude .exyonq-local \
  "$ROOT/crates/exyonq-cfd-gen/" "${HOST}:${REMOTE_WS}/crates/exyonq-cfd-gen/"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=20' \
  --exclude target --exclude .git \
  "$ROOT/crates/exyonq-cfd-dataplane/" "${HOST}:${REMOTE_WS}/crates/exyonq-cfd-dataplane/"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=20' \
  --exclude target --exclude .git \
  "$ROOT/crates/exyonq-cfd-control/" "${HOST}:${REMOTE_WS}/crates/exyonq-cfd-control/"
# workspace Cargo.toml/lock needed to build
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=20' \
  "$ROOT/Cargo.toml" "$ROOT/Cargo.lock" "${HOST}:${REMOTE_WS}/"

scp -o BatchMode=yes -o ConnectTimeout=20 \
  "$ROOT/.exyonq-local/tmp/pcr_remote.sh" \
  "${HOST}:/tmp/pcr_remote.sh"

ssh -o BatchMode=yes -o ConnectTimeout=20 "$HOST" \
  "chmod +x /tmp/pcr_remote.sh && PCR_RUN_ID='$RUN_ID' PCR_STACK='$STACK' PCR_NET='$NET' PCR_CONC='$CONC' PCR_THREADS='$THREADS' PCR_WARMUP='$WARMUP' PCR_MEASURE='$MEASURE' PCR_REPS='$REPS' PCR_CPUSET='$CPUSET' PCR_REMOTE_WS='$REMOTE_WS' bash /tmp/pcr_remote.sh" \
  2>&1 | tee "$LOCAL_EV/orchestrator.log"

rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=20' \
  "${HOST}:${REMOTE_WS}/.exyonq-local/evidence/phase2-multiconn-competitive-reality/${RUN_ID}/" \
  "$LOCAL_EV/"
echo "[pcr] done -> $LOCAL_EV"
