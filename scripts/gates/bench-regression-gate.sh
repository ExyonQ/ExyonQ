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
NORMALIZE_TO="${BENCH_NORMALIZE_TO:-}"
REQUIRE_SERVER="${BENCH_REQUIRE_SERVER:-}"

usage() {
    cat <<'EOF'
Usage: bench-regression-gate.sh --baseline report.json --candidate report.json
                                [--max-rps-drop-pct 10] [--max-p99-rise-pct 15]
                                [--normalize-to nginx] [--require-server ols]

Exit 0 only when every candidate cell stays within both tolerances.
With --normalize-to, RPS and p99 are compared as ratios to that server
measured in the same run, cancelling host-wide shifts between runs.
With --require-server, that server must be present for every payload in both
reports; a missing required cell fails closed.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --baseline) BASELINE="${2:?}"; shift 2 ;;
        --candidate) CANDIDATE="${2:?}"; shift 2 ;;
        --max-rps-drop-pct) MAX_RPS_DROP="${2:?}"; shift 2 ;;
        --max-p99-rise-pct) MAX_P99_RISE="${2:?}"; shift 2 ;;
        --normalize-to) NORMALIZE_TO="${2:?}"; shift 2 ;;
        --require-server) REQUIRE_SERVER="${2:?}"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

[[ -n "$BASELINE" && -n "$CANDIDATE" ]] || { usage >&2; exit 2; }
[[ -f "$BASELINE" ]] || { echo "ERROR: missing baseline: $BASELINE" >&2; exit 2; }
[[ -f "$CANDIDATE" ]] || { echo "ERROR: missing candidate: $CANDIDATE" >&2; exit 2; }

python3 - "$BASELINE" "$CANDIDATE" "$MAX_RPS_DROP" "$MAX_P99_RISE" "$NORMALIZE_TO" "$REQUIRE_SERVER" <<'PY'
import json
import sys

baseline_path, candidate_path, max_drop, max_rise, normalize_to, require_server = sys.argv[1:7]
max_drop = float(max_drop)
max_rise = float(max_rise)
baseline = json.load(open(baseline_path, encoding="utf-8"))
candidate = json.load(open(candidate_path, encoding="utf-8"))

if baseline.get("schema") != 1 or candidate.get("schema") != 1:
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL reason=unsupported_report_schema")

if require_server:
    missing = []
    for label, report in (("baseline", baseline), ("candidate", candidate)):
        for payload, servers in report.get("results", {}).items():
            if require_server not in servers:
                missing.append(f"{payload}/{require_server}: missing in {label}")
    if missing:
        for failure in missing:
            print(f"REQUIRED: {failure}")
        raise SystemExit(
            f"BENCH_REGRESSION_GATE=FAIL reason=required_server_missing:{require_server}"
        )


def ratio(value, reference):
    value = float(value)
    reference = float(reference)
    if reference <= 0:
        raise ValueError("non-positive normalization reference")
    return value / reference


failures = []
checked = 0
mode = f"normalized-to-{normalize_to}" if normalize_to else "absolute"
for payload, servers in baseline["results"].items():
    ref_base = None
    ref_cand = None
    if normalize_to:
        ref_base = servers.get(normalize_to)
        ref_cand = candidate.get("results", {}).get(payload, {}).get(normalize_to)
        if ref_base is None or ref_cand is None:
            failures.append(f"{payload}: missing normalization reference {normalize_to}")
            continue
    for server, expected in servers.items():
        actual = candidate.get("results", {}).get(payload, {}).get(server)
        if actual is None:
            failures.append(f"{payload}/{server}: missing in candidate")
            continue
        checked += 1
        try:
            if normalize_to:
                base_rps = ratio(expected["rps"]["median"], ref_base["rps"]["median"])
                cand_rps = ratio(actual["rps"]["median"], ref_cand["rps"]["median"])
                base_p99 = ratio(expected["p99_ms"]["median"], ref_base["p99_ms"]["median"])
                cand_p99 = ratio(actual["p99_ms"]["median"], ref_cand["p99_ms"]["median"])
            else:
                base_rps = float(expected["rps"]["median"])
                cand_rps = float(actual["rps"]["median"])
                base_p99 = float(expected["p99_ms"]["median"])
                cand_p99 = float(actual["p99_ms"]["median"])
        except (KeyError, TypeError, ValueError) as error:
            failures.append(f"{payload}/{server}: invalid metric: {error}")
            continue
        drop = (base_rps - cand_rps) / base_rps * 100.0 if base_rps > 0 else 100.0
        rise = (cand_p99 - base_p99) / base_p99 * 100.0 if base_p99 > 0 else 100.0
        status = "PASS"
        if drop > max_drop:
            status = "FAIL"
            failures.append(
                f"{payload}/{server}: rps drop {drop:.1f}% > {max_drop:.1f}% "
                f"({base_rps:.4f} -> {cand_rps:.4f})"
            )
        if rise > max_rise:
            status = "FAIL"
            failures.append(
                f"{payload}/{server}: p99 rise {rise:.1f}% > {max_rise:.1f}% "
                f"({base_p99:.4f} -> {cand_p99:.4f})"
            )
        print(
            f"{status} {payload}/{server} mode={mode} rps={cand_rps:.4f} "
            f"(base {base_rps:.4f}, drop {drop:.1f}%) "
            f"p99={cand_p99:.4f} (base {base_p99:.4f}, rise {rise:.1f}%)"
        )

if checked == 0:
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL reason=no_overlapping_cells")
if failures:
    for failure in failures:
        print(f"REGRESSION: {failure}")
    raise SystemExit("BENCH_REGRESSION_GATE=FAIL")
print("BENCH_REGRESSION_GATE=PASS")
PY
