#!/usr/bin/env python3
"""ADR-025 protector matrix gates — sendfile ENGAGED-only (PR-12).

PROTECTOR-FAIL for ADR-025/sendfile may only originate from P2/P3 when sendfile
was engaged (complete_delta > 0) and RPS ratio is below threshold.
"""
from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

SENDFILE_METRIC = "exyonq_epoll_sendfile_complete_total"
SENDFILE_RATIO_THRESHOLD = 0.95

SENDFILE_GATE = frozenset({"p2", "p3", "P2", "P3"})
NOT_ATTRIBUTABLE = frozenset({"p1", "p4", "p6", "p7", "p11", "P1", "P4", "P6", "P7", "P11"})
PERF_WATCH_NOISE = frozenset({"p10", "p12", "P10", "P12"})
HARNESS_INVALID = frozenset({"p5", "p8", "p13", "P5", "P8", "P13"})
SERVER_WATCH = frozenset({"p9", "P9"})
FORMAL_GATE_SCENARIOS = ("p2", "p3")

FORMAL_GATE_SUMMARY = """\
Formal ADR-025 gate:
  P2/P3 only.

Excluded:
  P10/P12 → PERF-WATCH-NOISE if complete_delta=0.
  P1/P4/P6/P7/P11 → NOT-ATTRIBUTABLE if complete_delta=0.
  P5/P8/P13 → HARNESS-INVALID.
  P9 → SERVER-WATCH.

PROTECTOR-PASS does not mean every scenario performed perfectly.
It means the sendfile-attributable P2/P3 gates are green; excluded/watch rows are informative only."""


def _norm_sid(scenario: str) -> str:
    return scenario.lower()


def scenario_bucket(scenario: str) -> str:
    sid = _norm_sid(scenario)
    if sid in {"p2", "p3"}:
        return "SENDFILE_GATE"
    if sid in {"p1", "p4", "p6", "p7", "p11"}:
        return "NOT-ATTRIBUTABLE"
    if sid in {"p10", "p12"}:
        return "PERF-WATCH-NOISE"
    if sid in {"p5", "p8", "p13"}:
        return "HARNESS-INVALID"
    if sid == "p9":
        return "SERVER-WATCH"
    return "UNKNOWN"


def _metric_value(summary_path: Path, metric: str) -> float | None:
    if not summary_path.is_file():
        return None
    for line in summary_path.read_text(encoding="utf-8", errors="replace").splitlines():
        parts = line.split()
        if len(parts) >= 2 and parts[0] == metric:
            try:
                return float(parts[1])
            except ValueError:
                return None
        if "=" in line:
            key, _, val = line.partition("=")
            if key.strip() == metric:
                try:
                    return float(val.strip())
                except ValueError:
                    return None
    return None


def sendfile_complete_delta(before: Path, after: Path) -> float | None:
    """Delta of exyonq_epoll_sendfile_complete_total; None if either side missing."""
    b = _metric_value(before, SENDFILE_METRIC)
    a = _metric_value(after, SENDFILE_METRIC)
    if b is None or a is None:
        return None
    return a - b


def write_sendfile_meta(
    scenario_dir: Path,
    *,
    complete_delta: float | None,
    bucket: str,
    scenario: str | None = None,
) -> None:
    scenario_dir.mkdir(parents=True, exist_ok=True)
    sid = scenario or scenario_dir.parent.name
    lines = [
        f"scenario={sid}",
        f"bucket={bucket}",
        f"sendfile_metric={SENDFILE_METRIC}",
    ]
    if complete_delta is None:
        lines.append("sendfile_complete_delta=MISSING")
    else:
        lines.append(f"sendfile_complete_delta={complete_delta:.6f}")
    (scenario_dir / "sendfile.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")


def _harness_invalid(cls: str | None) -> bool:
    return cls in ("HARNESS-INVALID", "INVALID-LOADGEN")


def _server_watch(cls: str | None) -> bool:
    return cls == "SERVER-WATCH"


def _stack_start_status(mode_dir: Path) -> str | None:
    """Return STACK_START_OK / STACK_START_FAILED / None if artifact missing."""
    path = mode_dir / "stack-start.status"
    if not path.is_file():
        return None
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("status="):
            return line.partition("=")[2].strip() or None
    return None


def _formal_gate_unevaluable(
    expected: list[str],
    incomplete: list[str],
    harness_invalid: list[str],
    server_watch: list[str],
    stack_start_failed: list[str] | None = None,
) -> list[str]:
    """Reasons P2/P3 formal gate cannot be evaluated → PROTECTOR-EXPLORATORY."""
    reasons: list[str] = []
    stack_start_failed = stack_start_failed or []
    for sid in FORMAL_GATE_SCENARIOS:
        if sid not in expected:
            reasons.append(f"{sid}: not in run scope")
            continue
        if sid in incomplete:
            reasons.append(f"{sid}: missing artifacts")
            continue
        matched = False
        for entry in stack_start_failed:
            if entry.startswith(f"{sid}:"):
                reasons.append(entry)
                matched = True
                break
        if matched:
            continue
        for entry in harness_invalid:
            if entry.startswith(f"{sid}:"):
                reasons.append(entry)
                matched = True
                break
        if matched:
            continue
        for entry in server_watch:
            if entry.startswith(f"{sid}:"):
                reasons.append(entry)
                break
    return reasons


def _excluded_informative_notes(
    harness_invalid: list[str],
    server_watch: list[str],
    exploratory_tags: list[str],
) -> list[str]:
    notes: list[str] = []
    notes.extend(harness_invalid)
    notes.extend(server_watch)
    notes.extend(exploratory_tags)
    return notes


def eval_matrix_gates(host_dir: Path, expected: list[str] | None = None) -> dict[str, Any]:
    """Evaluate matrix gates for one host output directory (p*/blocking|epoll layout)."""
    host_dir = Path(host_dir)
    if expected is None or not expected:
        expected = [f"p{i}" for i in range(1, 14)]
    expected = [_norm_sid(s) for s in expected]

    sendfile_gate_failures: list[str] = []
    exploratory_tags: list[str] = []
    harness_invalid: list[str] = []
    server_watch: list[str] = []
    incomplete: list[str] = []
    stack_start_failed: list[str] = []
    rows: list[tuple[str, float, float, float, int, int, str, str]] = []

    for sid in expected:
        bpath = host_dir / sid / "blocking" / "result.json"
        epath = host_dir / sid / "epoll" / "result.json"
        epoll_dir = host_dir / sid / "epoll"
        blocking_dir = host_dir / sid / "blocking"
        bucket = scenario_bucket(sid)

        if not bpath.is_file() or not epath.is_file():
            incomplete.append(sid)
            continue

        b_stack = _stack_start_status(blocking_dir)
        e_stack = _stack_start_status(epoll_dir)
        if b_stack != "STACK_START_OK" or e_stack != "STACK_START_OK":
            stack_start_failed.append(
                f"{sid}: stack start not OK blocking={b_stack} epoll={e_stack}"
            )
            # Still parse metrics for diagnostics, but formal PASS is blocked via unevaluable.
            if bucket == "SENDFILE_GATE":
                rows.append((sid, 0.0, 0.0, 0.0, 0, 0, "STACK_START_FAILED", bucket))
                continue

        b = json.loads(bpath.read_text(encoding="utf-8"))
        e = json.loads(epath.read_text(encoding="utf-8"))
        bh = b.get("harness_classification")
        eh = e.get("harness_classification")

        before = epoll_dir / "metrics-before.summary"
        after = epoll_dir / "metrics-after.summary"
        delta = sendfile_complete_delta(before, after)
        write_sendfile_meta(epoll_dir, complete_delta=delta, bucket=bucket, scenario=sid)

        br, er = float(b.get("rps", 0) or 0), float(e.get("rps", 0) or 0)
        ratio = er / br if br > 0 else (1.0 if er > 0 else 0.0)
        (epoll_dir / "ratio.txt").write_text(
            f"scenario={sid}\nratio={ratio:.6f}\nblocking_rps={br:.3f}\nepoll_rps={er:.3f}\n",
            encoding="utf-8",
        )

        estab_path = epoll_dir / "conns-settle.txt"
        estab = int(estab_path.read_text().strip()) if estab_path.is_file() else 0

        if _harness_invalid(bh) or _harness_invalid(eh):
            harness_invalid.append(f"{sid}: harness invalid blocking={bh} epoll={eh}")
            rows.append((sid, br, er, ratio, b.get("fail", 0), e.get("fail", 0), "HARNESS-INVALID", bucket))
            continue
        if _server_watch(bh) or _server_watch(eh):
            server_watch.append(f"{sid}: server watch blocking={bh} epoll={eh}")
            rows.append((sid, br, er, ratio, b.get("fail", 0), e.get("fail", 0), "SERVER-WATCH", bucket))
            continue

        gate_tag = bucket
        adr025_fail = False

        if bucket == "SENDFILE_GATE":
            if delta is None or delta <= 0:
                adr025_fail = True
                gate_tag = "SENDFILE-NOT-ENGAGED"
                sendfile_gate_failures.append(
                    f"{sid}: sendfile not engaged (complete_delta={delta})"
                )
            elif ratio < SENDFILE_RATIO_THRESHOLD:
                adr025_fail = True
                gate_tag = "SENDFILE-RATIO-FAIL"
                sendfile_gate_failures.append(
                    f"{sid}: epoll RPS {er:.1f} < {SENDFILE_RATIO_THRESHOLD:.0%} blocking {br:.1f} "
                    f"(ratio={ratio:.3f})"
                )
            else:
                gate_tag = "SENDFILE-GATE-PASS"
            if e.get("fail", 0) != 0:
                adr025_fail = True
                sendfile_gate_failures.append(f"{sid}-epoll: http fail={e['fail']}")
            if estab != 0:
                adr025_fail = True
                sendfile_gate_failures.append(f"{sid}-epoll: ESTAB={estab}")
        elif bucket == "NOT-ATTRIBUTABLE":
            gate_tag = "NOT-ATTRIBUTABLE"
            if delta is not None and delta <= 0 and ratio < SENDFILE_RATIO_THRESHOLD:
                exploratory_tags.append(
                    f"{sid}: degraded ratio={ratio:.3f} (NOT-ATTRIBUTABLE, not ADR-025 fail)"
                )
                gate_tag = "NOT-ATTRIBUTABLE-DEGRADED"
        elif bucket == "PERF-WATCH-NOISE":
            gate_tag = "PERF-WATCH-NOISE"
            if delta is not None and delta <= 0 and ratio < SENDFILE_RATIO_THRESHOLD:
                exploratory_tags.append(
                    f"{sid}: perf watch noise ratio={ratio:.3f} (complete_delta=0)"
                )
        elif bucket == "HARNESS-INVALID":
            gate_tag = "HARNESS-INVALID"
        elif bucket == "SERVER-WATCH":
            gate_tag = "SERVER-WATCH"

        rows.append(
            (
                sid,
                br,
                er,
                ratio,
                int(b.get("fail", 0) or 0),
                int(e.get("fail", 0) or 0),
                gate_tag,
                bucket,
            )
        )
        if adr025_fail and bucket != "SENDFILE_GATE":
            pass  # defensive — should never happen

    lines = [
        "# ADR-025 protector matrix (NOT publishable)",
        "",
        f"- Host: {host_dir.name}",
        f"- Expected scenarios: {', '.join(expected)}",
        "",
        "## Formal ADR-025 gate",
        "",
        "```text",
        FORMAL_GATE_SUMMARY,
        "```",
        "",
        "## Gate semantics",
        "",
        "- **PROTECTOR-PASS**: P2/P3 sendfile gates green; stack start OK; excluded/watch notes are informative only.",
        "- **PROTECTOR-FAIL**: P2/P3 failed (ratio, HTTP errors, or sendfile complete_delta=0).",
        "- **PROTECTOR-EXPLORATORY**: formal P2/P3 gate not evaluable (missing data, stack start failure, incomplete harness, partial run).",
        "",
        "| Scenario | blocking RPS | epoll RPS | ratio | b-fail | e-fail | gate | bucket |",
        "|----------|-------------|-----------|-------|--------|--------|------|--------|",
    ]
    for sid, br, er, ratio, bf, ef, gate_tag, bucket in rows:
        lines.append(
            f"| {sid} | {br:.1f} | {er:.1f} | {ratio:.3f} | {bf} | {ef} | {gate_tag} | {bucket} |"
        )
    lines += ["", "## Gate", ""]
    if incomplete:
        lines.append("**INCOMPLETE (missing result.json pairs):**")
        for sid in incomplete:
            lines.append(f"- {sid}")
        lines.append("")
    if stack_start_failed:
        lines.append("**STACK_START_FAILED (formal PASS forbidden):**")
        for s in stack_start_failed:
            lines.append(f"- {s}")
        lines.append("")
    if harness_invalid:
        lines.append("**HARNESS-INVALID (excluded from ADR-025 PROTECTOR-FAIL):**")
        for h in harness_invalid:
            lines.append(f"- {h}")
        lines.append("")
    if server_watch:
        lines.append("**SERVER-WATCH (excluded from ADR-025 PROTECTOR-FAIL):**")
        for s in server_watch:
            lines.append(f"- {s}")
        lines.append("")
    if exploratory_tags:
        lines.append("**Excluded / watch (informative on PASS, not ADR-025 fail):**")
        for t in exploratory_tags:
            lines.append(f"- {t}")
        lines.append("")
    unevaluable = _formal_gate_unevaluable(
        expected, incomplete, harness_invalid, server_watch, stack_start_failed
    )
    if sendfile_gate_failures:
        lines.append("**PROTECTOR-FAIL (P2/P3 sendfile gate only):**")
        for f in sendfile_gate_failures:
            lines.append(f"- {f}")
    elif unevaluable:
        lines.append("**PROTECTOR-EXPLORATORY** (formal P2/P3 gate not evaluable):")
        for r in unevaluable:
            lines.append(f"- {r}")
    else:
        lines.append(
            "**PROTECTOR-PASS** — P2/P3 sendfile gates green. "
            "Does not mean all scenarios performed perfectly."
        )

    (host_dir / "host-summary.md").write_text("\n".join(lines) + "\n", encoding="utf-8")

    excluded_notes = _excluded_informative_notes(harness_invalid, server_watch, exploratory_tags)
    unevaluable = _formal_gate_unevaluable(
        expected, incomplete, harness_invalid, server_watch, stack_start_failed
    )

    if sendfile_gate_failures:
        exit_code = 1
        verdict = "PROTECTOR-FAIL"
    elif unevaluable:
        exit_code = 2
        verdict = "PROTECTOR-EXPLORATORY"
    else:
        exit_code = 0
        verdict = "PROTECTOR-PASS"

    return {
        "verdict": verdict,
        "exit_code": exit_code,
        "host": host_dir.name,
        "sendfile_gate_failures": sendfile_gate_failures,
        "exploratory_tags": exploratory_tags,
        "harness_invalid": harness_invalid,
        "server_watch": server_watch,
        "incomplete": incomplete,
        "stack_start_failed": stack_start_failed,
        "unevaluable": unevaluable,
        "excluded_notes": excluded_notes,
        "sendfile_ratio_threshold": SENDFILE_RATIO_THRESHOLD,
        "formal_gate_summary": FORMAL_GATE_SUMMARY,
    }


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if len(argv) < 2:
        print(
            "usage: adr_025_protector_gates.py eval <host_dir> [p1 p2 ...]",
            file=sys.stderr,
        )
        print(
            "       adr_025_protector_gates.py write-sendfile-meta <epoll_dir> [scenario]",
            file=sys.stderr,
        )
        return 2
    cmd = argv[0]
    if cmd == "eval":
        host_dir = Path(argv[1])
        expected = argv[2:] if len(argv) > 2 else None
        out = eval_matrix_gates(host_dir, expected)
        if out["sendfile_gate_failures"]:
            print("\n".join(out["sendfile_gate_failures"]))
        elif out["unevaluable"]:
            print("PROTECTOR-EXPLORATORY: " + "; ".join(out["unevaluable"]))
        elif out["excluded_notes"]:
            print(out["verdict"])
            print("excluded: " + "; ".join(out["excluded_notes"][:6]), file=sys.stderr)
        else:
            print(out["verdict"])
        return int(out["exit_code"])
    if cmd == "write-sendfile-meta":
        epoll_dir = Path(argv[1])
        scenario = argv[2] if len(argv) > 2 else epoll_dir.parent.name
        bucket = scenario_bucket(scenario)
        before = epoll_dir / "metrics-before.summary"
        after = epoll_dir / "metrics-after.summary"
        delta = sendfile_complete_delta(before, after)
        write_sendfile_meta(epoll_dir, complete_delta=delta, bucket=bucket, scenario=scenario)
        return 0
    print("unknown command", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
