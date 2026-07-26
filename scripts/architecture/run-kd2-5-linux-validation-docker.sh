#!/usr/bin/env bash
# Wrapper: KD2.5 Linux amd64 via Docker when host is not Linux x86_64.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
LOG="${KD25_LINUX_LOG:-$ROOT/target/kd2-5-linux-validation.log}"
mkdir -p "$(dirname "$LOG")"

if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
  exec bash scripts/architecture/run-kd2-5-linux-validation.sh
fi

echo "Host $(uname -s)/$(uname -m) — Docker linux/amd64" | tee "$LOG"
docker run --platform linux/amd64 --rm \
  -v "$ROOT:/work" \
  -w /work \
  -e CARGO_HOME=/work/target/docker-cargo-home \
  -e CARGO_TARGET_DIR=/work/target/linux-amd64 \
  rust:1.93-bookworm \
  bash -lc '
    set -euo pipefail
    apt-get update -qq
    apt-get install -y -qq ripgrep curl ca-certificates python3 2>/dev/null
    chmod +x scripts/smoke/kd2-5-static-smoke.sh scripts/architecture/*.sh
    bash scripts/architecture/run-kd2-5-linux-validation.sh
  ' 2>&1 | tee -a "$LOG"
