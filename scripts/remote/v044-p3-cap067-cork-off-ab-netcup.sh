#!/usr/bin/env bash
# Launch Netcup large-body cork-off A/B (Cap067 P3 residual).
set -euo pipefail
ROOT="${ROOT:-/Volumes/Lexar/Cursor/exyonq-laboratorio}"
HOST="${CORK_HOST:-netcup-bench}"
REMOTE_BUILD="${CORK_REMOTE_BUILD:-/root/exyonq-forensic-p1p3}"
REMOTE_SEAL="${CORK_REMOTE_SEAL:-/root/exyonq-v044-p1-seal-06742e1b}"
PROJECT="${COMPOSE_PROJECT_NAME:-v044p1auth-clean}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOCAL_EV="$ROOT/.exyonq-local/evidence/v044-p3-cap067-cork-off-ab-$TS"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=30"

mkdir -p "$LOCAL_EV"

rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=30' \
  "$ROOT/crates/exyonq-mod-static/src/sendfile_fsm.rs" \
  "${HOST}:${REMOTE_BUILD}/crates/exyonq-mod-static/src/"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=30' \
  "$ROOT/crates/exyonq-mod-static/src/sendfile_fsm.rs" \
  "${HOST}:${REMOTE_SEAL}/crates/exyonq-mod-static/src/"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=30' \
  "$ROOT/benchmarks/docker/docker-compose.bench.yml" \
  "${HOST}:${REMOTE_SEAL}/benchmarks/docker/docker-compose.bench.yml"
rsync -az -e 'ssh -o BatchMode=yes -o ConnectTimeout=30' \
  "$ROOT/docs/governance/v044-p3-cap067-sendfile-productivity-diag.md" \
  "${HOST}:${REMOTE_SEAL}/docs/governance/" 2>/dev/null || true

scp -o BatchMode=yes -o ConnectTimeout=30 \
  "$ROOT/scripts/remote/v044-p3-cap067-cork-off-ab-remote.sh" \
  "${HOST}:/tmp/v044-p3-cap067-cork-off-ab-remote.sh"

echo "[cork-ab] remote build + A/B (nohup; ~2–3h)"
ssh -o BatchMode=yes -o ConnectTimeout=30 "$HOST" \
  "chmod +x /tmp/v044-p3-cap067-cork-off-ab-remote.sh; \
   setsid nohup env CORK_TS='$TS' CORK_REMOTE_BUILD='$REMOTE_BUILD' CORK_REMOTE_SEAL='$REMOTE_SEAL' \
   COMPOSE_PROJECT_NAME='$PROJECT' \
   bash /tmp/v044-p3-cap067-cork-off-ab-remote.sh \
     >/tmp/v044-p3-cap067-cork-off-ab-$TS.log 2>&1 < /dev/null & echo PID=\$! LOG=/tmp/v044-p3-cap067-cork-off-ab-$TS.log"

echo "[cork-ab] launched. Local evidence folder: $LOCAL_EV"
echo "$TS" >"$LOCAL_EV/suite_ts.txt"
echo "REMOTE_LOG=/tmp/v044-p3-cap067-cork-off-ab-$TS.log" | tee "$LOCAL_EV/remote_pointer.txt"
echo "REMOTE_SUITE=$REMOTE_SEAL/.exyonq-local-evidence/v044-p1p3-cork-off-ab-$TS" | tee -a "$LOCAL_EV/remote_pointer.txt"
