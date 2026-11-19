#!/usr/bin/env bash
# Run all ExyonQ fuzz targets (requires cargo-fuzz on Linux).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FUZZ_DIR="$ROOT/fuzz"
CI="${1:-}"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "cargo-fuzz not installed; skipping fuzz" >&2
  exit 0
fi

cd "$FUZZ_DIR"
TARGETS=(header_end static_request_line config_parse path_resolve fcgi_record)
FUZZ_ARGS=(-max_total_time=300)
if [[ "$CI" == "--ci" ]]; then
  FUZZ_ARGS=(-runs=10000)
fi

# rust-toolchain.toml pins the stable channel, which overrides the nightly
# default that CI installs; sanitizer coverage (-Zsanitizer) only exists on
# nightly, so fuzz targets are always built with the nightly toolchain.
for target in "${TARGETS[@]}"; do
  echo "Fuzzing $target ..."
  cargo +nightly fuzz run "$target" -- "${FUZZ_ARGS[@]}"
done

echo "Fuzz complete."
