#!/usr/bin/env bash
# P1.5-WS2 — run bounded fuzz campaigns.
# Modes: smoke | main | --target NAME --mode smoke|main
# Fail-closed if cargo-fuzz missing.
set -euo pipefail
source "${HOME}/.cargo/env" 2>/dev/null || true
export PATH="${HOME}/.cargo/bin:${PATH:-}"

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if ! command -v cargo-fuzz >/dev/null 2>&1; then
  echo "FATAL: cargo-fuzz missing" >&2
  exit 2
fi

TOOLCHAIN="${P15_WS2_TOOLCHAIN:-nightly}"
MODE="${P15_WS2_MODE:-smoke}"
ONLY_TARGET="${P15_WS2_TARGET:-}"
RUN_ID="${P15_WS2_RUN_ID:-p15-ws2-$(date -u +%Y%m%dT%H%M%SZ)}"
LOCK_ID="${P15_WS2_LOCK_ID:-}"
EV_DIR="${P15_WS2_EVIDENCE_DIR:-$ROOT/docs/operations/evidence/p1.5-ws2/$RUN_ID}"
mkdir -p "$EV_DIR"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode) MODE="$2"; shift 2 ;;
    --target) ONLY_TARGET="$2"; shift 2 ;;
    --run-id) RUN_ID="$2"; shift 2 ;;
    --lock-id) LOCK_ID="$2"; shift 2 ;;
    --evidence-dir) EV_DIR="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

# Evidence dir must be absolute — campaigns `cd` into fuzz/.
[[ "$EV_DIR" = /* ]] || EV_DIR="$ROOT/$EV_DIR"
mkdir -p "$EV_DIR"

# Budget per target (seconds or runs) — not one global arbitrary duration.
# smoke: short CI-like; main: bounded campaign.
declare -A BUDGET_SMOKE BUDGET_MAIN
BUDGET_SMOKE[header_end]="-runs=5000"
BUDGET_SMOKE[static_request_line]="-runs=5000"
BUDGET_SMOKE[config_parse]="-runs=3000"
BUDGET_SMOKE[path_resolve]="-runs=5000"
BUDGET_SMOKE[fcgi_record]="-runs=5000"
BUDGET_SMOKE[proxy_headers]="-runs=5000"
BUDGET_SMOKE[nginx_migrate]="-runs=2000"
BUDGET_SMOKE[htaccess_parse]="-runs=2000"

BUDGET_MAIN[header_end]="-max_total_time=90"
BUDGET_MAIN[static_request_line]="-max_total_time=60"
BUDGET_MAIN[config_parse]="-max_total_time=90"
BUDGET_MAIN[path_resolve]="-max_total_time=60"
BUDGET_MAIN[fcgi_record]="-max_total_time=60"
BUDGET_MAIN[proxy_headers]="-max_total_time=60"
BUDGET_MAIN[nginx_migrate]="-max_total_time=45"
BUDGET_MAIN[htaccess_parse]="-max_total_time=45"

TIMEOUT="${P15_WS2_TIMEOUT:-25}"
RSS_LIMIT="${P15_WS2_RSS_LIMIT_MB:-2048}"

ALL_TARGETS=(header_end static_request_line config_parse path_resolve fcgi_record proxy_headers nginx_migrate htaccess_parse)
if [[ -n "$ONLY_TARGET" ]]; then
  TARGETS=("$ONLY_TARGET")
else
  TARGETS=("${ALL_TARGETS[@]}")
fi

HEAD="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo nogit)"
HOST="$(hostname -s 2>/dev/null || hostname)"
ARCH="$(uname -m)"
RUSTC="$(RUSTUP_TOOLCHAIN=$TOOLCHAIN rustc --version 2>/dev/null || rustc --version)"
CFVER="$(cargo fuzz --version 2>/dev/null || echo unknown)"

meta() {
  local target="$1" exitc="$2" crashes="$3" hangs="$4" oom="$5" artifact="$6" budget="$7"
  cat >>"$EV_DIR/${target}.${MODE}.meta.env" <<EOF
RUN_ID=$RUN_ID
HEAD=$HEAD
LOCK_ID=$LOCK_ID
HOST=$HOST
ARCH=$ARCH
RUSTC=$RUSTC
CARGO_FUZZ_VERSION=$CFVER
TARGET=$target
MODE=$MODE
BUDGET=$budget
TIMEOUT=$TIMEOUT
RSS_LIMIT=${RSS_LIMIT}MB
ARTIFACT_PATH=$artifact
EXIT=$exitc
CRASHES=$crashes
HANGS=$hangs
OOM=$oom
EOF
}

FAIL=0
cd "$ROOT/fuzz"
for target in "${TARGETS[@]}"; do
  if [[ "$MODE" == "main" ]]; then
    budget="${BUDGET_MAIN[$target]}"
  else
    budget="${BUDGET_SMOKE[$target]}"
  fi
  # Writable work corpus only — do not pass curated seed/negative dirs (libFuzzer writes into them).
  work="corpus/$target/work"
  mkdir -p "$work"
  # Stage curated inputs into work (named files only) before campaign.
  for kind in seed negative; do
    src="corpus/$target/$kind"
    [[ -d "$src" ]] || continue
    find "$src" -maxdepth 1 -type f 2>/dev/null | while read -r f; do
      base="$(basename "$f")"
      [[ "$base" =~ ^[0-9a-f]{40}$ ]] && continue
      cp -f "$f" "$work/$base"
    done
  done
  log="$EV_DIR/${target}.${MODE}.log"
  art="artifacts/$target"
  mkdir -p "$art"
  echo "=== RUN $target mode=$MODE budget=$budget ===" | tee "$log"
  set +e
  # shellcheck disable=SC2086
  RUSTUP_TOOLCHAIN="$TOOLCHAIN" cargo fuzz run "$target" -- \
    -timeout="$TIMEOUT" -rss_limit_mb="$RSS_LIMIT" $budget \
    "$work" \
    >>"$log" 2>&1
  rc=$?
  set -e
  crashes=$(find "$art" -type f 2>/dev/null | wc -l | tr -d ' ')
  hangs=0
  oom=0
  if rg -q 'out-of-memory|RSS exceeded' "$log" 2>/dev/null; then oom=1; fi
  if rg -q 'ERROR: libFuzzer: timeout' "$log" 2>/dev/null; then hangs=1; fi
  meta "$target" "$rc" "$crashes" "$hangs" "$oom" "$art" "$budget"
  if [[ "$rc" -ne 0 ]]; then
    echo "FAIL $target rc=$rc" | tee -a "$log"
    FAIL=$((FAIL + 1))
  else
    echo "PASS $target" | tee -a "$log"
  fi
done

echo "RUN_ID=$RUN_ID HOST=$HOST ARCH=$ARCH MODE=$MODE FAIL=$FAIL" | tee "$EV_DIR/summary.${MODE}.txt"
if [[ "$FAIL" -ne 0 ]]; then
  echo "P15_WS2_FUZZ=FAIL"
  exit 1
fi
echo "P15_WS2_FUZZ=PASS"
