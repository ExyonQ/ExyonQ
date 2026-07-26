#!/usr/bin/env bash
# Phase 0 kernel boundary lint — contract: study/reports/11-phase0-kernel-freeze-signoff.md
#
# Tooling only. No runtime changes. Detects architectural drift vs Phase 0 freeze.
#
# Usage:
#   bash scripts/architecture/verify-phase0-kernel.sh
#   EXYONQ_PHASE0_KERNEL_STRICT=1 bash scripts/architecture/verify-phase0-kernel.sh
#   bash scripts/architecture/verify-phase0-kernel.sh --report study/reports/KERNEL-BOUNDARY-LINT-REPORT.md
#   bash scripts/architecture/verify-phase0-kernel.sh --selftest
#
# Env:
#   EXYONQ_PHASE0_KERNEL_ROOT   repo root (default: auto)
#   EXYONQ_PHASE0_KERNEL_STRICT 1 = fail on violations (allowlisted drift OK)
set -euo pipefail

_SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_REPO_ROOT="$(cd "$_SCRIPT_DIR/../.." && pwd)"
ROOT="${EXYONQ_PHASE0_KERNEL_ROOT:-$_REPO_ROOT}"
ALLOWLIST="${EXYONQ_PHASE0_ALLOWLIST:-$_SCRIPT_DIR/phase0-core-mod-allowlist.txt}"

STRICT="${EXYONQ_PHASE0_KERNEL_STRICT:-0}"
REPORT_PATH=""
SELFTEST=0

VIOLATIONS=0
WARNINGS=0
INFOS=0

REPORT_LINES=()
REPORT_MD=()

usage() {
  sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --report)
      REPORT_PATH="${2:-}"
      shift 2
      ;;
    --selftest)
      SELFTEST=1
      shift
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "verify-phase0-kernel: unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

cd "$ROOT"

log_violation() {
  VIOLATIONS=$((VIOLATIONS + 1))
  echo "verify-phase0-kernel: VIOLATION: $*" >&2
  REPORT_LINES+=("VIOLATION: $*")
}

log_warning() {
  WARNINGS=$((WARNINGS + 1))
  echo "verify-phase0-kernel: WARNING: $*" >&2
  REPORT_LINES+=("WARNING: $*")
}

log_info() {
  INFOS=$((INFOS + 1))
  echo "verify-phase0-kernel: INFO: $*"
  REPORT_LINES+=("INFO: $*")
}

log_pass() {
  echo "verify-phase0-kernel: PASS: $*"
  REPORT_LINES+=("PASS: $*")
}

# Return 0 if match line is covered by allowlist entry (path prefix or path:subpattern).
allowlisted() {
  local file="$1"
  local line_content="$2"
  local entry
  [[ -f "$ALLOWLIST" ]] || return 1
  while IFS= read -r entry || [[ -n "$entry" ]]; do
    entry="${entry%%#*}"
    entry="$(echo "$entry" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
    [[ -z "$entry" ]] && continue
    if [[ "$entry" == *:* ]]; then
      local prefix="${entry%%:*}"
      local sub="${entry#*:}"
      [[ "$file" == "$prefix"* ]] || continue
      if echo "$line_content" | grep -qE "$sub"; then
        return 0
      fi
    elif [[ "$file" == "$entry"* ]] || [[ "$file" == "$entry" ]]; then
      return 0
    fi
  done <"$ALLOWLIST"
  return 1
}

# Scan Rust sources: scan_rg LABEL path... -- pattern...
scan_rg() {
  local _label="$1"
  shift
  local paths=()
  while [[ $# -gt 0 && "$1" != "--" ]]; do
    paths+=("$1")
    shift
  done
  [[ "${1:-}" == "--" ]] && shift
  local patterns=("$@")

  if [[ ${#paths[@]} -eq 0 || ${#patterns[@]} -eq 0 ]]; then
    return 0
  fi

  local pat matches
  for pat in "${patterns[@]}"; do
    matches="$(rg -n "$pat" "${paths[@]}" --type rust 2>/dev/null || true)"
    if [[ -n "$matches" ]]; then
      echo "$matches"
    fi
  done
}

# --- Check: core → module crate imports (allowlisted) ---
check_core_to_mod() {
  local core_rust="core"
  [[ -d "$core_rust" ]] || return 0

  local patterns=(
    'use exyonq_(compression|metrics|ratelimit|acme)'
    'exyonq_(compression|metrics|ratelimit|acme)::'
    'CompressionModule|MetricsModule|RateLimitModule'
    'register_runtime_prometheus_append'
  )

  local line file lineno content rel
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    content="${rest#*:}"
    rel="${file#./}"

    if allowlisted "$rel" "$content"; then
      REPORT_LINES+=("ALLOWLIST: $rel:$lineno $content")
      continue
    fi

    log_warning "core→mod coupling outside allowlist at $rel:$lineno"
    echo "  $rel:$lineno:$content" >&2
  done < <(scan_rg "core-mod" core -- "${patterns[@]}" | sort -u)

  # Cargo.toml module path deps
  if [[ -f core/Cargo.toml ]]; then
    local dep_line
    while IFS= read -r dep_line; do
      [[ -z "$dep_line" ]] && continue
      if allowlisted "core/Cargo.toml" "$dep_line"; then
        REPORT_LINES+=("ALLOWLIST: core/Cargo.toml $dep_line")
      else
        log_warning "core/Cargo.toml module dependency outside allowlist"
        echo "  core/Cargo.toml:$dep_line" >&2
      fi
    done < <(grep -nE 'path = "\.\./modules/' core/Cargo.toml 2>/dev/null || true)
  fi
}

# --- Check: HandlerTable forbidden in kernel hot path ---
check_handler_table() {
  local matches
  matches="$(rg -n 'HandlerTable' core/src --type rust 2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    log_violation "HandlerTable found in core/src (forbidden — use BackendTable)"
    echo "$matches" >&2
  else
    log_pass "no HandlerTable in core/src"
  fi
}

# --- Check: FastCGI / PHP-FPM runtime pre-Plan-08 ---
check_fastcgi_runtime() {
  local paths=(core modules)
  local patterns=(
    'PhpFpmClient'
    'FastcgiBackend'
    'FastCGIBackend'
    'struct Fastcgi'
    'mod fastcgi'
    'fastcgi_pool'
  )
  local matches
  matches="$(scan_rg "fastcgi" core modules -- "${patterns[@]}" | sort -u || true)"
  if [[ -n "$matches" ]]; then
    log_violation "FastCGI runtime symbols before Plan 08 gate"
    echo "$matches" >&2
  else
    log_pass "no FastCGI runtime symbols in core/modules"
  fi
}

# --- Check: WordPress / Enterprise / Admin HTTP in hot path ---
check_forbidden_hotpath() {
  local hot_paths=(
    core/src/server
    core/src/proxy
    core/src/static_files
    core/src/modules
    core/src/router
    core/src/snapshot
  )
  local existing=()
  local p
  for p in "${hot_paths[@]}"; do
    [[ -e "$p" ]] && existing+=("$p")
  done
  [[ ${#existing[@]} -gt 0 ]] || return 0

  local patterns=(
    'WordPress|Wordpress'
    'Enterprise'
    'Admin API|AdminApi|admin_api'
    'dlopen|libloading'
    'dynamic plugin|plugin discovery|PluginDiscovery'
  )
  local matches
  matches="$(scan_rg "hotpath" "${existing[@]}" -- "${patterns[@]}" | sort -u || true)"
  if [[ -n "$matches" ]]; then
    log_violation "forbidden hot-path symbol (WordPress/Enterprise/Admin/dynamic plugin)"
    echo "$matches" >&2
  else
    log_pass "no WordPress/Enterprise/Admin/dynamic-plugin in kernel hot paths"
  fi
}

# --- Check: Phase 0 dispatch vocabulary in core/src ---
check_dispatch_vocabulary() {
  local core_src="core/src"
  [[ -d "$core_src" ]] || return 0

  local sym
  for sym in BackendTable RouteDecision BackendId; do
    if rg -q "$sym" "$core_src" --type rust 2>/dev/null; then
      log_pass "$sym present in core/src"
    else
      log_info "EXPECTED-GAP: $sym absent in core/src (target PR-4+)"
    fi
  done

  if rg -q 'enum Backend|Backend::' "$core_src" --type rust 2>/dev/null; then
    log_pass "Backend enum references in core/src"
  else
    log_info "EXPECTED-GAP: Backend enum absent in core/src (target PR-4+)"
  fi

  if rg -q 'RuntimePlan' "$core_src" --type rust 2>/dev/null; then
    log_pass "RuntimePlan symbol in core/src"
  elif rg -q 'RuntimeSnapshot' "$core_src" --type rust 2>/dev/null; then
    log_info "RuntimeSnapshot in core/src; RuntimePlan alias pending (PR-2)"
  else
    log_violation "neither RuntimePlan nor RuntimeSnapshot in core/src"
  fi

  if rg -q 'HandlerTable' "$core_src" --type rust 2>/dev/null; then
    : # handled above
  else
    log_pass "BackendTable is canonical handler binding (HandlerTable absent)"
  fi
}

# --- Check: module crates must not import core ---
check_mod_to_core() {
  local paths=()
  [[ -d modules ]] && paths+=(modules)
  # shellcheck disable=SC2206
  local crates=(crates/exyonq-mod-*/)
  for c in "${crates[@]}"; do
    [[ -d "$c" ]] && paths+=("$c")
  done
  [[ ${#paths[@]} -gt 0 ]] || return 0

  local matches
  matches="$(rg -n --type rust 'use exyonq_core::|exyonq_core::(server|proxy|static_files|snapshot|reload|router)::' "${paths[@]}" 2>/dev/null || true)"
  if [[ -n "$matches" ]]; then
    log_violation "module crate imports exyonq-core internals"
    echo "$matches" >&2
  else
    log_pass "module crates do not import exyonq-core"
  fi
}

run_selftest() {
  echo "verify-phase0-kernel: --selftest ..."
  local tmp rc
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN

  mkdir -p "$tmp/core/src/server"
  printf '%s\n' 'pub struct HandlerTable;' >"$tmp/core/src/server/evil.rs"
  mkdir -p "$tmp/modules/probe/src"
  printf '%s\n' 'use exyonq_core::server::probe;' >"$tmp/modules/probe/src/lib.rs"

  rc=0
  EXYONQ_PHASE0_KERNEL_ROOT="$tmp" EXYONQ_PHASE0_KERNEL_STRICT=1 \
    bash "$_SCRIPT_DIR/verify-phase0-kernel.sh" >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 1 ]]; then
    echo "verify-phase0-kernel: --selftest FAILED: synthetic HandlerTable tree must exit 1 (got $rc)" >&2
    return 1
  fi

  rc=0
  EXYONQ_PHASE0_KERNEL_STRICT=1 bash "$_SCRIPT_DIR/verify-phase0-kernel.sh" >/dev/null 2>&1 || rc=$?
  if [[ "$rc" -ne 0 ]]; then
    echo "verify-phase0-kernel: --selftest FAILED: clean repo must exit 0 under STRICT (got $rc)" >&2
    return 1
  fi

  echo "verify-phase0-kernel: --selftest OK"
  return 0
}

write_report() {
  local out="$1"
  local ts verdict
  ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  if [[ "$VIOLATIONS" -gt 0 ]]; then
    verdict="FAIL"
  elif [[ "$WARNINGS" -gt 0 ]]; then
    verdict="PASS-WITH-WARNINGS"
  else
    verdict="PASS"
  fi

  mkdir -p "$(dirname "$out")"
  {
    echo "# Kernel Boundary Lint Report"
    echo ""
    echo "**Generated:** $ts"
    echo "**Script:** \`scripts/architecture/verify-phase0-kernel.sh\`"
    echo "**Contract:** \`study/reports/11-phase0-kernel-freeze-signoff.md\`"
    echo "**Verdict:** \`$verdict\`"
    echo ""
    echo "| Metric | Count |"
    echo "|--------|-------|"
    echo "| Violations | $VIOLATIONS |"
    echo "| Warnings | $WARNINGS |"
    echo "| Info | $INFOS |"
    echo ""
    echo "## Findings"
    echo ""
    local entry
    for entry in "${REPORT_LINES[@]}"; do
      if [[ "$entry" == VIOLATION:* ]]; then
        echo "- **${entry}**"
      elif [[ "$entry" == WARNING:* ]]; then
        echo "- ⚠ ${entry#WARNING: }"
      elif [[ "$entry" == INFO:* ]]; then
        echo "- ℹ ${entry#INFO: }"
      elif [[ "$entry" == PASS:* ]]; then
        echo "- ✓ ${entry#PASS: }"
      elif [[ "$entry" == ALLOWLIST:* ]]; then
        echo "- \`${entry#ALLOWLIST: }\` (allowlisted)"
      fi
    done
    echo ""
    echo "## Allowlist"
    echo ""
    echo "Temporary core→mod composition allowlist: \`scripts/architecture/phase0-core-mod-allowlist.txt\`"
    echo ""
    echo "## Usage"
    echo ""
    echo '```bash'
    echo "bash scripts/architecture/verify-phase0-kernel.sh"
    echo "EXYONQ_PHASE0_KERNEL_STRICT=1 bash scripts/architecture/verify-phase0-kernel.sh"
    echo "bash scripts/verify-oss-boundaries.sh   # includes Phase 0 kernel lint"
    echo '```'
    echo ""
    echo "_PR-0 tooling only. No core/modules logic changes._"
  } >"$out"
  echo "verify-phase0-kernel: report → $out"
}

main() {
  if [[ "$SELFTEST" == "1" ]]; then
    run_selftest
    exit $?
  fi

  echo "verify-phase0-kernel: scanning root=$ROOT strict=$STRICT"

  check_mod_to_core
  check_core_to_mod
  check_handler_table
  check_fastcgi_runtime
  check_forbidden_hotpath
  check_dispatch_vocabulary

  echo "verify-phase0-kernel: summary violations=$VIOLATIONS warnings=$WARNINGS info=$INFOS"

  if [[ -n "$REPORT_PATH" ]]; then
    write_report "$REPORT_PATH"
  fi

  if [[ "$VIOLATIONS" -gt 0 ]]; then
    if [[ "$STRICT" == "1" ]]; then
      echo "verify-phase0-kernel: STRICT — failing" >&2
      exit 1
    fi
    echo "verify-phase0-kernel: violations present (warn-only; set EXYONQ_PHASE0_KERNEL_STRICT=1)" >&2
    exit 0
  fi

  if [[ "$WARNINGS" -gt 0 && "$STRICT" == "1" ]]; then
    echo "verify-phase0-kernel: STRICT — warnings only (allowlisted drift documented)" >&2
  fi

  echo "verify-phase0-kernel: OK"
  exit 0
}

main
