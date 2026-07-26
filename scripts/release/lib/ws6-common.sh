#!/usr/bin/env bash
# P1.5-WS6 — shared helpers for release artifact harness.
# BIT_FOR_BIT NOT_CLAIMED — see docs/release/p1.5-packaging-product-contract.md
set -euo pipefail

WS6_BIT_FOR_BIT_CLAIM="${WS6_BIT_FOR_BIT_CLAIM:-NOT_CLAIMED}"

ws6_script_dir() {
  cd "$(dirname "${BASH_SOURCE[0]}")" && pwd
}

ws6_repo_root() {
  local root="${WS6_REPO_ROOT:-}"
  if [[ -n "$root" && -d "$root" ]]; then
    printf '%s\n' "$root"
    return 0
  fi
  cd "$(ws6_script_dir)/../../.." && pwd
}

ws6_require_linux() {
  [[ "$(uname -s)" == "Linux" ]] || {
    echo "ERROR: Linux host required (got $(uname -s))" >&2
    return 1
  }
}

ws6_log() {
  echo "[ws6] $*" >&2
}

ws6_read_version() {
  local workspace="${1:-$(ws6_repo_root)}"
  if [[ -n "${VERSION:-}" ]]; then
    printf '%s\n' "$VERSION"
    return 0
  fi
  if [[ -n "${P15_WS6_VERSION:-}" ]]; then
    printf '%s\n' "$P15_WS6_VERSION"
    return 0
  fi
  grep -E '^[[:space:]]*version[[:space:]]*=' "$workspace/Cargo.toml" \
    | head -n1 \
    | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/'
}

ws6_version_label() {
  local base
  base="$(ws6_read_version "${1:-}")"
  if [[ -n "${VERSION_LABEL:-}" ]]; then
    printf '%s-%s\n' "$base" "$VERSION_LABEL"
    return 0
  fi
  if [[ -n "${P15_WS6_VERSION_LABEL:-}" ]]; then
    printf '%s-%s\n' "$base" "$P15_WS6_VERSION_LABEL"
    return 0
  fi
  printf '%s\n' "$base"
}

ws6_target_to_arch() {
  case "$1" in
    x86_64-unknown-linux-gnu) printf 'amd64\n' ;;
    aarch64-unknown-linux-gnu) printf 'arm64\n' ;;
    *) echo "ERROR: unsupported target triple: $1" >&2; return 1 ;;
  esac
}

ws6_arch_to_target() {
  case "$1" in
    amd64|x86_64) printf 'x86_64-unknown-linux-gnu\n' ;;
    arm64|aarch64) printf 'aarch64-unknown-linux-gnu\n' ;;
    *) echo "ERROR: unsupported arch label: $1" >&2; return 1 ;;
  esac
}

ws6_host_arch_label() {
  case "$(uname -m)" in
    x86_64) printf 'amd64\n' ;;
    aarch64) printf 'arm64\n' ;;
    *) echo "ERROR: unsupported host arch: $(uname -m)" >&2; return 1 ;;
  esac
}

ws6_host_uname_arch() {
  uname -m
}

ws6_rust_toolchain() {
  local root="${1:-$(ws6_repo_root)}"
  if [[ -f "$root/rust-toolchain.toml" ]]; then
    sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
      "$root/rust-toolchain.toml" | head -n1
    return 0
  fi
  rustc --version | awk '{print $2}'
}

ws6_git_head() {
  local workspace="${1:-$(ws6_repo_root)}"
  if [[ -n "${EXYONQ_SOURCE_REVISION:-}" ]]; then
    printf '%s\n' "$EXYONQ_SOURCE_REVISION"
    return 0
  fi
  if [[ -f "$workspace/.ws6-source-head" ]]; then
    tr -d '[:space:]' <"$workspace/.ws6-source-head"
    return 0
  fi
  if git -C "$workspace" rev-parse HEAD >/dev/null 2>&1; then
    git -C "$workspace" rev-parse HEAD
    return 0
  fi
  echo "ERROR: no SOURCE_HEAD (set EXYONQ_SOURCE_REVISION or .ws6-source-head; .git missing)" >&2
  return 1
}

ws6_git_head_short() {
  local workspace="${1:-$(ws6_repo_root)}"
  local head
  head="$(ws6_git_head "$workspace")"
  printf '%.12s\n' "$head"
}

ws6_git_dirty() {
  local workspace="${1:-$(ws6_repo_root)}"
  if [[ -n "${EXYONQ_SOURCE_TREE_STATUS:-}" ]]; then
    case "$EXYONQ_SOURCE_TREE_STATUS" in
      CLEAN|clean|false|0) printf 'false\n' ;;
      *) printf 'true\n' ;;
    esac
    return 0
  fi
  if [[ -f "$workspace/.ws6-source-tree-status" ]]; then
    local st
    st="$(tr -d '[:space:]' <"$workspace/.ws6-source-tree-status")"
    case "$st" in
      CLEAN|clean|false|0) printf 'false\n' ;;
      *) printf 'true\n' ;;
    esac
    return 0
  fi
  if git -C "$workspace" rev-parse HEAD >/dev/null 2>&1; then
    if git -C "$workspace" diff --quiet && git -C "$workspace" diff --cached --quiet; then
      printf 'false\n'
    else
      printf 'true\n'
    fi
    return 0
  fi
  # Synced trees without .git: assume dirty unless orchestrator stamped CLEAN.
  printf 'true\n'
}

ws6_sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

ws6_sha256_verify_file() {
  local file="$1" expected="$2"
  local actual
  actual="$(ws6_sha256_file "$file")"
  [[ "$actual" == "$expected" ]]
}

ws6_tarball_name() {
  local version="$1" arch="$2"
  printf 'exyonq-%s-linux-%s.tar.gz\n' "$version" "$arch"
}

ws6_cargo_build_features() {
  if [[ "${EXYONQ_WS6_JEMALLOC:-0}" == "1" ]]; then
    printf '%s\n' "--features" "allocator-jemalloc"
  fi
}

ws6_emit_json_field() {
  python3 - "$@" <<'PY'
import json, sys
obj = json.loads(sys.argv[1])
print(json.dumps(obj, indent=2, sort_keys=True))
PY
}

ws6_write_summary_pair() {
  local out_dir="$1" human="$2" json="$3"
  mkdir -p "$out_dir"
  printf '%s\n' "$human" >"$out_dir/summary.txt"
  printf '%s\n' "$json" >"$out_dir/summary.json"
}

ws6_lockfile_hash() {
  local workspace="${1:-$(ws6_repo_root)}"
  ws6_sha256_file "$workspace/Cargo.lock"
}

ws6_now_utc() {
  date -u +%Y-%m-%dT%H:%M:%SZ
}

ws6_build_timestamp() {
  if [[ -n "${BUILD_TIMESTAMP:-}" ]]; then
    printf '%s\n' "$BUILD_TIMESTAMP"
    return 0
  fi
  ws6_now_utc
}

ws6_assert_arch_matches_host() {
  local tarball="$1"
  local host_label
  host_label="$(ws6_host_arch_label)"
  case "$tarball" in
    *-linux-amd64.tar.gz)
      [[ "$host_label" == "amd64" ]]
      ;;
    *-linux-arm64.tar.gz)
      [[ "$host_label" == "arm64" ]]
      ;;
    *)
      return 1
      ;;
  esac
}

ws6_extract_arch_from_tarball_name() {
  local name="$1"
  if [[ "$name" =~ -linux-(amd64|arm64)\.tar\.gz$ ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
    return 0
  fi
  return 1
}
