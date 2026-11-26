#!/usr/bin/env bash
# Run the ExyonQ fuzz targets. Requires cargo-fuzz and a nightly toolchain.
#
#   bash scripts/security-fuzz.sh --ci
#       Quick gate used by Security Nightly: the five parser targets, 10 000
#       runs each.
#
#   bash scripts/security-fuzz.sh --mode main [--out DIR] [--budget SECONDS]
#       The full matrix: every fuzz target declared as [[bin]] in
#       fuzz/Cargo.toml, run for BUDGET seconds each (default 60). With --out it
#       writes one log and one meta.env per target plus summary.main.txt, so a
#       scheduled run leaves evidence instead of a terminal scroll.
#
# Both modes fail closed when cargo-fuzz is missing: a security gate that
# "skips" is a gate that cannot fail.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
FUZZ_DIR="$ROOT/fuzz"
CI_TARGETS=(header_end static_request_line config_parse path_resolve fcgi_record)

MODE=ci
OUT=""
BUDGET=60
while [ $# -gt 0 ]; do
  case "$1" in
    --ci) MODE=ci ;;
    --mode)
      MODE="${2:-}"
      shift
      ;;
    --out)
      OUT="${2:-}"
      shift
      ;;
    --budget)
      BUDGET="${2:-}"
      shift
      ;;
    *)
      echo "security-fuzz: unknown argument '$1'" >&2
      exit 2
      ;;
  esac
  shift
done

case "$MODE" in
  ci | main) ;;
  *)
    echo "security-fuzz: --mode must be 'main' (or use --ci)" >&2
    exit 2
    ;;
esac

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "security-fuzz: cargo-fuzz is not installed (cargo install cargo-fuzz --locked)" >&2
  exit 2
fi
if ! cargo +nightly --version >/dev/null 2>&1; then
  echo "security-fuzz: the nightly toolchain is required (-Zsanitizer), 'cargo +nightly' is unavailable" >&2
  exit 2
fi

# Targets declared as [[bin]] in fuzz/Cargo.toml, in file order.
main_targets() {
  awk '
    /^\[\[bin\]\]/ { inbin = 1; next }
    /^\[/ { inbin = 0 }
    inbin && /^name *= *"/ { line = $0; sub(/^[^"]*"/, "", line); sub(/".*/, "", line); print line }
  ' "$FUZZ_DIR/Cargo.toml"
}

cd "$FUZZ_DIR"

if [ "$MODE" = ci ]; then
  # A disposable corpus: without an explicit directory libFuzzer writes the
  # generated inputs next to the tracked seeds in fuzz/corpus/<target>/, which
  # leaves hundreds of untracked files behind.
  CI_WORK="$ROOT/target/fuzz-ci-corpus"
  for target in "${CI_TARGETS[@]}"; do
    mkdir -p "$CI_WORK/$target" "$CI_WORK/$target.artifacts"
    echo "Fuzzing $target ..."
    cargo +nightly fuzz run "$target" "$CI_WORK/$target" -- \
      -runs=10000 -artifact_prefix="$CI_WORK/$target.artifacts/"
  done
  echo "Fuzz complete."
  exit 0
fi

TARGETS="$(main_targets)"
if [ -z "$TARGETS" ]; then
  echo "security-fuzz: no [[bin]] fuzz targets found in fuzz/Cargo.toml" >&2
  exit 2
fi

RUN_ID="main-$(date -u +%Y%m%dT%H%M%SZ)"
if [ -z "$OUT" ]; then
  OUT="$ROOT/.exyonq-local/evidence/fuzz/$RUN_ID"
fi
# We are already inside fuzz/; a relative --out would land under fuzz/ by
# accident instead of next to the tree it documents.
case "$OUT" in
  /*) ;;
  *) OUT="$ROOT/$OUT" ;;
esac
mkdir -p "$OUT/logs" "$OUT/corpus" "$OUT/artifacts"

HEAD_REV="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
HOST_NAME="$(hostname -s 2>/dev/null || echo unknown)"
ARCH="$(uname -m)"
RUSTC_TAG="$(rustc +nightly --version 2>/dev/null | head -1 || echo unknown)"
FUZZ_VERSION="$(cargo fuzz --version 2>/dev/null | head -1 || echo unknown)"

fail=0
for target in $TARGETS; do
  log="$OUT/logs/$target.main.log"
  meta="$OUT/logs/$target.main.meta.env"
  work_corpus="$OUT/corpus/$target"
  art="$OUT/artifacts/$target"
  mkdir -p "$work_corpus" "$art"
  echo "Fuzzing $target (main, ${BUDGET}s) ..."
  set +e
  cargo +nightly fuzz run "$target" "$work_corpus" -- \
    -max_total_time="$BUDGET" -timeout=25 -rss_limit_mb=2048 -artifact_prefix="$art/" \
    >"$log" 2>&1
  rc=$?
  set -e

  crashes="$(find "$art" -type f 2>/dev/null | wc -l | tr -d ' ')"
  hangs=0
  oom=0
  if [ "$rc" -eq 77 ]; then hangs=1; fi
  if [ "$rc" -eq 71 ]; then oom=1; fi
  if [ "$crashes" -ne 0 ]; then
    echo "FAIL $target: $crashes artifact(s) in $art" >&2
    fail=1
  elif [ "$rc" -ne 0 ]; then
    echo "FAIL $target: exit $rc with no artifact (see $log)" >&2
    fail=1
  else
    echo "PASS $target"
  fi

  {
    echo "RUN_ID=$RUN_ID"
    echo "HEAD=$HEAD_REV"
    echo "LOCK_ID="
    echo "HOST=$HOST_NAME"
    echo "ARCH=$ARCH"
    echo "RUSTC=$RUSTC_TAG"
    echo "CARGO_FUZZ_VERSION=$FUZZ_VERSION"
    echo "TARGET=$target"
    echo "MODE=main"
    echo "BUDGET=-max_total_time=$BUDGET"
    echo "TIMEOUT=25"
    echo "RSS_LIMIT=2048MB"
    echo "ARTIFACT_PATH=artifacts/$target"
    echo "WORK_CORPUS=corpus/$target"
    echo "LOG=logs/$target.main.log"
    echo "EXIT=$rc"
    echo "CRASHES=$crashes"
    echo "HANGS=$hangs"
    echo "OOM=$oom"
  } >"$meta"
done

printf 'RUN_ID=%s HOST=%s ARCH=%s MODE=main FAIL=%s\n' \
  "$RUN_ID" "$HOST_NAME" "$ARCH" "$fail" | tee "$OUT/summary.main.txt"
echo "Evidence: $OUT"
exit "$fail"
