#!/usr/bin/env bash
# KD2.5 final Linux amd64 validation (run natively on Linux x86_64).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
LOG="${KD25_LINUX_LOG:-$ROOT/target/kd2-5-linux-validation.log}"
mkdir -p "$(dirname "$LOG")"

exec > >(tee -a "$LOG") 2>&1

echo "=== KD2.5 Linux validation $(date -Iseconds) ==="
uname -a
rustc --version
cargo --version
git rev-parse HEAD
echo "dirty_entries: $(git status --short | wc -l | tr -d ' ')"

echo "--- cargo fmt --check ---"
cargo fmt --check

echo "--- cargo check ---"
cargo check -p exyonq-module-api
cargo check -p exyonq-mod-static
cargo check -p exyonq-core
cargo check -p exyonq

echo "--- cargo test exyonq-module-api ---"
cargo test -p exyonq-module-api -- --test-threads=1

echo "--- cargo test exyonq-mod-static ---"
cargo test -p exyonq-mod-static -- --test-threads=1

echo "--- cargo test exyonq-core ---"
cargo test -p exyonq-core -- --test-threads=1

echo "--- architecture guards ---"
bash scripts/architecture/verify-kd2-static-guards.sh closure
bash scripts/architecture/verify-kd2-ci.sh
bash scripts/architecture/verify-kd1-ci.sh
bash scripts/architecture/verify-kd0-fcgi-guards.sh

echo "--- static smoke ---"
bash scripts/smoke/kd2-5-static-smoke.sh

echo "=== KD2.5 Linux validation OK ==="
