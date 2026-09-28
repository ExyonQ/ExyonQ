#!/usr/bin/env bash
# P1.5-WS2 — build all fuzz targets (requires cargo-fuzz + nightly).
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT/fuzz"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "FATAL: cargo-fuzz missing (install: cargo install cargo-fuzz --locked)" >&2
  exit 2
fi

TOOLCHAIN="${P15_WS2_TOOLCHAIN:-nightly}"
TARGETS=(header_end static_request_line config_parse path_resolve fcgi_record proxy_headers nginx_migrate htaccess_parse)

echo "=== P15-WS2 build toolchain=$TOOLCHAIN ==="
for t in "${TARGETS[@]}"; do
  echo "BUILD $t"
  RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo fuzz build "$t"
done
echo "P15_WS2_BUILD=PASS"
