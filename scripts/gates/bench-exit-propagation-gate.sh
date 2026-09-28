#!/usr/bin/env bash
# Regression gate for ZF-003: bench scripts must not soft-pass missing work.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:---tree}"
TREE="${2:-$ROOT}"

fail=0

if [[ "$MODE" == "--selftest" ]]; then
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$tmp/benchmarks/scenarios/perf"
  cat >"$tmp/benchmarks/scenarios/perf/collect-loadgen.sh" <<'EOF'
#!/usr/bin/env bash
if [[ -z "${cid:-}" ]]; then
  exit 0
fi
EOF
  cat >"$tmp/benchmarks/scenarios/perf/collect-resources.sh" <<'EOF'
#!/usr/bin/env bash
exit 1
EOF
  cat >"$tmp/benchmarks/scenarios/perf/profile-hotpaths.sh" <<'EOF'
#!/usr/bin/env bash
if [[ "$(uname -s)" != "Linux" ]]; then
  exit 0
fi
EOF
  cat >"$tmp/benchmarks/scenarios/perf/strace-p1-syscalls.sh" <<'EOF'
#!/usr/bin/env bash
exit 2
EOF
  cat >"$tmp/benchmarks/scenarios/perf/adr-025-protector-matrix.sh" <<'EOF'
#!/usr/bin/env bash
if [[ "${PROTECTOR_SELECTIVE:-0}" == "1" ]]; then
  exit 0
fi
EOF
  cat >"$tmp/benchmarks/scenarios/perf/adr-025-protector-orchestrator.sh" <<'EOF'
#!/usr/bin/env bash
run_host a b c d || true
EOF
  cat >"$tmp/benchmarks/scenarios/perf/run-perf-inner.sh" <<'EOF'
#!/usr/bin/env bash
wait "$cpid" || echo "  WARN: collect-resources failed"
EOF
  cat >"$tmp/benchmarks/scenarios/perf/run-peak-scan-inner.sh" <<'EOF'
#!/usr/bin/env bash
wait "$cpid" || echo "  WARN: collect-resources failed for peak"
EOF
  cat >"$tmp/benchmarks/scenarios/perf/run-perf-fixed-inner.sh" <<'EOF'
#!/usr/bin/env bash
true
EOF
  cat >"$tmp/benchmarks/scenarios/perf/refresh-rivals-resources.sh" <<'EOF'
#!/usr/bin/env bash
true
EOF
  # strace without empty-capture check should fail gate
  cat >"$tmp/benchmarks/scenarios/perf/strace-p1-syscalls.sh" <<'EOF'
#!/usr/bin/env bash
if [[ "$(uname -s)" != "Linux" ]]; then
  exit 2
fi
echo Wrote
EOF
  if bash "$0" --tree "$tmp"; then
    echo "SELFTEST_FAIL: expected FAIL on poisoned tree"
    exit 1
  fi
  echo "BENCH_EXIT_PROPAGATION_GATE_SELFTEST = PASS"
  exit 0
fi

require_file() {
  local f="$1"
  if [[ ! -f "$TREE/$f" ]]; then
    echo "MISSING $f"
    fail=1
    return 1
  fi
  return 0
}

# collect-loadgen: missing peer must exit 1, not 0
if require_file benchmarks/scenarios/perf/collect-loadgen.sh; then
  if ! rg -n 'bench-runner container not found' "$TREE/benchmarks/scenarios/perf/collect-loadgen.sh" >/dev/null; then
    echo "FAIL collect-loadgen missing explicit not-found error"
    fail=1
  fi
  # After the error echo, next exit must be 1 (not 0)
  if awk '
    /bench-runner container not found/ { hit=1; next }
    hit && /exit[[:space:]]+0/ { bad=1; exit }
    hit && /exit[[:space:]]+1/ { ok=1; exit }
    END { exit (ok && !bad) ? 0 : 1 }
  ' "$TREE/benchmarks/scenarios/perf/collect-loadgen.sh"; then
    echo "PASS collect-loadgen missing peer → exit 1"
  else
    echo "FAIL collect-loadgen missing peer still exits 0 or lacks exit 1"
    fail=1
  fi
fi

if require_file benchmarks/scenarios/perf/collect-resources.sh; then
  if awk '
    /container not found for/ { hit=1; next }
    hit && /exit[[:space:]]+0/ { bad=1; exit }
    hit && /exit[[:space:]]+1/ { ok=1; exit }
    END { exit (ok && !bad) ? 0 : 1 }
  ' "$TREE/benchmarks/scenarios/perf/collect-resources.sh"; then
    echo "PASS collect-resources missing peer → exit 1"
  else
    echo "FAIL collect-resources missing peer still exits 0 or lacks exit 1"
    fail=1
  fi
fi

if require_file benchmarks/scenarios/perf/profile-hotpaths.sh; then
  if awk '
    /!= "Linux"/ { hit=1; next }
    hit && /exit[[:space:]]+0/ { bad=1; exit }
    hit && /exit[[:space:]]+2/ { ok=1; exit }
    END { exit (ok && !bad) ? 0 : 1 }
  ' "$TREE/benchmarks/scenarios/perf/profile-hotpaths.sh"; then
    echo "PASS profile-hotpaths non-Linux → exit 2"
  else
    echo "FAIL profile-hotpaths non-Linux soft-pass"
    fail=1
  fi
fi

if require_file benchmarks/scenarios/perf/strace-p1-syscalls.sh; then
  if awk '
    /!= "Linux"/ { hit=1; next }
    hit && /exit[[:space:]]+0/ { bad=1; exit }
    hit && /exit[[:space:]]+2/ { ok=1; exit }
    END { exit (ok && !bad) ? 0 : 1 }
  ' "$TREE/benchmarks/scenarios/perf/strace-p1-syscalls.sh"; then
    echo "PASS strace-p1 non-Linux → exit 2"
  else
    echo "FAIL strace-p1 non-Linux soft-pass"
    fail=1
  fi
fi

if require_file benchmarks/scenarios/perf/adr-025-protector-matrix.sh; then
  if rg -n 'PROTECTOR_SELECTIVE' -A3 "$TREE/benchmarks/scenarios/perf/adr-025-protector-matrix.sh" | rg -q 'exit 0'; then
    echo "FAIL PROTECTOR_SELECTIVE still forces exit 0"
    fail=1
  else
    echo "PASS PROTECTOR_SELECTIVE does not force exit 0"
  fi
fi

if require_file benchmarks/scenarios/perf/adr-025-protector-orchestrator.sh; then
  if rg -n 'run_host .* \|\| true' "$TREE/benchmarks/scenarios/perf/adr-025-protector-orchestrator.sh" >/dev/null; then
    echo "FAIL orchestrator masks run_host with || true"
    fail=1
  else
    echo "PASS orchestrator does not mask run_host with || true"
  fi
fi

if require_file benchmarks/scenarios/perf/run-perf-inner.sh; then
  if rg -n 'WARN: collect-(resources|loadgen) failed' "$TREE/benchmarks/scenarios/perf/run-perf-inner.sh" >/dev/null; then
    echo "FAIL run-perf-inner still soft-warns collect failures"
    fail=1
  else
    echo "PASS run-perf-inner hard-fails collect errors"
  fi
fi

for soft_warn_file in \
  benchmarks/scenarios/perf/run-peak-scan-inner.sh \
  benchmarks/scenarios/perf/run-perf-fixed-inner.sh \
  benchmarks/scenarios/perf/refresh-rivals-resources.sh
do
  if require_file "$soft_warn_file"; then
    if rg -n 'WARN: collect-(resources|loadgen) failed' "$TREE/$soft_warn_file" >/dev/null; then
      echo "FAIL $soft_warn_file still soft-warns collect failures"
      fail=1
    else
      echo "PASS $soft_warn_file hard-fails collect errors"
    fi
  fi
done

if require_file benchmarks/scenarios/perf/strace-p1-syscalls.sh; then
  if ! rg -n 'empty or missing capture' "$TREE/benchmarks/scenarios/perf/strace-p1-syscalls.sh" >/dev/null; then
    echo "FAIL strace-p1 lacks empty-capture hard-fail"
    fail=1
  else
    echo "PASS strace-p1 requires non-empty capture"
  fi
fi

if [[ "$fail" -ne 0 ]]; then
  echo "BENCH_EXIT_PROPAGATION_GATE = FAIL"
  exit 1
fi
echo "BENCH_EXIT_PROPAGATION_GATE = PASS"
exit 0
