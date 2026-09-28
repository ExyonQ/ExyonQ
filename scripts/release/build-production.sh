#!/usr/bin/env bash
# Production Linux binaries: musl, statically linked, self-contained.
# glibc crt-static is not used: a static glibc still dlopens NSS and breaks DNS.
# macOS and Windows stay dynamically linked to the platform libc.
#
# Usage:
#   bash scripts/release/build-production.sh
#   bash scripts/release/build-production.sh -p exyonq --features http3-provider-quiche --no-default-features
#   EXYONQ_TARGET=x86_64-unknown-linux-musl bash scripts/release/build-production.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "ERROR: production static build runs on Linux (got $(uname -s))" >&2
  exit 1
fi

target="${EXYONQ_TARGET:-}"
if [[ -z "$target" ]]; then
  case "$(uname -m)" in
    x86_64) target="x86_64-unknown-linux-musl" ;;
    aarch64|arm64) target="aarch64-unknown-linux-musl" ;;
    *)
      echo "ERROR: unsupported production arch $(uname -m)" >&2
      exit 1
      ;;
  esac
fi

if ! command -v musl-gcc >/dev/null 2>&1; then
  echo "ERROR: musl-gcc missing. Install musl-tools (apt install musl-tools)." >&2
  exit 1
fi

cc_var="CC_${target//-/_}"
cxx_var="CXX_${target//-/_}"
export "${cc_var}=musl-gcc"
if command -v musl-g++ >/dev/null 2>&1; then
  export "${cxx_var}=musl-g++"
fi

rustup target add "$target" >/dev/null

args=("$@")
if [[ ${#args[@]} -eq 0 ]]; then
  args=(-p exyonq -p exyonqctl -p exyonq-compat-cli)
fi

echo "PRODUCTION_TARGET=$target"
echo "PRODUCTION_LINK=static-musl"
out_root="${CARGO_TARGET_DIR:-$ROOT/target}"
cargo build --release --locked --target "$target" "${args[@]}"

verify_static() {
  local bin="$1"
  local out
  out="$(ldd "$bin" 2>&1 || true)"
  if grep -Eq 'not a dynamic executable|statically linked' <<<"$out"; then
    echo "STATIC_OK $bin"
    return 0
  fi
  echo "ERROR: $bin is not a static production binary" >&2
  printf '%s\n' "$out" >&2
  file "$bin" >&2 || true
  exit 1
}

mkdir -p "$out_root/release"
shopt -s nullglob
copied=0
for bin in "$out_root/${target}/release/"*; do
  [[ -f "$bin" && -x "$bin" ]] || continue
  base="$(basename "$bin")"
  case "$base" in
    *.d|*.rlib|build) continue ;;
  esac
  # Cargo leaves only the executables next to deps/; skip the deps directory.
  if [[ -d "$bin" ]]; then
    continue
  fi
  verify_static "$bin"
  cp -f "$bin" "$out_root/release/$base"
  copied=$((copied + 1))
done
shopt -u nullglob

if [[ "$copied" -eq 0 ]]; then
  echo "ERROR: no production binaries in $out_root/${target}/release" >&2
  exit 1
fi

echo "PRODUCTION_BINARIES=$copied"
echo "PRODUCTION_OUT=$out_root/release"
