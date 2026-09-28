#!/usr/bin/env bash
# Launch Netcup sendfile-path productivity diagnostic (Cap067 P3 residual).
set -euo pipefail
ROOT="${ROOT:-/Volumes/Lexar/Cursor/exyonq-laboratorio}"
HOST="${SFPROD_HOST:-netcup-bench}"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOCAL_EV="$ROOT/.exyonq-local/evidence/v044-p3-cap067-sendfile-productivity-$TS"
SSH_OPTS="-o BatchMode=yes -o ConnectTimeout=30"

mkdir -p "$LOCAL_EV"
scp $SSH_OPTS \
  "$ROOT/scripts/remote/v044-p3-cap067-sendfile-productivity-diag-remote.sh" \
  "${HOST}:/tmp/v044-p3-cap067-sendfile-productivity-diag-remote.sh"

ssh $SSH_OPTS "$HOST" \
  "chmod +x /tmp/v044-p3-cap067-sendfile-productivity-diag-remote.sh && \
   V044_P3_SFPROD_TS='$TS' nohup bash /tmp/v044-p3-cap067-sendfile-productivity-diag-remote.sh \
     >/tmp/v044-p3-cap067-sendfile-productivity-$TS.log 2>&1 & \
   echo PID=\$! LOG=/tmp/v044-p3-cap067-sendfile-productivity-$TS.log"

echo "$TS" >"$LOCAL_EV/suite_ts.txt"
{
  echo "REMOTE_LOG=/tmp/v044-p3-cap067-sendfile-productivity-$TS.log"
  echo "REMOTE_SUITE=/root/exyonq-v044-p1-seal-06742e1b/.exyonq-local-evidence/v044-p3-cap067-sendfile-productivity-$TS"
} | tee "$LOCAL_EV/remote_pointer.txt"
echo "[sfprod] launched TS=$TS (~25–40 min)"
