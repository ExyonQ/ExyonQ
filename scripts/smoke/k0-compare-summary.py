#!/usr/bin/env python3
"""Summarize K0.6 controlled compare for internal gate report (no public claims)."""
from __future__ import annotations

import json
import sys
from pathlib import Path

SCENARIOS = [
    "p1", "p2", "p3", "p4", "p5", "p6", "p7", "p8", "p9", "p10", "p11", "p12", "p13",
]
RIVALS = [
    "exyonq",
    "nginx-stable",
    "nginx-mainline",
    "haproxy",
    "envoy",
    "traefik",
    "caddy",
    "apache",
]


def load_perf(path: Path) -> dict | None:
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text())
    except (json.JSONDecodeError, OSError):
        return None
    summary = data.get("summary") or {}
    rewrk = data.get("rewrk") or {}
    return {
        "rps": summary.get("requestsPerSec"),
        "p50": summary.get("latency", {}).get("p50"),
        "p99": summary.get("latency", {}).get("p99"),
        "errors": summary.get("errors") or rewrk.get("errors_total"),
        "non2xx": summary.get("non2xx") or rewrk.get("non2xx_total"),
    }


def functional_matrix(run_dir: Path) -> dict[str, dict[str, str]]:
    out: dict[str, dict[str, str]] = {}
    fpath = run_dir / "functional.jsonl"
    if not fpath.is_file():
        return out
    for line in fpath.read_text().splitlines():
        if not line.strip():
            continue
        try:
            row = json.loads(line)
        except json.JSONDecodeError:
            continue
        srv = row.get("server") or row.get("variant") or "?"
        case = row.get("case") or row.get("id") or "?"
        status = row.get("status") or row.get("result") or "?"
        out.setdefault(srv, {})[case] = status
    return out


def exyonq_counters(run_dir: Path) -> dict[str, int]:
    counters: dict[str, int] = {}
    for name in ("metrics-final.txt", "metrics-after-compare.txt"):
        p = run_dir / name
        if not p.is_file():
            continue
        for line in p.read_text().splitlines():
            if line.startswith("exyonq_"):
                k, _, v = line.partition(" ")
                try:
                    counters[k] = int(v)
                except ValueError:
                    pass
    return counters


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: k0-compare-summary.py RESULTS_DEV_DIR [HOST_LABEL]", file=sys.stderr)
        return 2
    base = Path(sys.argv[1])
    host = sys.argv[2] if len(sys.argv) > 2 else base.name

    run_dir = base / "run"
    if not run_dir.is_file() and (base / "run").is_dir():
        run_dir = base / "run"
    elif (base / "latest").is_symlink() or (base / "latest").is_dir():
        run_dir = base / "latest"
    if not run_dir.is_dir():
        run_dir = base

    gate = (base / "gate.txt").read_text().strip() if (base / "gate.txt").is_file() else "UNKNOWN"
    meta = {}
    if (run_dir / "run_meta.json").is_file():
        meta = json.loads((run_dir / "run_meta.json").read_text())

    matrix: dict[str, dict[str, dict | None]] = {}
    for sid in SCENARIOS:
        matrix[sid] = {}
        for rival in RIVALS:
            matrix[sid][rival] = load_perf(run_dir / f"{sid}-{rival}.json")

    report = {
        "host": host,
        "gate": gate,
        "run_id": meta.get("run_id") or run_dir.name,
        "loadgen_saturated": meta.get("loadgen_saturated"),
        "loadgen_cpu_peak_pct": meta.get("loadgen_cpu_peak_pct"),
        "p2_p3_bytes_verified": meta.get("p2_p3_bytes_verified"),
        "functional": functional_matrix(run_dir),
        "perf_matrix": matrix,
        "counters": exyonq_counters(run_dir),
        "meta_flags": {
            k: meta.get(k)
            for k in (
                "official",
                "official_claim",
                "publishable",
                "rivals_from_cache",
                "network",
                "load_mode",
                "claim_level",
            )
        },
    }
    print(json.dumps(report, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
