#!/usr/bin/env bash
# Fail when a published example config is still on an older config_version.
# `config migrate --check` writes nothing and exits 1 while the file is still
# v1, so this gate catches examples that were not migrated with the IR.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
BIN="${EXYONQCTL_BIN:-$ROOT/target/debug/exyonqctl}"
if [[ ! -x "$BIN" ]]; then
  cargo build -q -p exyonqctl --locked
fi

# Published full configs. packaging/config/distributed-cache-redis.example.toml
# is an IR fragment without config_version and is intentionally excluded.
FILES=(
  packaging/config/config.toml.example
  packaging/docker/default.toml
  packaging/release/conf/wordpress.toml
)

selftest() {
  local dir out ec
  dir="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-config-drift.XXXXXX")"
  sed 's/^config_version = 2$/config_version = 1/' \
    packaging/config/config.toml.example >"$dir/config.toml"
  set +e
  out="$("$BIN" config migrate --config "$dir/config.toml" --check 2>&1)"
  ec=$?
  set -e
  rm -rf "$dir"
  if [[ "$ec" -ne 1 || "$out" != *"MIGRATION=NEEDED"* ]]; then
    echo "FAIL: migrate --check did not report v1 drift (exit $ec)"
    printf '%s\n' "$out"
    return 1
  fi
  echo "PASS: migrate --check flags a v1 config"
}

if [[ "${1:-}" == "--selftest" ]]; then
  selftest
  echo "RESULT=PASS config drift check self-test"
  exit 0
fi

for rel in "${FILES[@]}"; do
  if ! out="$("$BIN" config migrate --config "$rel" --check 2>&1)"; then
    echo "FAIL: example config is not at the current version: $rel"
    printf '%s\n' "$out"
    exit 1
  fi
  echo "PASS: $rel"
done

echo "RESULT=PASS published example configs are current"
