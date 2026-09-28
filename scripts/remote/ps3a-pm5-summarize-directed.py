#!/usr/bin/env python3
"""Summarize PS3A-PM5 directed P1 reps and compare to frozen PS2 baseline medians."""
from __future__ import annotations

import json
import math
import sys
from pathlib import Path


def stats(vals: list[float]) -> dict:
    if not vals:
        return {"n": 0}
    vals = sorted(vals)
    n = len(vals)
    mean = sum(vals) / n
    if n % 2:
        median = vals[n // 2]
    else:
        median = (vals[n // 2 - 1] + vals[n // 2]) / 2
    var = sum((x - mean) ** 2 for x in vals) / n
    stdev = math.sqrt(var)
    cv = (stdev / mean * 100.0) if mean else 0.0
    return {
        "n": n,
        "median": median,
        "mean": mean,
        "min": vals[0],
        "max": vals[-1],
        "stdev": stdev,
        "cv_pct": round(cv, 3),
    }


def load_rep_metrics(rep_dir: Path) -> dict | None:
    summary = rep_dir / "p1" / "exyonq" / "summary.json"
    if summary.is_file():
        d = json.loads(summary.read_text())
        return {
            "rps": float(d.get("rps") or 0),
            "p50_ms": float(d.get("p50_ms") or 0),
            "p95_ms": float(d.get("p95_ms") or 0),
            "p99_ms": float(d.get("p99_ms") or 0),
            "http_error_rate": float(100.0 - float(d.get("success_rate") or 100.0)),
        }
    alt = rep_dir / "p1-exyonq.json"
    if alt.is_file():
        d = json.loads(alt.read_text())
        s = d.get("summary") or {}
        return {
            "rps": float(s.get("rps") or s.get("requests_per_sec") or 0),
            "p50_ms": float(s.get("p50_ms") or 0),
            "p95_ms": float(s.get("p95_ms") or 0),
            "p99_ms": float(s.get("p99_ms") or 0),
            "http_error_rate": 0.0,
        }
    return None


def baseline_p1_median(index: dict, worker: str) -> float | None:
    for s in index.get("scenarios", []):
        if s.get("scenario") == "p1" and s.get("worker") == worker:
            st = s.get("statistics") or {}
            rps = st.get("rps") if isinstance(st.get("rps"), dict) else st
            if isinstance(rps, dict) and "median" in rps:
                return float(rps["median"])
    # Contract revalidation may carry default_tokio / epoll_static compare metadata only.
    return None


def classify(delta_pct: float | None, health_ok: bool, fatals: int, cv: float) -> str:
    if not health_ok or fatals > 0:
        return "FAIL"
    if delta_pct is None:
        # No baseline row — health/fatal gate only; CV may be CONDITIONAL
        return "CONDITIONAL" if cv > 8.0 else "PASS"
    # Blocking regression band ~ -5% median (aligned with prior PM3 directed judgment)
    if delta_pct < -5.0:
        return "FAIL"
    if cv > 8.0 or delta_pct < -2.0:
        return "CONDITIONAL"
    return "PASS"


def main() -> int:
    if len(sys.argv) < 3:
        print("usage: ps3a-pm5-summarize-directed.py RESULTS_DIR BASELINE_INDEX_JSON", file=sys.stderr)
        return 2
    results = Path(sys.argv[1])
    index = json.loads(Path(sys.argv[2]).read_text())
    modes = ["default_tokio", "sync_accept", "io_uring", "epoll_listen", "epoll_static"]
    # Map mode → PS2 worker row (epoll_static has no dedicated P1 row → use epoll_listen)
    baseline_map = {
        "default_tokio": "default_tokio",
        "sync_accept": "sync_accept",
        "io_uring": "io_uring",
        "epoll_listen": "epoll_listen",
        "epoll_static": "epoll_listen",
    }
    out: dict = {"modes": {}, "contracts": {}, "verdicts": {}}
    for mode in modes:
        mode_dir = results / "modes" / mode
        reps = []
        for rep in sorted(mode_dir.glob("rep*")):
            m = load_rep_metrics(rep)
            if m:
                reps.append(m)
        summary_txt = (mode_dir / "summary.txt").read_text() if (mode_dir / "summary.txt").is_file() else ""
        health_ok = False
        fatals = 0
        if "HEALTH_OK=" in summary_txt:
            # HEALTH_OK=5/5
            part = [p for p in summary_txt.split() if p.startswith("HEALTH_OK=")][0]
            num, den = part.split("=")[1].split("/")
            health_ok = num == den
        if "FATAL=" in summary_txt:
            fatals = int([p for p in summary_txt.split() if p.startswith("FATAL=")][0].split("=")[1])

        rps = [r["rps"] for r in reps]
        p50 = [r["p50_ms"] for r in reps]
        p95 = [r["p95_ms"] for r in reps]
        p99 = [r["p99_ms"] for r in reps]
        err = [r["http_error_rate"] for r in reps]
        rps_s = stats(rps)
        base_worker = baseline_map[mode]
        base_med = baseline_p1_median(index, base_worker)
        # Also try default_tokio / sync_accept names if missing
        if base_med is None and mode == "default_tokio":
            base_med = baseline_p1_median(index, "tokio") or baseline_p1_median(index, "default")
        delta = None
        if base_med and rps_s.get("median"):
            delta = (rps_s["median"] - base_med) / base_med * 100.0
        verdict = classify(delta, health_ok, fatals, float(rps_s.get("cv_pct") or 0))
        out["modes"][mode] = {
            "rps": rps_s,
            "p50_ms": stats(p50),
            "p95_ms": stats(p95),
            "p99_ms": stats(p99),
            "http_error_rate": stats(err),
            "health_ok": health_ok,
            "worker_fatal_events": fatals,
            "baseline_worker": base_worker,
            "baseline_median_rps": base_med,
            "delta_pct": None if delta is None else round(delta, 3),
            "comparison": verdict,
        }
        key = {
            "default_tokio": "DEFAULT_TOKIO_COMPARISON",
            "sync_accept": "SYNC_ACCEPT_COMPARISON",
            "io_uring": "IO_URING_COMPARISON",
            "epoll_listen": "EPOLL_LISTEN_COMPARISON",
            "epoll_static": "EPOLL_STATIC_COMPARISON",
        }[mode]
        out["verdicts"][key] = verdict

    # Contract qualification probes
    for scen in ["p3", "p8", "p11", "p13"]:
        sfile = results / "contracts" / scen / "summary.txt"
        status = "FAIL"
        if sfile.is_file():
            t = sfile.read_text()
            if "HEALTH=PASS" in t and "FATAL=0" in t and "RC=0" in t:
                status = "PASS"
            elif "HEALTH=PASS" in t and "FATAL=0" in t:
                status = "CONDITIONAL"
        out["contracts"][scen.upper() + "_STATUS"] = status
        out["verdicts"][scen.upper() + "_STATUS"] = status

    # Functional matrix
    func_pass = True
    for mode in modes:
        f = results / "functional" / f"{mode}.txt"
        if not f.is_file() or "HEALTH=PASS" not in f.read_text() or "REQUEST=PASS" not in f.read_text():
            func_pass = False
    out["verdicts"]["LINUX_FUNCTIONAL_MATRIX"] = "PASS" if func_pass else "FAIL"

    out_path = results / "directed-stats.json"
    out_path.write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps(out["verdicts"], indent=2))
    print(f"WROTE {out_path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
