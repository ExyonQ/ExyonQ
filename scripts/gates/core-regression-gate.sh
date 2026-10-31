#!/usr/bin/env bash
# Core Regression Gate — Static / Proxy / FastCGI / Operations
#
# USAGE:
#   bash scripts/gates/core-regression-gate.sh [--evidence-root DIR] [--run-id ID]
#
# Ephemeral state defaults under /Volumes/Lexar/Cursor/temp/ (WIP rule).
# Exit 0 only when all four CORE_* axes PASS.
# Child exit 2 → axis NOT_EXECUTED (ENVIRONMENT); exit 3 → axis BLOCKED.
# First PRODUCT/HARNESS failure stops the run (fail-closed).
#
# RULE: USES_REAL_DATA=YES — no smoke/mock/port-only substitutes.
set -euo pipefail
LC_ALL=C
export LC_ALL

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
cd "$REPO_ROOT"

EVIDENCE_ROOT=""
RUN_ID=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --evidence-root) EVIDENCE_ROOT="${2:-}"; shift 2 ;;
    --run-id) RUN_ID="${2:-}"; shift 2 ;;
    -h|--help)
      sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

RUN_ID="${RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
if [[ -z "$EVIDENCE_ROOT" ]]; then
  EVIDENCE_ROOT="/Volumes/Lexar/Cursor/temp/exyonq-core-regression-${RUN_ID}"
fi
mkdir -p "$EVIDENCE_ROOT"/{meta,cases,summary,cleanup,target}

# Isolate Cargo artifacts away from the canonical tree when building.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$EVIDENCE_ROOT/target}"
export EXYONQ_BIN="${EXYONQ_BIN:-$CARGO_TARGET_DIR/debug/exyonq}"
export EXYONQCTL_BIN="${EXYONQCTL_BIN:-$CARGO_TARGET_DIR/debug/exyonqctl}"

CONSOLE="$EVIDENCE_ROOT/summary/console.log"
AXES_ENV="$EVIDENCE_ROOT/summary/axes.env"
RESULT_ENV="$EVIDENCE_ROOT/summary/result.env"
LEDGER="$EVIDENCE_ROOT/cleanup/process-ledger.txt"
: >"$CONSOLE"
: >"$LEDGER"

log() { printf '%s\n' "$*" | tee -a "$CONSOLE"; }

classify_child_exit() {
  local ec="$1"
  case "$ec" in
    0) echo PASS ;;
    2) echo NOT_EXECUTED ;;
    3) echo BLOCKED ;;
    *) echo FAIL ;;
  esac
}

attribute_failure() {
  local axis="$1" ec="$2" status="$3"
  case "$status" in
    FAIL)
      if [[ "$ec" -ge 128 ]]; then
        echo "PRODUCT (signal/crash ec=$ec axis=$axis)"
      else
        echo "PRODUCT (axis=$axis exit=$ec)"
      fi
      ;;
    BLOCKED) echo "ENVIRONMENT (axis=$axis blocked prerequisite)" ;;
    NOT_EXECUTED) echo "ENVIRONMENT (axis=$axis not executed on this platform)" ;;
    *) echo "UNKNOWN (axis=$axis status=$status ec=$ec)" ;;
  esac
}

ensure_bins() {
  if [[ ! -x "$EXYONQ_BIN" ]]; then
    log "BUILD: cargo build -p exyonq --bin exyonq (CARGO_TARGET_DIR=$CARGO_TARGET_DIR)"
    cargo build -p exyonq --bin exyonq
  fi
  if [[ ! -x "$EXYONQCTL_BIN" ]]; then
    log "BUILD: cargo build -p exyonqctl --bin exyonqctl"
    cargo build -p exyonqctl --bin exyonqctl
  fi
  [[ -x "$EXYONQ_BIN" ]] || {
    log "RESULT=BLOCKED missing=exyonq binary"
    echo "FAILURE_ATTRIBUTION=HARNESS (binary build)"
    exit 3
  }
}

run_case() {
  local axis="$1" name="$2" script="$3"
  local case_dir="$EVIDENCE_ROOT/cases/${axis}_${name}"
  mkdir -p "$case_dir"
  local stdout_f="$case_dir/stdout.txt"
  local stderr_f="$case_dir/stderr.txt"
  local case_env="$case_dir/case.env"

  log "=== BEGIN $axis / $name ==="
  if [[ ! -f "$script" ]]; then
    log "RESULT=BLOCKED missing_script=$script"
    {
      echo "AXIS=$axis"
      echo "CASE=$name"
      echo "STATUS=BLOCKED"
      echo "EXIT_CODE=3"
      echo "ATTRIBUTION=HARNESS (missing $script)"
    } >"$case_env"
    return 3
  fi

  set +e
  bash "$script" >"$stdout_f" 2>"$stderr_f"
  local ec=$?
  set +e
  local status
  status="$(classify_child_exit "$ec")"
  {
    echo "AXIS=$axis"
    echo "CASE=$name"
    echo "STATUS=$status"
    echo "EXIT_CODE=$ec"
    echo "SCRIPT=$script"
    echo "STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$case_env"
  echo "pid=child axis=$axis case=$name ec=$ec status=$status" >>"$LEDGER"
  log "=== END $axis / $name status=$status ec=$ec ==="
  return "$ec"
}

axis_combine() {
  # Combine multiple case statuses for one axis: FAIL > BLOCKED > NOT_EXECUTED > PASS
  local worst=PASS
  local s
  for s in "$@"; do
    case "$s" in
      FAIL) worst=FAIL ;;
      BLOCKED) [[ "$worst" != FAIL ]] && worst=BLOCKED ;;
      NOT_EXECUTED) [[ "$worst" != FAIL && "$worst" != BLOCKED ]] && worst=NOT_EXECUTED ;;
      PASS) ;;
      *) [[ "$worst" == PASS ]] && worst=UNKNOWN ;;
    esac
  done
  echo "$worst"
}

# --- meta ---
{
  echo "RUN_ID=$RUN_ID"
  echo "EVIDENCE_ROOT=$EVIDENCE_ROOT"
  echo "REPO_ROOT=$REPO_ROOT"
  echo "PLATFORM=$(uname -s)"
  echo "ARCH=$(uname -m)"
  echo "HEAD=$(git rev-parse HEAD 2>/dev/null || echo UNKNOWN)"
  echo "TREE=$(git rev-parse 'HEAD^{tree}' 2>/dev/null || echo UNKNOWN)"
  echo "EXYONQ_BIN=$EXYONQ_BIN"
  echo "CARGO_TARGET_DIR=$CARGO_TARGET_DIR"
  echo "STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_ROOT/meta/run.env"

log "CORE_REGRESSION_GATE start run_id=$RUN_ID evidence=$EVIDENCE_ROOT"
ensure_bins

CORE_STATIC=UNKNOWN
CORE_PROXY=UNKNOWN
CORE_FASTCGI=UNKNOWN
CORE_OPERATIONS=UNKNOWN
OVERALL=PASS
FAILURE_ATTRIBUTION=NONE
HTTP_OR_TRANSPORT_ERRORS=0
PROCESS_PANICS=0
PROCESS_CRASHES=0

# --- CORE_STATIC ---
set +e
run_case CORE_STATIC static-e2e "$REPO_ROOT/scripts/e2e/static-e2e.sh"
STAT_EC=$?
set -e
CORE_STATIC="$(classify_child_exit "$STAT_EC")"
if [[ "$CORE_STATIC" != PASS ]]; then
  OVERALL=BLOCKED
  if [[ "$CORE_STATIC" == FAIL ]]; then
    OVERALL=FAIL
  fi
  FAILURE_ATTRIBUTION="$(attribute_failure CORE_STATIC "$STAT_EC" "$CORE_STATIC")"
  if [[ "$OVERALL" == FAIL ]]; then
    log "STOP on first PRODUCT failure (CORE_STATIC)"
  fi
fi

# --- CORE_PROXY ---
if [[ "$OVERALL" != FAIL ]]; then
  set +e
  run_case CORE_PROXY proxy-e2e "$REPO_ROOT/scripts/e2e/proxy-e2e.sh"
  PROXY_EC=$?
  set -e
  CORE_PROXY="$(classify_child_exit "$PROXY_EC")"
  if [[ "$CORE_PROXY" != PASS ]]; then
    OVERALL=BLOCKED
    if [[ "$CORE_PROXY" == FAIL ]]; then
      OVERALL=FAIL
    fi
    FAILURE_ATTRIBUTION="$(attribute_failure CORE_PROXY "$PROXY_EC" "$CORE_PROXY")"
    if [[ "$OVERALL" == FAIL ]]; then
      log "STOP on first PRODUCT failure (CORE_PROXY)"
    fi
  fi
else
  CORE_PROXY=SKIPPED
fi

# --- CORE_FASTCGI (php-fpm suite + wordpress-shaped PHP app) ---
if [[ "$OVERALL" != FAIL ]]; then
  set +e
  run_case CORE_FASTCGI php-fpm "$REPO_ROOT/scripts/e2e/fastcgi-php-fpm-e2e.sh"
  FCGI1_EC=$?
  set -e
  FCGI1="$(classify_child_exit "$FCGI1_EC")"
  set +e
  run_case CORE_FASTCGI wordpress-profile "$REPO_ROOT/scripts/e2e/03b-wordpress-profile.sh"
  FCGI2_EC=$?
  set -e
  FCGI2="$(classify_child_exit "$FCGI2_EC")"
  CORE_FASTCGI="$(axis_combine "$FCGI1" "$FCGI2")"
  if [[ "$CORE_FASTCGI" != PASS ]]; then
    OVERALL=BLOCKED
    if [[ "$CORE_FASTCGI" == FAIL ]]; then
      OVERALL=FAIL
    fi
    if [[ "$FCGI1" == FAIL ]]; then
      FAILURE_ATTRIBUTION="$(attribute_failure CORE_FASTCGI "$FCGI1_EC" FAIL)"
    elif [[ "$FCGI2" == FAIL ]]; then
      FAILURE_ATTRIBUTION="$(attribute_failure CORE_FASTCGI "$FCGI2_EC" FAIL)"
    else
      FAILURE_ATTRIBUTION="$(attribute_failure CORE_FASTCGI "$FCGI1_EC" "$CORE_FASTCGI")"
    fi
    if [[ "$OVERALL" == FAIL ]]; then
      log "STOP on first PRODUCT failure (CORE_FASTCGI)"
    fi
  fi
else
  CORE_FASTCGI=SKIPPED
fi

# --- CORE_OPERATIONS ---
if [[ "$OVERALL" != FAIL ]]; then
  set +e
  run_case CORE_OPERATIONS operations "$REPO_ROOT/scripts/e2e/04-operations.sh"
  OPS_EC=$?
  set -e
  CORE_OPERATIONS="$(classify_child_exit "$OPS_EC")"
  if [[ "$CORE_OPERATIONS" != PASS ]]; then
    OVERALL=BLOCKED
    if [[ "$CORE_OPERATIONS" == FAIL ]]; then
      OVERALL=FAIL
    fi
    FAILURE_ATTRIBUTION="$(attribute_failure CORE_OPERATIONS "$OPS_EC" "$CORE_OPERATIONS")"
  fi
else
  CORE_OPERATIONS=SKIPPED
fi

# Process crash scan across case logs — real runtime signatures only.
# Do not match PASS prose that mentions the word "panic".
set +e
grep -REiq "thread '.*' panicked|panicked at|fatal runtime error|SIGSEGV|Aborted \\(core dumped\\)" \
  "$EVIDENCE_ROOT/cases" 2>/dev/null
GREP_EC=$?
set -e
if [[ "$GREP_EC" -eq 0 ]]; then
  PROCESS_PANICS=1
  if [[ "$OVERALL" == PASS ]]; then
    OVERALL=FAIL
    FAILURE_ATTRIBUTION="PRODUCT (process crash marker in case logs)"
  fi
fi

{
  echo "CORE_STATIC=$CORE_STATIC"
  echo "CORE_PROXY=$CORE_PROXY"
  echo "CORE_FASTCGI=$CORE_FASTCGI"
  echo "CORE_OPERATIONS=$CORE_OPERATIONS"
} >"$AXES_ENV"

if [[ "$OVERALL" == PASS ]]; then
  GATE_VERDICT=PASS
  EXIT_CODE=0
elif [[ "$OVERALL" == BLOCKED ]]; then
  GATE_VERDICT=BLOCKED
  EXIT_CODE=3
else
  GATE_VERDICT=FAIL
  EXIT_CODE=1
fi

{
  echo "GATE_VERDICT=$GATE_VERDICT"
  echo "FAILURE_ATTRIBUTION=$FAILURE_ATTRIBUTION"
  echo "HTTP_OR_TRANSPORT_ERRORS=$HTTP_OR_TRANSPORT_ERRORS"
  echo "PROCESS_PANICS=$PROCESS_PANICS"
  echo "PROCESS_CRASHES=$PROCESS_CRASHES"
  echo "ENDED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "EXIT_CODE=$EXIT_CODE"
} >"$RESULT_ENV"

python3 - <<PY >"$EVIDENCE_ROOT/summary/summary.json"
import json
print(json.dumps({
  "run_id": "$RUN_ID",
  "verdict": "$GATE_VERDICT",
  "axes": {
    "CORE_STATIC": "$CORE_STATIC",
    "CORE_PROXY": "$CORE_PROXY",
    "CORE_FASTCGI": "$CORE_FASTCGI",
    "CORE_OPERATIONS": "$CORE_OPERATIONS",
  },
  "failure_attribution": "$FAILURE_ATTRIBUTION",
  "evidence_root": "$EVIDENCE_ROOT",
}, indent=2))
PY

log "GATE_VERDICT=$GATE_VERDICT"
log "CORE_STATIC=$CORE_STATIC CORE_PROXY=$CORE_PROXY CORE_FASTCGI=$CORE_FASTCGI CORE_OPERATIONS=$CORE_OPERATIONS"
log "FAILURE_ATTRIBUTION=$FAILURE_ATTRIBUTION"
log "EVIDENCE=$EVIDENCE_ROOT"

exit "$EXIT_CODE"
