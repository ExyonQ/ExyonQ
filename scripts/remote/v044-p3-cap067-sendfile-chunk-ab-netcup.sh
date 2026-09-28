#!/usr/bin/env bash
# V044_P3_CAP067_LARGE_BODY_SENDFILE_COUNT_CLAMP — Netcup live-3way A/B KEEP gate.
# PUBLIC_BENCHMARK_CLAIMS=FORBIDDEN. Development evidence only.
#
# Arms (same binary):
#   A uncapped-for-P3: EXYONQ_SENDFILE_CHUNK=1048576
#   B product 256 KiB: unset EXYONQ_SENDFILE_CHUNK
#   C 128 KiB:         EXYONQ_SENDFILE_CHUNK=131072
#
# Geometry: Cap067-native cork geom8 (accept/worker/epoll=8).
set -euo pipefail

ROOT="${ROOT:-/Volumes/Lexar/Cursor/exyonq-laboratorio}"
HOST="${CLAMP_HOST:-netcup-bench}"
REMOTE_BUILD="${CLAMP_REMOTE_BUILD:-/root/exyonq-forensic-p1p3}"
REMOTE_SEAL="${CLAMP_REMOTE_SEAL:-/root/exyonq-v044-p1-seal-06742e1b}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOCAL_EV="$ROOT/.exyonq-local/evidence/v044-p3-cap067-sendfile-chunk-ab-$TS"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=30"

mkdir -p "$LOCAL_EV"

echo "[clamp-ab] sync sendfile sources + compose -> $HOST"
rsync -az -e "ssh $SSH_OPTS" \
  "$ROOT/crates/exyonq-mod-static/src/sendfile.rs" \
  "$ROOT/crates/exyonq-mod-static/src/sendfile_fsm.rs" \
  "${HOST}:${REMOTE_BUILD}/crates/exyonq-mod-static/src/"
rsync -az -e "ssh $SSH_OPTS" \
  "$ROOT/crates/exyonq-mod-static/src/sendfile.rs" \
  "$ROOT/crates/exyonq-mod-static/src/sendfile_fsm.rs" \
  "${HOST}:${REMOTE_SEAL}/crates/exyonq-mod-static/src/"
rsync -az -e "ssh $SSH_OPTS" \
  "$ROOT/benchmarks/docker/docker-compose.bench.yml" \
  "${HOST}:${REMOTE_SEAL}/benchmarks/docker/docker-compose.bench.yml"
rsync -az -e "ssh $SSH_OPTS" \
  "$ROOT/docs/governance/v044-p3-cap067-large-body-sendfile-count-clamp.md" \
  "${HOST}:${REMOTE_SEAL}/docs/governance/v044-p3-cap067-large-body-sendfile-count-clamp.md"

scp -o BatchMode=yes -o ConnectTimeout=30 \
  "$ROOT/scripts/remote/v044-p3-cap067-sendfile-chunk-ab-remote.sh" \
  "${HOST}:/tmp/v044-p3-cap067-sendfile-chunk-ab-remote.sh"

echo "[clamp-ab] remote build + A/B suites (nohup; ~3h)"
ssh $SSH_OPTS "$HOST" \
  "chmod +x /tmp/v044-p3-cap067-sendfile-chunk-ab-remote.sh && \
   CLAMP_TS='$TS' CLAMP_REMOTE_BUILD='$REMOTE_BUILD' CLAMP_REMOTE_SEAL='$REMOTE_SEAL' \
   COMPOSE_PROJECT_NAME='$PROJECT' \
   nohup bash /tmp/v044-p3-cap067-sendfile-chunk-ab-remote.sh \
     >/tmp/v044-p3-cap067-sendfile-chunk-ab-$TS.log 2>&1 & echo PID=\$! LOG=/tmp/v044-p3-cap067-sendfile-chunk-ab-$TS.log"

echo "[clamp-ab] launched. Local evidence folder: $LOCAL_EV"
echo "$TS" >"$LOCAL_EV/suite_ts.txt"
echo "REMOTE_LOG=/tmp/v044-p3-cap067-sendfile-chunk-ab-$TS.log" | tee "$LOCAL_EV/remote_pointer.txt"
echo "REMOTE_SUITE=$REMOTE_SEAL/.exyonq-local-evidence/v044-p1p3-sendfile-chunk-ab-$TS" | tee -a "$LOCAL_EV/remote_pointer.txt"
