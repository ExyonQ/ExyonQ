#!/usr/bin/env bash
set -euo pipefail
WS="${1:-/root/exyonq-v044-p1-auth}"
EV="$WS/.exyonq-local-evidence/v044-p1-authoritative"
mkdir -p "$EV"
{
  echo "=== disk cleanup (benchmark temp artifacts only) ==="
  df -h /
  rm -rf /tmp/rust1971-* /tmp/exyonq-sub-let-* /tmp/exyonq-protocol-write-* 2>/dev/null || true
  docker system prune -f 2>/dev/null || true
  docker builder prune -f 2>/dev/null || true
  df -h /
} | tee "$EV/disk_cleanup.log"
echo "DISK_CLEANUP=DONE"
