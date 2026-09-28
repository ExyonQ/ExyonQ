#!/usr/bin/env bash
# Launch Netcup geometry contention A/B (8 vs 4) for Cap067 P3.
set -euo pipefail
ROOT="${ROOT:-/Volumes/Lexar/Cursor/exyonq-laboratorio}"
HOST="${GEOM_HOST:-netcup-bench}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOCAL_EV="$ROOT/.exyonq-local/evidence/v044-p3-cap067-geometry-contention-ab-$TS"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=30"

mkdir -p "$LOCAL_EV"
scp $SSH_OPTS \
  "$ROOT/scripts/remote/v044-p3-cap067-geometry-contention-ab-remote.sh" \
  "${HOST}:/tmp/v044-p3-cap067-geometry-contention-ab-remote.sh"

ssh $SSH_OPTS "$HOST" \
  "chmod +x /tmp/v044-p3-cap067-geometry-contention-ab-remote.sh && \
   GEOM_TS='$TS' nohup bash /tmp/v044-p3-cap067-geometry-contention-ab-remote.sh \
     >/tmp/v044-p3-cap067-geometry-contention-ab-$TS.log 2>&1 & \
   echo PID=\$! LOG=/tmp/v044-p3-cap067-geometry-contention-ab-$TS.log"

echo "$TS" >"$LOCAL_EV/suite_ts.txt"
echo "REMOTE_LOG=/tmp/v044-p3-cap067-geometry-contention-ab-$TS.log" | tee "$LOCAL_EV/remote_pointer.txt"
echo "REMOTE_SUITE=/root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p1p3-geometry-contention-ab-$TS" | tee -a "$LOCAL_EV/remote_pointer.txt"
echo "[geom-ab] launched TS=$TS (~2h)"
