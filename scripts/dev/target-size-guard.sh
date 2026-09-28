#!/usr/bin/env bash
# Keep workspace target/ under a size budget (default 6 GiB). Runs cargo clean when exceeded.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
MAX_GIB="${TARGET_MAX_GIB:-6}"
MAX_KIB=$((MAX_GIB * 1024 * 1024))

mkdir -p "$TARGET"

size_kib=$(du -sk "$TARGET" | awk '{print $1}')
size_gib=$(awk -v k="$size_kib" 'BEGIN { printf "%.2f", k / 1024 / 1024 }')

if (( size_kib > MAX_KIB )); then
  echo "target-size-guard: ${size_gib} GiB > ${MAX_GIB} GiB — cargo clean ($TARGET)"
  (cd "$ROOT" && cargo clean --target-dir "$TARGET")
  echo "target-size-guard: done ($(du -sh "$TARGET" | awk '{print $1}'))"
else
  echo "target-size-guard: OK ${size_gib} GiB / ${MAX_GIB} GiB ($TARGET)"
fi
