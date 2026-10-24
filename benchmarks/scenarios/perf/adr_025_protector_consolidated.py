#!/usr/bin/env python3
"""Write ADR-025 protector matrix consolidated summary — diagnostic only."""
from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

DIR = Path(__file__).resolve().parent

PROVISIONAL = {
    "p1": "NOT-ATTRIBUTABLE provisional",
    "p2": "SENDFILE_GATE provisional",
    "p3": "SENDFILE_GATE provisional",
    "p4": "NOT-ATTRIBUTABLE / PERF-WATCH provisional",
    "p5": "HARNESS-INVALID provisional",
    "p6": "NOT-ATTRIBUTABLE provisional",
    "p7": "NOT-ATTRIBUTABLE / PERF-WATCH provisional",
    "p8": "HARNESS-INVALID provisional",
    "p9": "SERVER-WATCH provisional",
    "p10": "PERF-WATCH-NOISE provisional",
    "p11": "NOT-ATTRIBUTABLE provisional",
    "p12": "PERF-WATCH-NOISE provisional",
    "p13": "HARNESS-INVALID provisional",
}


def _load_gates():
    spec = importlib.util.spec_from_file_location(
        "adr_025_protector_gates", DIR / "adr_025_protector_gates.py"
    )
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


gates = _load_gates()


def load_pair(root: Path, host: str, sid: str) -> tuple[dict, dict] | None:
    b = root / host / sid / "blocking" / "result.json"
    e = root / host / sid / "epoll" / "result.json"
    if not b.exists() or not e.exists():
        return None
    return json.loads(b.read_text()), json.loads(e.read_text())


def host_gate(root: Path, host: str) -> str:
    g = root / host / "gate.txt"
    if not g.exists():
        return "in progress or missing"
    for line in g.read_text().splitlines():
        if line.startswith("gate="):
            return line.split("=", 1)[1]
    return "in progress or missing"


def sendfile_line(root: Path, host: str, sid: str) -> str:
    sf = root / host / sid / "epoll" / "sendfile.txt"
    if not sf.is_file():
        return "missing"
    for line in sf.read_text(encoding="utf-8").splitlines():
        if line.startswith("sendfile_complete_delta="):
            return line.split("=", 1)[1]
    return "missing"


def write_consolidated(root: Path) -> str:
    lines = [
        "# ADR-025 protector matrix — consolidated",
        "",
        "**Classification: PROTECTOR-EXPLORATORY** (orchestrator diagnostic pass — see host `gate.txt` for formal P2/P3 verdict per host)",
        "",
        "Not publishable. No public claims. No HTML. No mode_b_run_id.",
        "",
        "Blocking (flags OFF) vs `EXYONQ_EPOLL_STATIC=1` + `EXYONQ_EPOLL_SENDFILE=1`. exyonq-only.",
        "",
        "## Formal ADR-025 gate",
        "",
        "```text",
        gates.FORMAL_GATE_SUMMARY,
        "```",
        "",
        "## Gate semantics",
        "",
        "| Verdict | Meaning |",
        "|---------|---------|",
        "| **PROTECTOR-PASS** | P2/P3 sendfile gates green; excluded/watch rows are informative only. |",
        "| **PROTECTOR-FAIL** | P2/P3 failed (ratio, HTTP errors, or sendfile `complete_delta=0`). |",
        "| **PROTECTOR-EXPLORATORY** | Formal P2/P3 gate not evaluable (missing data, incomplete harness, partial run). |",
        "",
        "PROTECTOR-PASS does **not** mean every scenario performed perfectly.",
        "",
        "## Policy",
        "",
        "Host `gate.txt` reflects the formal P2/P3 sendfile gate for that host.",
        "Orchestrator full-matrix runs remain diagnostic until selective P2/P3 reruns are clean on both hosts.",
        "",
        f"Sendfile engaged gate: `complete_delta > 0` and RPS ratio ≥ {gates.SENDFILE_RATIO_THRESHOLD:.0%} on **P2/P3 only**.",
        "",
        "## Harness invalid vs protector fail",
        "",
        "| Tag | Meaning |",
        "|-----|---------|",
        "| HARNESS-INVALID | loader/harness bug or timeout — not epoll signal |",
        "| SERVER-WATCH | server response anomaly — investigate separately |",
        "| NOT-ATTRIBUTABLE | degraded throughput not attributable to sendfile (delta=0) |",
        "| PERF-WATCH-NOISE | P10/P12 watch noise — not ADR-025 fail |",
        "| PROTECTOR-FAIL | P2/P3 sendfile engaged but ratio/stability gate failed |",
        "",
        "## Provisional scenario tags",
        "",
        "| Scenario | Tag | Notes |",
        "|----------|-----|-------|",
        "| P1 | NOT-ATTRIBUTABLE provisional | |",
        "| P2 | SENDFILE_GATE provisional | formal ADR-025 gate |",
        "| P3 | SENDFILE_GATE provisional | formal ADR-025 gate |",
        "| P4 | NOT-ATTRIBUTABLE provisional | degraded OK if sendfile delta=0 |",
        "| P5 | HARNESS-INVALID provisional | Connection close loadgen |",
        "| P6 | NOT-ATTRIBUTABLE provisional | |",
        "| P7 | NOT-ATTRIBUTABLE provisional | confirm Oracle on selective rerun |",
        "| P8 | HARNESS-INVALID provisional | SSE loader |",
        "| P9 | SERVER-WATCH provisional | GET 64k under modules |",
        "| P10 | PERF-WATCH-NOISE provisional | |",
        "| P11 | NOT-ATTRIBUTABLE provisional | |",
        "| P12 | PERF-WATCH-NOISE provisional | |",
        "| P13 | HARNESS-INVALID provisional | HTTP/3 harness |",
        "",
        "## Host run status",
        "",
        "| Host | Remote gate.txt | Formal P2/P3 |",
        "|------|-----------------|--------------|",
    ]
    for host in ["netcup-amd64", "oracle-aarch64"]:
        gate = host_gate(root, host)
        formal = gate
        if gate == "PROTECTOR-PASS":
            formal = "PASS (excluded/watch may remain)"
        elif gate == "PROTECTOR-EXPLORATORY":
            formal = "unevaluable on this host"
        lines.append(f"| {host} | {gate} | {formal} |")

    lines += ["", "## Throughput snapshot (when artifacts present)", ""]
    for host in ["netcup-amd64", "oracle-aarch64"]:
        rows = []
        for sid in [f"p{i}" for i in range(1, 14)]:
            pair = load_pair(root, host, sid)
            if not pair:
                continue
            b, e = pair
            br, er = b.get("rps", 0), e.get("rps", 0)
            ratio = er / br if br > 0 else 0
            tag = PROVISIONAL.get(sid, "")
            hb = b.get("harness_classification") or ""
            he = e.get("harness_classification") or ""
            if hb or he:
                tag = f"{tag} [{hb or '-'}/{he or '-'}]".strip()
            sf = sendfile_line(root, host, sid)
            rows.append((sid, br, er, ratio, b.get("fail", 0), e.get("fail", 0), sf, tag))
        if not rows:
            continue
        lines.append(f"### {host}")
        lines.append("")
        lines.append(
            "| Scenario | blocking RPS | epoll RPS | ratio | b-fail | e-fail | sendfile Δ | tag |"
        )
        lines.append(
            "|----------|-------------|-----------|-------|--------|--------|------------|-----|"
        )
        for sid, br, er, ratio, bf, ef, sf, tag in rows:
            lines.append(
                f"| {sid} | {br:.1f} | {er:.1f} | {ratio:.3f} | {bf} | {ef} | {sf} | {tag} |"
            )
        lines.append("")

    lines += [
        "## Next (selective rerun only)",
        "",
        "1. P2/P3 sendfile engagement + ratio on Netcup + Oracle for formal ADR-025 gate.",
        "2. P5, P8, P13 harness fixes stay out of sendfile PROTECTOR-FAIL.",
        "3. P4/P7/P10/P12 degraded with sendfile delta=0 → watch only.",
        "",
        "_Diagnostic only. No rivals, HTML, mode_b_run_id, or public claims._",
        "",
    ]
    root.mkdir(parents=True, exist_ok=True)
    (root / "consolidated-summary.md").write_text("\n".join(lines), encoding="utf-8")
    (root / "classification.txt").write_text(
        "classification=PROTECTOR-EXPLORATORY\n"
        "final_gate=no\n"
        "sendfile_gate_scenarios=P2,P3\n"
        "publishable=no\n"
        "html=no\n"
        "mode_b_run_id=no\n"
        "claims=no\n",
        encoding="utf-8",
    )
    return "PROTECTOR-EXPLORATORY"


def main() -> int:
    root = Path(sys.argv[1])
    print(write_consolidated(root))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
