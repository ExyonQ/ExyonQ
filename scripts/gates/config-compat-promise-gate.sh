#!/usr/bin/env bash
# Machine-checked half of the config compatibility promise
# (docs/compat/config-compatibility-promise-v1.md): a configuration taken from a
# previously published 0.9.x release must still validate with the current
# binary. The fixtures are the real published examples, copied verbatim from
# their release tags; they are not authored for this gate.
#
# The self-test injects an unknown key and requires the parser to reject it, so
# a green run cannot come from a validator that accepts anything.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
if [[ ! -x "$BIN" ]]; then
  cargo build -q -p exyonq --bin exyonq
fi

FIXTURES=()
while IFS= read -r rel; do
  FIXTURES+=("$rel")
done < <(find scripts/gates/fixtures/config-compat -maxdepth 1 -name '*.toml' -print | sort)

if [[ ${#FIXTURES[@]} -eq 0 ]]; then
  echo "FAIL: no config-compat fixtures in scripts/gates/fixtures/config-compat"
  exit 1
fi

selftest() {
  local dir src out ec
  dir="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-config-compat.XXXXXX")"
  src="${FIXTURES[0]}"
  {
    cat "$src"
    printf '\nexyonq_compat_probe_unknown_key = 1\n'
  } >"$dir/unknown-key.toml"
  set +e
  out="$("$BIN" validate --config "$dir/unknown-key.toml" 2>&1)"
  ec=$?
  set -e
  rm -rf "$dir"
  if [[ "$ec" -eq 0 ]]; then
    echo "FAIL: the validator accepted an unknown key, so this gate proves nothing"
    printf '%s\n' "$out"
    return 1
  fi
  echo "PASS: an unknown key is rejected (exit $ec)"
}

for rel in "${FIXTURES[@]}"; do
  if ! out="$("$BIN" validate --config "$rel" 2>&1)"; then
    echo "FAIL: a previously published config no longer validates: $rel"
    printf '%s\n' "$out"
    exit 1
  fi
  echo "PASS: $rel"
done

selftest

echo "RESULT=PASS previously published configs still validate"
