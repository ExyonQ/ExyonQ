#!/usr/bin/env bash
# Build ExyonQ allocator distribution variants with isolated CARGO_TARGET_DIR trees.
# Official product variants: system (default) and jemalloc.
# Mimalloc is not a product option (historical AC1–AC5 evidence only).
# Usage: bash scripts/allocator/build-variants.sh [system|jemalloc|all]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

source "${HOME}/.cargo/env" 2>/dev/null || true

VARIANT_FILTER="${1:-all}"
RUSTC_VERSION="$(rustc -vV | awk '/^release:/{print $2}')"
HOST_TRIPLE="$(rustc -vV | awk '/^host:/{print $2}')"
TARGET_TRIPLE="${CARGO_BUILD_TARGET:-$HOST_TRIPLE}"
TARGET_CPU="${TARGET_CPU:-native}"
SOURCE_COMMIT="$(git rev-parse HEAD 2>/dev/null || true)"
if [[ -z "$SOURCE_COMMIT" || "$SOURCE_COMMIT" == "UNKNOWN" ]]; then
  SOURCE_COMMIT="${SOURCE_COMMIT_OVERRIDE:-bee1d68414fed48b4e4d7138eb12b8fe1a696558}"
fi
IDENTITY_DIR="${IDENTITY_DIR:-$ROOT/docs/benchmarks/allocators/identities}"
mkdir -p "$IDENTITY_DIR"

build_one() {
  local name="$1"
  local feature="$2"
  local target_dir="$ROOT/target/allocator-${name}"
  local features_args=()
  local cargo_features="(none / system)"

  if [[ -n "$feature" ]]; then
    features_args=(--features "$feature")
    cargo_features="$feature"
  fi

  echo "=== Building allocator-${name} (features=${cargo_features}) ==="
  echo "CARGO_TARGET_DIR=$target_dir"
  export CARGO_TARGET_DIR="$target_dir"
  cargo build --release --locked -p exyonq "${features_args[@]}"

  local bin="$target_dir/release/exyonq"
  local sha
  if command -v sha256sum >/dev/null 2>&1; then
    sha="$(sha256sum "$bin" | awk '{print $1}')"
  else
    sha="$(shasum -a 256 "$bin" | awk '{print $1}')"
  fi
  local size
  size="$(wc -c <"$bin" | tr -d ' ')"

  local out="$IDENTITY_DIR/identity-${name}.txt"
  {
    echo "SOURCE_COMMIT=${SOURCE_COMMIT}"
    echo "ALLOCATOR_VARIANT=${name}"
    echo "RUSTC_VERSION=${RUSTC_VERSION}"
    echo "TARGET_TRIPLE=${TARGET_TRIPLE}"
    echo "TARGET_CPU=${TARGET_CPU}"
    echo "CARGO_PROFILE=release"
    echo "CARGO_FEATURES=${cargo_features}"
    echo "BINARY_PATH=${bin}"
    echo "BINARY_SHA256=${sha}"
    echo "BINARY_SIZE_BYTES=${size}"
    echo "CARGO_TARGET_DIR=${target_dir}"
    echo "LTO=true"
    echo "CODEGEN_UNITS=1"
    echo "CONTAINER_IMAGE_ID=(host build; set after docker build)"
  } | tee "$out"
  echo "Wrote $out"
}

case "$VARIANT_FILTER" in
  system) build_one system "" ;;
  jemalloc) build_one jemalloc "allocator-jemalloc" ;;
  mimalloc)
    echo "ERROR: mimalloc is not a product allocator (MIMALLOC_PRODUCT_SUPPORT=REMOVE)" >&2
    echo "Historical identities remain under docs/benchmarks/allocators/identities/" >&2
    exit 2
    ;;
  all)
    build_one system ""
    build_one jemalloc "allocator-jemalloc"
    ;;
  *)
    echo "usage: $0 [system|jemalloc|all]" >&2
    exit 2
    ;;
esac

echo "=== Feature surface check ==="
if cargo metadata --format-version 1 --no-deps 2>/dev/null | grep -q 'allocator-mimalloc'; then
  echo "FAIL: allocator-mimalloc still present in package metadata" >&2
  exit 1
fi
echo "PASS: no allocator-mimalloc feature on exyonq package"
