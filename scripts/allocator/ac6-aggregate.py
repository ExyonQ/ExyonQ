#!/usr/bin/env python3
"""Aggregate AC6 sustained results → ALLOCATOR_SUSTAINED_REPORT.md"""
from __future__ import annotations

import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

VARIANTS = ("system", "jemalloc", "mimalloc")
SCENARIOS = ("p1", "p3", "p4", "p6", "p11")


def median(xs):
    return statistics.median(xs) if xs else None


def mean(xs):
    return statistics.mean(xs) if xs else None


def pstdev(xs):
    return statistics.pstdev(xs) if len(xs) > 1 else 0.0


def cv(xs):
    m = mean(xs)
    if not m:
        return None
    return pstdev(xs) / m * 100.0


def delta_pct(new, base):
    if base in (None, 0) or new is None:
        return None
    return (new - base) / abs(base) * 100.0


def load_runs(results: Path):
    by = defaultdict(lambda: defaultdict(list))
    rejected = []
    for d in sorted((results / "runs").glob("ac6-*")):
        sj = d / "summary.json"
        if not sj.exists():
            continue
        data = json.loads(sj.read_text())
        # parse tag ac6-{sid}-r{n}-{variant}-o{m}
        name = d.name
        parts = name.split("-")
        # ac6 p1 r1 system o1  OR ac6 p11 r1 jemalloc o2
        try:
            sid = parts[1]
            if parts[1].startswith("p") and parts[2].startswith("r"):
                sid = parts[1]
                rnd = int(parts[2][1:])
                # variant may be multi? no
                # find variant
                rest = "-".join(parts[3:])
                # form: system-o1 / jemalloc-o2
                if rest.endswith("-o1") or rest.endswith("-o2") or rest.endswith("-o3"):
                    variant = rest.rsplit("-", 1)[0]
                    order = int(rest.rsplit("-", 1)[1][1:])
                else:
                    continue
            else:
                continue
        except Exception:
            # fallback from identity
            ident = {}
            if (d / "identity.txt").exists():
                for line in (d / "identity.txt").read_text().splitlines():
                    if "=" in line:
                        k, v = line.split("=", 1)
                        ident[k] = v
            sid = ident.get("SCENARIO")
            variant = ident.get("ALLOCATOR_VARIANT")
            rnd = int(ident.get("RUN_INDEX") or 0)
            order = int(ident.get("RUN_ORDER") or 0)
        data["_dir"] = str(d)
        data["scenario"] = sid
        data["variant"] = variant
        data["round"] = rnd
        data["order"] = order
        if data.get("classification") == "VALID_RUN":
            by[sid][variant].append(data)
        else:
            rejected.append(data)
    return by, rejected


def agg_field(runs, key):
    xs = [float(r[key]) for r in runs if r.get(key) is not None]
    if not xs:
        return None
    return {
        "n": len(xs),
        "median": round(median(xs), 4),
        "mean": round(mean(xs), 4),
        "stdev": round(pstdev(xs), 4),
        "cv_pct": round(cv(xs), 4) if cv(xs) is not None else None,
        "min": round(min(xs), 4),
        "max": round(max(xs), 4),
    }


def sustained_verdict(runs, system_retained_med):
    """PASS / WARNING / FAIL for an allocator on a scenario (and later global)."""
    if len(runs) < 3:
        return "FAIL"
    mono = sum(1 for r in runs if r.get("memory_class") == "MONOTONIC_GROWTH")
    creep_n = sum(1 for r in runs if r.get("RAM_CREEP_SUSPECTED"))
    degrade = sum(1 for r in runs if r.get("degrade_rps") or r.get("degrade_p99"))
    retained = [float(r["RSS_RETAINED_AFTER_RECOVERY"]) for r in runs if r.get("RSS_RETAINED_AFTER_RECOVERY") is not None]
    ret_med = median(retained) if retained else None

    if mono >= 2:
        return "FAIL"
    if system_retained_med is not None and ret_med is not None:
        if system_retained_med >= 0 and ret_med > system_retained_med + abs(system_retained_med) * 0.5 + 1.0:
            # > system+50% retained (absolute guard +1MiB)
            if ret_med > max(system_retained_med * 1.5, system_retained_med + 5):
                return "FAIL"
        if ret_med > system_retained_med * 1.10 + 0.5 and system_retained_med >= 0:
            return "WARNING"
        if system_retained_med < 0 and ret_med > 5:
            return "WARNING"
    if creep_n >= 2:
        return "WARNING"
    if degrade >= 2:
        return "WARNING"
    return "PASS"


def creep_status(runs):
    n = sum(1 for r in runs if r.get("RAM_CREEP_SUSPECTED"))
    if n >= 2:
        return "CONFIRMED"
    if n == 1:
        return "INCONCLUSIVE"
    return "NOT_CONFIRMED"


def main() -> int:
    results = Path(sys.argv[1] if len(sys.argv) > 1 else "benchmarks/results/allocator-ac6-sustained")
    report = Path(
        sys.argv[2]
        if len(sys.argv) > 2
        else "docs/benchmarks/allocators/ALLOCATOR_SUSTAINED_REPORT.md"
    )
    profiles = {}
    if (results / "frozen-profiles.json").exists():
        profiles = json.loads((results / "frozen-profiles.json").read_text())

    by, rejected = load_runs(results)
    stats = {}
    for sid in SCENARIOS:
        stats[sid] = {}
        for v in VARIANTS:
            runs = by.get(sid, {}).get(v, [])
            stats[sid][v] = {
                "valid_n": len(runs),
                "rps": agg_field(runs, "achieved_rps_median"),
                "rps_window_cv": agg_field(runs, "rps_window_cv"),
                "p99_window_cv": agg_field(runs, "p99_window_cv"),
                "rss_mean": agg_field(runs, "RSS_LOAD_MEAN"),
                "rss_max": agg_field(runs, "RSS_LOAD_MAX"),
                "rss_retained": agg_field(runs, "RSS_RETAINED_AFTER_RECOVERY"),
                "recovery_ratio": agg_field(runs, "RSS_RECOVERY_RATIO"),
                "creep_suspected_n": sum(1 for r in runs if r.get("RAM_CREEP_SUSPECTED")),
                "degrade_n": sum(1 for r in runs if r.get("degrade_rps") or r.get("degrade_p99")),
                "memory_classes": [r.get("memory_class") for r in runs],
            }

    # per-scenario sustained grades
    grades = {}
    for sid in SCENARIOS:
        sys_ret = (stats[sid]["system"].get("rss_retained") or {}).get("median")
        grades[sid] = {}
        for v in VARIANTS:
            grades[sid][v] = sustained_verdict(by.get(sid, {}).get(v, []), sys_ret)

    def global_grade(v):
        # worst across mandatory scenarios (exclude p6 diagnostic from FAIL-only? include all)
        gs = [grades[s][v] for s in ("p1", "p3", "p4", "p11")]
        if "FAIL" in gs:
            return "FAIL"
        if "WARNING" in gs:
            return "WARNING"
        return "PASS"

    system_g = global_grade("system")
    jem_g = global_grade("jemalloc")
    mim_g = global_grade("mimalloc")

    p11_creep = {v: creep_status(by.get("p11", {}).get(v, [])) for v in VARIANTS}

    # sustained leader: among PASS, prefer lower retained then higher rps on p4/p11
    valid_sc = [s for s in SCENARIOS if all(stats[s][v]["valid_n"] >= 3 for v in VARIANTS)]
    invalid_sc = [s for s in SCENARIOS if s not in valid_sc]

    leaders = []
    for sid in ("p1", "p3", "p4", "p11"):
        if sid not in valid_sc:
            continue
        # skip if all WARNING/FAIL equally — pick lowest retained median among non-FAIL
        cands = [v for v in VARIANTS if grades[sid][v] != "FAIL"]
        if not cands:
            continue
        def key(v):
            ret = (stats[sid][v].get("rss_retained") or {}).get("median")
            rps = (stats[sid][v].get("rps") or {}).get("median") or 0
            # lower retained better; higher rps tie-break
            return ((ret if ret is not None else 1e9), -rps)
        leaders.append(min(cands, key=key))

    if not leaders:
        sust_leader = "NONE"
    elif len(set(leaders)) == 1:
        sust_leader = leaders[0].upper()
    else:
        sust_leader = "MIXED"

    complete = "COMPLETE" if all(s in valid_sc for s in ("p1", "p3", "p4", "p11")) else "INCOMPLETE"
    ready = complete == "COMPLETE"

    # deltas vs system
    deltas = {}
    for sid in SCENARIOS:
        deltas[sid] = {}
        base = stats[sid]["system"]
        for v in ("jemalloc", "mimalloc"):
            d = {}
            for metric in ("rps", "rss_mean", "rss_max", "rss_retained"):
                bm = (base.get(metric) or {}).get("median")
                vm = (stats[sid][v].get(metric) or {}).get("median")
                d[f"{metric}_delta_pct"] = (
                    round(delta_pct(vm, bm), 3) if bm is not None and vm is not None else None
                )
            deltas[sid][v] = d

    out = {
        "profiles": profiles,
        "stats": stats,
        "grades": grades,
        "deltas_vs_system": deltas,
        "rejected_count": len(rejected),
        "valid_scenarios": valid_sc,
        "invalid_scenarios": invalid_sc,
        "system_sustained": system_g,
        "jemalloc_sustained": jem_g,
        "mimalloc_sustained": mim_g,
        "p11_ram_creep": p11_creep,
        "sustained_leader": sust_leader,
        "ready_for_ac7": ready,
        "ac6_status": complete,
    }
    (results / "ac6-aggregate.json").write_text(json.dumps(out, indent=2) + "\n")

    lines = []
    lines += [
        "# ALLOCATOR_SUSTAINED_REPORT",
        "",
        "```text",
        "PROGRAM = ALLOCATOR_COMPARISON",
        "PHASE = AC6_SUSTAINED_MEMORY_AND_STABILITY",
        "EXPERIMENT_CLASS = ALLOCATOR_COMPARATIVE_HIGH_LOAD",
        "PRIMARY_HOST = NETCUP_AMD64",
        "WORKSPACE = /root/exyonq-allocator-compare",
        "SOURCE_COMMIT = bee1d68414fed48b4e4d7138eb12b8fe1a696558",
        "```",
        "",
        "## 1. Frozen profile per scenario",
        "",
        "| Scenario | Target RPS | Conns | Warmup | Measure | Recovery | Window | Config |",
        "|----------|-----------:|------:|-------:|--------:|---------:|-------:|--------|",
    ]
    for sid in SCENARIOS:
        p = profiles.get(sid, {})
        lines.append(
            f"| {sid} | {p.get('target_rps')} | {p.get('connections')} | {p.get('warmup_s')}s | "
            f"{p.get('measure_s')}s | {p.get('recovery_s')}s | {p.get('window_s')}s | {p.get('config')} |"
        )
    lines += [
        "",
        "Common: `rewrk_threads=2`, `server_workers=4`, `accept_workers=4`, rivals stopped,",
        "open-loop ReWrk (no native fixed-RPS); connections calibrated toward ~70% of AC5 min median.",
        "",
        "## 2. Run order",
        "",
        "```text",
        "R1 = system → jemalloc → mimalloc",
        "R2 = jemalloc → mimalloc → system",
        "R3 = mimalloc → system → jemalloc",
        "```",
        "",
        f"## 3. Valid / rejected",
        "",
        f"- Rejected: **{len(rejected)}**",
        f"- Valid scenarios (≥3/variant): {', '.join(valid_sc) or 'NONE'}",
        f"- Invalid: {', '.join(invalid_sc) or 'NONE'}",
        f"- Raw: `docs/benchmarks/allocators/raw/ac6/`",
        "",
        "## 4–6. Memory & recovery (median across valid runs)",
        "",
    ]
    for sid in SCENARIOS:
        lines.append(f"### {sid}")
        lines.append("")
        lines.append(
            "| Variant | n | RPS med | RPS win CV med | RSS mean | RSS max | RSS retained 180s | Recov ratio | Creep sus n | Grade |"
        )
        lines.append(
            "|---------|--:|--------:|---------------:|---------:|--------:|------------------:|------------:|------------:|:-----:|"
        )
        for v in VARIANTS:
            e = stats[sid][v]
            lines.append(
                f"| {v} | {e['valid_n']} | {(e.get('rps') or {}).get('median')} | "
                f"{(e.get('rps_window_cv') or {}).get('median')} | "
                f"{(e.get('rss_mean') or {}).get('median')} | {(e.get('rss_max') or {}).get('median')} | "
                f"{(e.get('rss_retained') or {}).get('median')} | {(e.get('recovery_ratio') or {}).get('median')} | "
                f"{e.get('creep_suspected_n')} | {grades[sid][v]} |"
            )
        lines.append("")
        lines.append("| Variant | ΔRPS% | ΔRSS mean% | ΔRSS max% | ΔRSS retained% |")
        lines.append("|---------|------:|-----------:|----------:|---------------:|")
        for v in ("jemalloc", "mimalloc"):
            d = deltas[sid][v]
            lines.append(
                f"| {v} | {d.get('rps_delta_pct')} | {d.get('rss_mean_delta_pct')} | "
                f"{d.get('rss_max_delta_pct')} | {d.get('rss_retained_delta_pct')} |"
            )
        lines.append("")

    for sid, title in (
        ("p1", "7. P1 results"),
        ("p3", "8. P3 results"),
        ("p4", "9. P4 results"),
        ("p6", "10. P6 results (DIAGNOSTIC_HIGH_VARIANCE)"),
        ("p11", "11. P11 results"),
    ):
        lines.append(f"## {title}")
        lines.append("")
        for v in VARIANTS:
            e = stats[sid][v]
            lines.append(
                f"- **{v}**: grade={grades[sid][v]} n={e['valid_n']} "
                f"RPS_med={(e.get('rps') or {}).get('median')} "
                f"retained_med={(e.get('rss_retained') or {}).get('median')} "
                f"creep_sus={e.get('creep_suspected_n')} "
                f"classes={e.get('memory_classes')}"
            )
        lines.append("")

    lines += [
        "## 12. Global comparison",
        "",
        f"- SYSTEM_SUSTAINED = **{system_g}**",
        f"- JEMALLOC_SUSTAINED = **{jem_g}**",
        f"- MIMALLOC_SUSTAINED = **{mim_g}**",
        f"- P11 creep: system={p11_creep['system']} jemalloc={p11_creep['jemalloc']} mimalloc={p11_creep['mimalloc']}",
        f"- SUSTAINED_LEADER = **{sust_leader}**",
        "",
        "## 13. Recommended AC7 input",
        "",
        "```text",
        "AC7_FOCUS = fixed-RPS fairness P1 P3 P4 P11",
        f"SUSTAINED_LEADER = {sust_leader}",
        "WATCH = RSS_RETAINED_AFTER_RECOVERY, P11 creep, mimalloc RSS reserve",
        "KEEP_CONNECTIONS = AC6 frozen profiles",
        "```",
        "",
        "---",
        "",
        "```text",
        f"AC6_SUSTAINED = {complete}",
        f"VALID_SCENARIOS = {','.join(valid_sc) if valid_sc else 'NONE'}",
        f"INVALID_SCENARIOS = {','.join(invalid_sc) if invalid_sc else 'NONE'}",
        f"SYSTEM_SUSTAINED = {system_g}",
        f"JEMALLOC_SUSTAINED = {jem_g}",
        f"MIMALLOC_SUSTAINED = {mim_g}",
        f"P11_RAM_CREEP_SYSTEM = {p11_creep['system']}",
        f"P11_RAM_CREEP_JEMALLOC = {p11_creep['jemalloc']}",
        f"P11_RAM_CREEP_MIMALLOC = {p11_creep['mimalloc']}",
        f"SUSTAINED_LEADER = {sust_leader}",
        f"READY_FOR_AC7_FIXED_RPS = {'YES' if ready else 'NO'}",
        "DEFAULT_ALLOCATOR = STILL_SYSTEM",
        "```",
        "",
    ]
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text("\n".join(lines) + "\n")
    print(f"wrote {report}")
    print(json.dumps({k: out[k] for k in ("ac6_status", "valid_scenarios", "system_sustained", "jemalloc_sustained", "mimalloc_sustained", "sustained_leader", "ready_for_ac7", "p11_ram_creep")}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
