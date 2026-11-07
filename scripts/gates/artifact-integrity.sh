#!/usr/bin/env bash
# Bind E2E evidence to one binary artifact.
#
# The failure this catches: a build or test step replaces target/debug/exyonq
# between the moment a harness records its hash and the moment the E2E actually
# executes it. A log that prints one hash is not enough; the artifact must be
# re-hashed after the tests and after the E2E, and the hash embedded in the log
# must match the recorded one.
#
# Usage:
#   artifact-integrity.sh hash   <binary>
#   artifact-integrity.sh record <state.tsv> <label> <binary>
#   artifact-integrity.sh verify <state.tsv> <label> <binary>
#   artifact-integrity.sh log    <state.tsv> <label> <log>
#
# Exit codes: 0 match, 1 usage/state error, 2 missing artifact/record,
# 3 drift (hash changed), 4 log has no or mismatched binary_sha256.
set -uo pipefail

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
  exit 1
}

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

file_size() {
  wc -c <"$1" | tr -d ' '
}

file_mtime() {
  if stat -c %Y "$1" >/dev/null 2>&1; then
    stat -c %Y "$1"
  else
    stat -f %m "$1"
  fi
}

recorded_hash() {
  # Last record wins so a state file can be reused intentionally.
  awk -F'\t' -v label="$2" '$1 == label { hash = $2 } END { print hash }' "$1"
}

cmd=${1:-}
case "$cmd" in
  hash)
    [[ $# -eq 2 && -f "$2" ]] || { echo "ARTIFACT_MISSING path=${2:-}" >&2; exit 2; }
    hash_file "$2"
    ;;
  record)
    [[ $# -eq 4 ]] || usage
    state=$2 label=$3 bin=$4
    if [[ ! -f "$bin" ]]; then
      echo "ARTIFACT_MISSING label=$label path=$bin" >&2
      exit 2
    fi
    sha=$(hash_file "$bin")
    size=$(file_size "$bin")
    mtime=$(file_mtime "$bin")
    printf '%s\t%s\t%s\t%s\t%s\n' "$label" "$sha" "$size" "$mtime" "$bin" >>"$state"
    echo "ARTIFACT_RECORD label=$label sha256=$sha size=$size mtime_epoch=$mtime path=$bin"
    ;;
  verify)
    [[ $# -eq 4 ]] || usage
    state=$2 label=$3 bin=$4
    expected=$(recorded_hash "$state" "$label")
    if [[ -z "$expected" ]]; then
      echo "ARTIFACT_NO_RECORD label=$label state=$state" >&2
      exit 2
    fi
    if [[ ! -f "$bin" ]]; then
      echo "ARTIFACT_MISSING label=$label path=$bin expected=$expected" >&2
      exit 2
    fi
    actual=$(hash_file "$bin")
    if [[ "$expected" == "$actual" ]]; then
      echo "ARTIFACT_MATCH label=$label sha256=$actual"
    else
      echo "ARTIFACT_DRIFT label=$label expected=$expected actual=$actual path=$bin" >&2
      exit 3
    fi
    ;;
  log)
    [[ $# -eq 4 ]] || usage
    state=$2 label=$3 log=$4
    expected=$(recorded_hash "$state" "$label")
    if [[ -z "$expected" ]]; then
      echo "ARTIFACT_NO_RECORD label=$label state=$state" >&2
      exit 2
    fi
    if [[ ! -f "$log" ]]; then
      echo "ARTIFACT_LOG_MISSING label=$label log=$log" >&2
      exit 2
    fi
    seen=0
    while IFS= read -r actual; do
      seen=$((seen + 1))
      if [[ "$actual" != "$expected" ]]; then
        echo "ARTIFACT_LOG_DRIFT label=$label expected=$expected actual=$actual log=$log" >&2
        exit 3
      fi
    done < <(sed -n 's/.*binary_sha256=\([0-9a-f]\{64\}\).*/\1/p' "$log")
    if [[ $seen -eq 0 ]]; then
      echo "ARTIFACT_LOG_UNBOUND label=$label log=$log" >&2
      exit 4
    fi
    echo "ARTIFACT_LOG_BOUND label=$label sha256=$expected occurrences=$seen log=$log"
    ;;
  *)
    usage
    ;;
esac
