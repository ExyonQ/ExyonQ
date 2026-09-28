#!/usr/bin/env bash
# PS2-IU1 Mac orchestrator → Netcup minimal io_uring probe.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
SSH_OPTS="${PS2_SSH_OPTS:--o BatchMode=yes -o ConnectTimeout=30}"
NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
NETCUP_WORKSPACE="${NETCUP_WORKSPACE:-/root/exyonq-dev-soak-src}"
RUN_ID="${PS2_IU1_RUN_ID:-iu1-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCAL_BASE="$ROOT/docs/benchmarks/platform-split/ps2-results/netcup-amd64-iu1"
LOCAL_DEST="$LOCAL_BASE/$RUN_ID"
PROBE_LOCAL="$ROOT/scripts/remote/ps2-iu1-io-uring-probe.sh"
chmod +x "$PROBE_LOCAL"
mkdir -p "$LOCAL_DEST"
scp $SSH_OPTS "$PROBE_LOCAL" "${NETCUP_HOST}:${NETCUP_WORKSPACE}/scripts/remote/ps2-iu1-io-uring-probe.sh"
ssh $SSH_OPTS "$NETCUP_HOST" "source ~/.cargo/env 2>/dev/null; PS2_IU1_RUN_ID='$RUN_ID' PS2_IU1_REPS='${PS2_IU1_REPS:-3}' bash '$NETCUP_WORKSPACE/scripts/remote/ps2-iu1-io-uring-probe.sh'" \
  | tee "$LOCAL_DEST/orchestrator.log"
rsync -az -e "ssh $SSH_OPTS" \
  "${NETCUP_HOST}:${NETCUP_WORKSPACE}/docs/benchmarks/platform-split/ps2-results/netcup-amd64-iu1/$RUN_ID/" \
  "$LOCAL_DEST/"
echo "IU1_PULL_COMPLETE dest=$LOCAL_DEST"
