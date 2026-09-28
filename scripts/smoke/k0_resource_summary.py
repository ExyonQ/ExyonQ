#!/usr/bin/env python3
"""Parse docker stats CSV (sample_resources.sh format) for K0 baseline summaries."""
from __future__ import annotations

import json
import sys
from pathlib import Path


def mem_mib_to_bytes(value: float) -> int:
    return int(round(value * 1024 * 1024))


def parse_docker_mem(s: str) -> float:
    """Parse docker MemUsage used part to MiB (e.g. 3.844MiB, 1.2GiB)."""
    s = (s or "0").split("/")[0].strip().upper()
    if not s:
        return 0.0
    if s.endswith("GIB"):
        return float(s[:-3]) * 1024
    if s.endswith("MIB"):
        return float(s[:-3])
    if s.endswith("KIB"):
        return float(s[:-3]) / 1024
    if s.endswith("B") and len(s) > 1:
        return float(s[:-1]) / (1024 * 1024)
    try:
        return float(s) / (1024 * 1024)
    except ValueError:
        return 0.0


def summarize_resources_csv(csv_path: Path) -> dict[str, float]:
    cpu_peak = 0.0
    mem_mib_peak = 0.0
    if not csv_path.is_file():
        return {"cpu_peak_pct": 0.0, "rss_peak_mib": 0.0, "rss_peak_bytes": 0}
    header = csv_path.read_text().splitlines()[:1]
    col_mem = 2
    if header and "mem_mib" in header[0].lower():
        col_mem = 2
    elif header and "mem_used" in header[0].lower():
        col_mem = 2
    for line in csv_path.read_text().splitlines()[1:]:
        parts = line.split(",")
        if len(parts) < 3:
            continue
        try:
            cpu_peak = max(cpu_peak, float(parts[1].replace("%", "")))
        except ValueError:
            pass
        raw_mem = parts[col_mem]
        try:
            mem_mib_peak = max(mem_mib_peak, float(raw_mem))
        except ValueError:
            mem_mib_peak = max(mem_mib_peak, parse_docker_mem(raw_mem))
    return {
        "cpu_peak_pct": cpu_peak,
        "rss_peak_mib": mem_mib_peak,
        "rss_peak_bytes": mem_mib_to_bytes(mem_mib_peak),
    }


def summarize_stats_line(stats_path: Path) -> dict[str, float]:
    if not stats_path.is_file():
        return {"cpu_peak_pct": 0.0, "rss_peak_mib": 0.0, "rss_peak_bytes": 0}
    text = stats_path.read_text().strip()
    cpu = 0.0
    mem_s = ""
    if "cpu=" in text and "mem=" in text:
        for part in text.replace("cpu=", " cpu=").replace("mem=", " mem=").split():
            if part.startswith("cpu="):
                cpu = float(part[4:].replace("%", "") or 0)
            if part.startswith("mem="):
                mem_s = part[4:]
    elif "," in text:
        cpu_s, mem_s = text.split(",", 1)
        cpu = float(cpu_s.replace("%", "") or 0)
    mem_mib = parse_docker_mem(mem_s)
    return {
        "cpu_peak_pct": cpu,
        "rss_peak_mib": mem_mib,
        "rss_peak_bytes": mem_mib_to_bytes(mem_mib),
    }


def merge_result_summary(sdir: Path, sid: str) -> dict:
    result = json.loads((sdir / "result.json").read_text())
    res = summarize_resources_csv(sdir / "resources.csv")
    if res["rss_peak_bytes"] == 0:
        res = summarize_stats_line(sdir / "stats-after.txt")
    out = {
        "scenario": sid,
        "rps": result.get("rps"),
        "p50_ms": result.get("p50_ms"),
        "p95_ms": result.get("p95_ms"),
        "p99_ms": result.get("p99_ms"),
        "ok": result.get("ok"),
        "fail": result.get("fail"),
        "success_rate": result.get("success_rate"),
        "expect_body": result.get("expect_body"),
        **res,
    }
    (sdir / "summary.json").write_text(json.dumps(out, indent=2) + "\n")
    return out


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: k0_resource_summary.py SCENARIO_DIR SCENARIO_ID", file=sys.stderr)
        return 2
    out = merge_result_summary(Path(sys.argv[1]), sys.argv[2])
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
