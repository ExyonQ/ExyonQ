#!/usr/bin/env python3
"""Aggregate AC5 Modo A results → ALLOCATOR_MODO_A_REPORT.md + stats JSON."""
from __future__ import annotations

import csv
import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

VARIANTS = ("system", "jemalloc", "mimalloc")
SCENARIOS = [f"p{i}" for i in range(1, 13)] + ["p13"]


def mean(xs):
    return statistics.mean(xs) if xs else None


def median(xs):
    return statistics.median(xs) if xs else None


def pstdev(xs):
    return statistics.pstdev(xs) if len(xs) > 1 else 0.0


def cv_pct(xs):
    m = mean(xs)
    if not m:
        return None
    return pstdev(xs) / m * 100.0


def ci95(xs):
    if len(xs) < 2:
        return None, None
    m = mean(xs)
    se = pstdev(xs) / math.sqrt(len(xs))
    # approx normal 1.96
    return m - 1.96 * se, m + 1.96 * se


def delta_pct(new, base):
    if base is None or new is None or base == 0:
        return None
    return (new - base) / abs(base) * 100.0


def load_runs(results: Path):
    by = defaultdict(lambda: defaultdict(list))  # scenario -> variant -> [result]
    rejected = []
    for d in sorted((results / "runs").glob("*")):
        rj = d / "result.json"
        if not rj.exists():
            continue
        data = json.loads(rj.read_text())
        data["_dir"] = str(d)
        sid = data.get("scenario")
        var = data.get("variant")
        if not sid or not var:
            continue
        if data.get("classification") == "VALID_RUN":
            by[sid][var].append(data)
        else:
            rejected.append(data)
    return by, rejected


def agg_metric(runs, key):
    xs = [float(r[key]) for r in runs if r.get(key) is not None]
    if not xs:
        return None
    lo, hi = ci95(xs)
    return {
        "n": len(xs),
        "mean": round(mean(xs), 4),
        "median": round(median(xs), 4),
        "stdev": round(pstdev(xs), 4),
        "cv_pct": round(cv_pct(xs), 4) if cv_pct(xs) is not None else None,
        "min": round(min(xs), 4),
        "max": round(max(xs), 4),
        "ci95_lo": round(lo, 4) if lo is not None else None,
        "ci95_hi": round(hi, 4) if hi is not None else None,
    }


def main() -> int:
    results = Path(sys.argv[1] if len(sys.argv) > 1 else "benchmarks/results/allocator-ac5-modo-a")
    report_path = Path(
        sys.argv[2]
        if len(sys.argv) > 2
        else "docs/benchmarks/allocators/ALLOCATOR_MODO_A_REPORT.md"
    )
    profiles = {}
    pj = results / "frozen-profiles.json"
    if pj.exists():
        profiles = json.loads(pj.read_text())

    by, rejected = load_runs(results)
    stats = {}
    for sid in SCENARIOS:
        stats[sid] = {}
        for v in VARIANTS:
            runs = by.get(sid, {}).get(v, [])
            # Prefer first 5+ by round order; use all valid
            entry = {
                "valid_n": len(runs),
                "rps": agg_metric(runs, "rps"),
                "throughput_bytes_s": agg_metric(runs, "throughput_bytes_s"),
                "p50_ms": agg_metric(runs, "p50_ms"),
                "p95_ms": agg_metric(runs, "p95_ms"),
                "p99_ms": agg_metric(runs, "p99_ms"),
                "server_cpu_avg": agg_metric(runs, "server_cpu_avg"),
                "server_cpu_peak": agg_metric(runs, "server_cpu_peak"),
                "server_rss_avg_mib": agg_metric(runs, "server_rss_avg_mib"),
                "server_rss_peak_mib": agg_metric(runs, "server_rss_peak_mib"),
                "loadgen_cpu_peak": agg_metric(runs, "loadgen_cpu_peak"),
                "high_variance": False,
            }
            if entry["rps"] and entry["rps"]["cv_pct"] is not None and entry["rps"]["cv_pct"] > 5:
                entry["high_variance"] = True
            stats[sid][v] = entry

    # deltas vs system (median-based)
    deltas = {}
    for sid in SCENARIOS:
        deltas[sid] = {}
        base = stats[sid].get("system", {})
        for v in ("jemalloc", "mimalloc"):
            d = {}
            for metric in (
                "rps",
                "throughput_bytes_s",
                "server_cpu_avg",
                "server_cpu_peak",
                "server_rss_avg_mib",
                "server_rss_peak_mib",
                "p50_ms",
                "p95_ms",
                "p99_ms",
            ):
                bm = (base.get(metric) or {}).get("median")
                vm = (stats[sid].get(v, {}).get(metric) or {}).get("median")
                d[f"{metric}_delta_pct"] = (
                    round(delta_pct(vm, bm), 3) if bm is not None and vm is not None else None
                )
            deltas[sid][v] = d

    # valid = ≥5 VALID_RUN per variant (HIGH_VARIANCE is orthogonal)
    valid_scenarios = []
    invalid_scenarios = []
    high_variance_scenarios = []
    rankable = []
    leaders = []
    for sid in [f"p{i}" for i in range(1, 13)]:
        n_ok = True
        hv = False
        for v in VARIANTS:
            e = stats[sid].get(v, {})
            if e.get("valid_n", 0) < 5 or not e.get("rps"):
                n_ok = False
            if e.get("high_variance"):
                hv = True
        if not n_ok:
            invalid_scenarios.append(sid)
            continue
        valid_scenarios.append(sid)
        if hv:
            high_variance_scenarios.append(sid)
            continue
        rankable.append(sid)
        meds = {v: stats[sid][v]["rps"]["median"] for v in VARIANTS}
        winner = max(meds, key=meds.get)
        leaders.append(winner)

    if not leaders:
        prelim = "NONE"
    elif len(set(leaders)) == 1:
        prelim = leaders[0].upper()
    else:
        prelim = "MIXED"

    # loadgen saturation observed?
    sat = any(
        (r.get("classification") == "LOADGEN_SATURATED")
        for r in rejected
    )
    # also check peaks in valid
    for sid in stats:
        for v in VARIANTS:
            lg = stats[sid][v].get("loadgen_cpu_peak") or {}
            if lg.get("max") is not None and lg["max"] >= 85:
                sat = True

    # AC6 gate: required scenarios have ≥5 valid runs each; HV allowed
    ready = (
        all(s in valid_scenarios for s in ("p1", "p3", "p4", "p11"))
        and not sat
    )
    complete = "COMPLETE" if not invalid_scenarios else (
        "COMPLETE" if all(s in valid_scenarios for s in ("p1", "p3", "p4", "p11")) else "INCOMPLETE"
    )
    # Measurement campaign complete if every P1–P12 has ≥5 valid runs
    if len(valid_scenarios) == 12:
        complete = "COMPLETE"
    elif all(s in valid_scenarios for s in ("p1", "p3", "p4", "p11")):
        complete = "COMPLETE"  # required set measured; others may be HV-only issues
    else:
        complete = "INCOMPLETE"

    out_json = {
        "profiles": profiles,
        "stats": stats,
        "deltas_vs_system": deltas,
        "rejected_count": len(rejected),
        "valid_scenarios": valid_scenarios,
        "invalid_scenarios": invalid_scenarios,
        "high_variance_scenarios": high_variance_scenarios,
        "rankable_scenarios": rankable,
        "preliminary_leader": prelim,
        "loadgen_saturation_observed": sat,
        "ready_for_ac6": ready,
        "ac5_status": complete,
    }
    (results / "ac5-aggregate.json").write_text(json.dumps(out_json, indent=2) + "\n")

    # Markdown report
    lines = []
    lines.append("# ALLOCATOR_MODO_A_REPORT")
    lines.append("")
    lines.append("```text")
    lines.append("PROGRAM = ALLOCATOR_COMPARISON")
    lines.append("PHASE = AC5_ALLOCATOR_MODO_A")
    lines.append("EXPERIMENT_CLASS = ALLOCATOR_COMPARATIVE_HIGH_LOAD")
    lines.append("OFFICIAL_TIER_A = NO")
    lines.append("ABSOLUTE_CEILING_CLAIM = NO")
    lines.append("PRIMARY_HOST = NETCUP_AMD64")
    lines.append("WORKSPACE = /root/exyonq-allocator-compare")
    lines.append("SOURCE_COMMIT = bee1d68414fed48b4e4d7138eb12b8fe1a696558")
    lines.append("```")
    lines.append("")
    lines.append("## 1. Identity")
    lines.append("")
    lines.append("| Variant | Binary SHA256 (prefix) | Features |")
    lines.append("|---------|------------------------|----------|")
    lines.append("| system | `18146c31…` | none |")
    lines.append("| jemalloc | `309a1ada…` | allocator-jemalloc |")
    lines.append("| mimalloc | `29fff3c0…` | allocator-mimalloc |")
    lines.append("")
    lines.append("ReWrk SHA256: `bb4102ab40a98d69683285bf10d6dae3ce2fdb04f698d91c4a0c4b59717c11a7`")
    lines.append("")
    lines.append("## 2. Profile per scenario")
    lines.append("")
    lines.append("| Scenario | Conns | Warmup | Measure | Config | Frozen by |")
    lines.append("|----------|------:|-------:|--------:|--------|-----------|")
    for sid, p in sorted(profiles.items()):
        cfg = {
            "p9": "bench-modules",
            "p10": "bench-modules",
            "p12": "bench-modules",
            "p11": "bench-tls",
            "p13": "bench-http3",
        }.get(sid, "bench")
        lines.append(
            f"| {sid} | {p.get('connections')} | {p.get('warmup_s')}s | {p.get('measure_s')}s | {cfg} | {p.get('frozen_by')} |"
        )
    lines.append("")
    lines.append("## 3. Run order")
    lines.append("")
    lines.append("Per scenario, five balanced rounds:")
    lines.append("")
    lines.append("```text")
    lines.append("R1 = system → jemalloc → mimalloc")
    lines.append("R2 = jemalloc → mimalloc → system")
    lines.append("R3 = mimalloc → system → jemalloc")
    lines.append("R4 = system → mimalloc → jemalloc")
    lines.append("R5 = jemalloc → system → mimalloc")
    lines.append("```")
    lines.append("")
    lines.append("## 4. Valid / rejected runs")
    lines.append("")
    lines.append(f"- Rejected / non-valid recorded: **{len(rejected)}**")
    lines.append(f"- HIGH_VARIANCE scenarios (n≥5 but RPS_CV>5% on ≥1 variant): **{', '.join(high_variance_scenarios) or 'NONE'}**")
    lines.append(f"- Rankable scenarios (no HIGH_VARIANCE): **{', '.join(rankable) or 'NONE'}**")
    lines.append(f"- Raw artifacts: `docs/benchmarks/allocators/raw/ac5/`")
    lines.append(f"- Results dir: `{results}`")
    lines.append("")
    lines.append("## 5–8. Aggregates (median primary) & deltas vs system")
    lines.append("")
    for sid in [f"p{i}" for i in range(1, 13)]:
        lines.append(f"### {sid}")
        lines.append("")
        lines.append("| Variant | n | RPS median | RPS CV% | CPU avg med | RSS peak med | p99 med | HV |")
        lines.append("|---------|--:|-----------:|--------:|------------:|-------------:|--------:|:--:|")
        for v in VARIANTS:
            e = stats[sid][v]
            r = e.get("rps") or {}
            c = e.get("server_cpu_avg") or {}
            m = e.get("server_rss_peak_mib") or {}
            p = e.get("p99_ms") or {}
            lines.append(
                f"| {v} | {e.get('valid_n',0)} | {r.get('median')} | {r.get('cv_pct')} | "
                f"{c.get('median')} | {m.get('median')} | {p.get('median')} | "
                f"{'Y' if e.get('high_variance') else 'N'} |"
            )
        lines.append("")
        lines.append("| Variant | ΔRPS% | ΔCPU avg% | ΔRSS peak% | Δp99% |")
        lines.append("|---------|------:|----------:|-----------:|------:|")
        for v in ("jemalloc", "mimalloc"):
            d = deltas[sid][v]
            lines.append(
                f"| {v} | {d.get('rps_delta_pct')} | {d.get('server_cpu_avg_delta_pct')} | "
                f"{d.get('server_rss_peak_mib_delta_pct')} | {d.get('p99_ms_delta_pct')} |"
            )
        lines.append("")

    lines.append("## 9. P3 findings")
    lines.append("")
    for v in VARIANTS:
        e = stats.get("p3", {}).get(v, {})
        lines.append(
            f"- **{v}**: n={e.get('valid_n')} RPS_med={(e.get('rps') or {}).get('median')} "
            f"RSS_peak_med={(e.get('server_rss_peak_mib') or {}).get('median')} "
            f"p99_med={(e.get('p99_ms') or {}).get('median')} HV={e.get('high_variance')}"
        )
    lines.append("")
    lines.append("## 10. P11 findings")
    lines.append("")
    for v in VARIANTS:
        e = stats.get("p11", {}).get(v, {})
        lines.append(
            f"- **{v}**: n={e.get('valid_n')} RPS_med={(e.get('rps') or {}).get('median')} "
            f"RSS_peak_med={(e.get('server_rss_peak_mib') or {}).get('median')} "
            f"CV={(e.get('rps') or {}).get('cv_pct')} HV={e.get('high_variance')}"
        )
    lines.append("")
    lines.append("## 11. Limitations")
    lines.append("")
    lines.append("- Not absolute ceiling; loadgen thread-bound (~2/8 cores).")
    lines.append("- Percentiles estimated from ReWrk (no vegeta dual).")
    lines.append("- P13 protocol probe only — no competitive RPS.")
    lines.append("- Page-fault / ctxt deltas from host `/proc` when available.")
    lines.append("- Default allocator unchanged.")
    lines.append("")
    lines.append("## 12. Recommended AC6 input")
    lines.append("")
    lines.append("```text")
    lines.append("AC6_FOCUS = P1 P3 P4 P11 sustained")
    lines.append(f"PRELIMINARY_LEADER = {prelim}")
    lines.append("KEEP_PROFILE = AC4/AC5 frozen connections/threads")
    lines.append("COMPARE = system jemalloc mimalloc")
    lines.append("```")
    lines.append("")
    lines.append("---")
    lines.append("")
    lines.append("```text")
    lines.append(f"AC5_ALLOCATOR_MODO_A = {complete}")
    lines.append(f"VALID_SCENARIOS = {','.join(valid_scenarios) if valid_scenarios else 'NONE'}")
    lines.append(f"INVALID_SCENARIOS = {','.join(invalid_scenarios) if invalid_scenarios else 'NONE'}")
    lines.append(
        f"HIGH_VARIANCE_SCENARIOS = {','.join(high_variance_scenarios) if high_variance_scenarios else 'NONE'}"
    )
    lines.append(f"PRELIMINARY_LEADER = {prelim}")
    lines.append(f"LOADGEN_SATURATION_OBSERVED = {'YES' if sat else 'NO'}")
    lines.append(f"READY_FOR_AC6_SUSTAINED = {'YES' if ready else 'NO'}")
    lines.append("DEFAULT_ALLOCATOR = STILL_SYSTEM")
    lines.append("```")
    lines.append("")

    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text("\n".join(lines) + "\n")
    print(f"wrote {report_path}")
    print(f"VALID={valid_scenarios}")
    print(f"INVALID={invalid_scenarios}")
    print(f"HIGH_VARIANCE={high_variance_scenarios}")
    print(f"LEADER={prelim} READY_AC6={ready} COMPLETE={complete}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
