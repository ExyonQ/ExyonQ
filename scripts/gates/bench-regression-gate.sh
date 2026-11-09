#!/usr/bin/env bash
# Q-12 performance anti-regression gate.
#
# Compares two reports produced by scripts/bench/run-http-bench.sh and
# fails when the candidate loses too much throughput or gains too much p99
# latency for any measured server/payload cell.
set -euo pipefail

BASELINE=""
CANDIDATE=""
MAX_RPS_DROP="${BENCH_MAX_RPS_DROP_PCT:-10}"
MAX_P99_RISE="${BENCH_MAX_P99_RISE_PCT:-15}"

usage() {
    cat <<'EOF'
Usage: bench-regression-gate.sh --baseline report.json --candidate report.json
                                [--max-rps-drop-pct 10] [--max-p99-rise-pct 15]

Exit 0 only when every candidate cell stays within both tolerances.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --baseline) BASELINE="${2:?}"; shift 2 ;;
        --candidate) CANDIDATE="${2:?}"; shift 2 ;;
        --max-rps-drop-pct) MAX_RPS_DROP="${2:?}"; shift 2 ;;
        --max-p99-rise-pct) MAX_P99_RISE="${2:?}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[[ -n "$BASELINE" && -n "$CANDIDATE" ]] || { usage >&2; exit 2; }
[[ -f "$BASELINE" ]] || { echo "ERROR: missing baseline: $BASELINE" >&2; exit 2; }
[[ -f "$CANDIDATE" ]] || { echo "ERROR: missing candidate: $CANDIDATE" >&2; exit 2; }

python3 - "$BASELINE" "$CANDIDATE" "$MAX_RPS_DROP" "$MAX_P99_RISE" <<'PY'
import json
import sys

baseline_path, candidate_path, max_drop, max_rise = sys.argv[1:5]
max_drop = float(max_drop)
max_rise = float(max_rise)
baseline = json.load(open(baseline_path, encoding="utf-8"))
candidate = json.load(open(candidate_path, encoding="utf-8"))

if baseline.get("schema") != 1 or candidate.get("schema") != 1:
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL reason=unsupported_report_schema")

failures = []
checked = 0
for payload, servers in baseline["results"].items():
    for server, expected in servers.items():
        actual = candidate.get("results", {}).get(payload, {}).get(server)
        if actual is None:
            failures.append(f"{payload}/{server}: missing in candidate")
            continue
        checked += 1
        base_rps = float(expected["rps"]["median"])
        cand_rps = float(actual["rps"]["median"])
        base_p99 = float(expected["p99_ms"]["median"])
        cand_p99 = float(actual["p99_ms"]["median"])
        drop = (base_rps - cand_rps) / base_rps * 100.0 if base_rps > 0 else 100.0
        rise = (cand_p99 - base_p99) / base_p99 * 100.0 if base_p99 > 0 else 100.0
        status = "PASS"
        if drop > max_drop:
            status = "FAIL"
            failures.append(
                f"{payload}/{server}: rps drop {drop:.1f}% > {max_drop:.1f}% "
                f"({base_rps:.1f} -> {cand_rps:.1f})"
            )
        if rise > max_rise:
            status = "FAIL"
            failures.append(
                f"{payload}/{server}: p99 rise {rise:.1f}% > {max_rise:.1f}% "
                f"({base_p99:.2f}ms -> {cand_p99:.2f}ms)"
            )
        print(
            f"{status} {payload}/{server} rps={cand_rps:.1f} (base {base_rps:.1f}, "
            f"drop {drop:.1f}%) p99={cand_p99:.2f}ms (base {base_p99:.2f}ms, rise {rise:.1f}%)"
        )

if checked == 0:
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL reason=no_overlapping_cells")
if failures:
    for failure in failures:
        print(f"REGRESSION: {failure}")
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL")
print("BENCH_REGRESSION_GATE=PASS")
PY
