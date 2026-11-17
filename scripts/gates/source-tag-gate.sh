#!/usr/bin/env bash
# EXY-14: a release binary must say which tag and revision it came from.
#
# The 0.9.0 binaries were built in the lab with a private SHA, so `--version`
# reported an empty `source_tag` and nothing noticed. This gate closes that
# hole: when HEAD is a tag (or a tag is supplied), the binary must report that
# exact tag and the exact commit. On an untagged working tree it is
# NOT_APPLICABLE instead of a false pass — `--selftest` proves the check is not
# vacuous by feeding it the failure modes.
#
# Usage:
#   bash scripts/gates/source-tag-gate.sh                  # tag from HEAD
#   SOURCE_TAG_EXPECT=v0.9.2 bash scripts/gates/source-tag-gate.sh
#   bash scripts/gates/source-tag-gate.sh --selftest
#   EXYONQ_BIN=target/debug/exyonq bash scripts/gates/source-tag-gate.sh
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 2
BIN="${EXYONQ_BIN:-$ROOT/target/release/exyonq}"

field() { # field <name> <version-output>
  printf '%s\n' "$2" | sed -n "s/^$1=//p" | head -1
}

# check_version <tag> <head> <version-output> -> prints PASS/FAIL reason
check_version() {
  local tag=$1 head=$2 out=$3 got_tag got_rev
  got_tag="$(field source_tag "$out")"
  got_rev="$(field source_revision "$out")"
  if [[ -z "$got_tag" ]]; then
    echo "FAIL empty source_tag: the binary does not identify its tag (EXY-14)"
    return 1
  fi
  if [[ "$got_tag" != "$tag" ]]; then
    echo "FAIL source_tag=$got_tag expected=$tag"
    return 1
  fi
  if [[ "$got_rev" != "$head" ]]; then
    echo "FAIL source_revision=$got_rev expected=$head"
    return 1
  fi
  echo "PASS source_tag=$got_tag source_revision=$got_rev"
  return 0
}

selftest() {
  local head=0123456789abcdef0123456789abcdef01234567 failures=0
  expect() { # expect <want-rc> <name> <tag> <out>
    local want=$1 name=$2 tag=$3 out=$4 rc
    check_version "$tag" "$head" "$out" >/dev/null
    rc=$?
    if [[ "$rc" -eq "$want" ]]; then
      echo "PASS selftest $name"
    else
      echo "FAIL selftest $name expected_rc=$want got=$rc"
      failures=$((failures + 1))
    fi
  }
  expect 0 "matching tag" v0.9.2 "$(printf 'source_revision=%s\nsource_tag=v0.9.2\n' "$head")"
  expect 1 "empty tag" v0.9.2 "$(printf 'source_revision=%s\nsource_tag=\n' "$head")"
  expect 1 "wrong tag" v0.9.2 "$(printf 'source_revision=%s\nsource_tag=v0.9.1\n' "$head")"
  expect 1 "lab revision" v0.9.2 "$(printf 'source_revision=deadbeef\nsource_tag=v0.9.2\n')"
  if [[ "$failures" -ne 0 ]]; then
    echo "RESULT=FAIL SOURCE_TAG_GATE_SELFTEST"
    return 1
  fi
  echo "RESULT=PASS SOURCE_TAG_GATE_SELFTEST"
  return 0
}

if [[ "${1:-}" == "--selftest" ]]; then
  selftest
  exit $?
fi

tag="${SOURCE_TAG_EXPECT:-$(git describe --tags --exact-match 2>/dev/null || true)}"
head="$(git rev-parse HEAD 2>/dev/null || true)"

if [[ -z "$tag" ]]; then
  echo "RESULT=NOT_APPLICABLE SOURCE_TAG_GATE head=${head:0:7} (not a tag; set SOURCE_TAG_EXPECT to enforce)"
  exit 0
fi

if [[ ! -x "$BIN" ]]; then
  echo "FAIL missing binary: $BIN (build it with: cargo build --release -p exyonq --bin exyonq --locked)"
  echo "RESULT=FAIL SOURCE_TAG_GATE"
  exit 2
fi

out="$("$BIN" --version 2>&1)"
if check_version "$tag" "$head" "$out"; then
  echo "RESULT=PASS SOURCE_TAG_GATE tag=$tag"
  exit 0
fi
echo "binary=$BIN"
echo "RESULT=FAIL SOURCE_TAG_GATE tag=$tag"
exit 1
