#!/usr/bin/env python3
"""GATE-DEP-007 — hot-path pattern budget."""

from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Any

from common import (
    EXIT_CONFIG,
    Finding,
    SEVERITY_ERROR,
    SEVERITY_INFO,
    SEVERITY_WARN,
    emit_human,
    emit_json,
    find_repo_root,
    load_toml,
    worst_exit,
)
from provider_common import (
    STATUS_KNOWN_DEBT,
    STATUS_REVIEW,
    STATUS_VIOLATION,
    surface_finding,
)
from rust_scan import iter_code_lines

GATE = "GATE-DEP-007"
POLICY = "DB1 §15; dependency-hot-path-baseline.toml"

# box_dyn is CRITICAL: new unbaselined Box<dyn> on hot path is FAIL_CLOSED (VIOLATION),
# not REVIEW. Baselined rows remain KNOWN_DEBT when counts match.
CRITICAL = frozenset(
    {"arc_dyn", "box_dyn", "pin_box_dyn_future", "boxbody", "bodyext_collect"}
)
AMBIGUOUS = frozenset({"headermap_clone", "mutex_rwlock", "arc_clone", "to_bytes"})

PATTERN_RES: dict[str, re.Pattern[str]] = {
    "arc_dyn": re.compile(r"\bArc\s*<\s*dyn\b"),
    "box_dyn": re.compile(r"\bBox\s*<\s*dyn\b"),
    "pin_box_dyn_future": re.compile(
        r"\bPin\s*<\s*Box\s*<\s*dyn\s+(?:std::future::)?Future\b"
    ),
    "boxbody": re.compile(r"\bBoxBody\b"),
    "bodyext_collect": re.compile(r"\bBodyExt\s*::\s*collect\b"),
    "to_bytes": re.compile(r"\bto_bytes\b"),
    "headermap_clone": re.compile(r"headers\.clone\(|HeaderMap::clone", re.I),
    "mutex_rwlock": re.compile(r"\b(Mutex|RwLock)\s*<"),
    "arc_clone": re.compile(r"\bArc\s*::\s*clone\b"),
}


def load_hot_path_baseline(root: Path) -> tuple[list[str], list[dict[str, Any]]]:
    path = root / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml"
    data = load_toml(path)
    zones = [z["path"] for z in data.get("hot_path_zone", []) if isinstance(z, dict) and "path" in z]
    patterns = data.get("pattern", [])
    if not isinstance(patterns, list):
        raise SystemExit(f"{EXIT_CONFIG}: {path}: [[pattern]] missing")
    for row in patterns:
        if row.get("expansion_allowed", False):
            raise SystemExit(f"{EXIT_CONFIG}: pattern {row.get('id')}: expansion_allowed must be false")
    return zones, patterns


def zone_files(root: Path, zones: list[str]) -> list[Path]:
    out: list[Path] = []
    for z in zones:
        p = root / z
        if p.is_file() and p.suffix == ".rs":
            out.append(p)
        elif p.is_dir():
            for f in sorted(p.rglob("*.rs")):
                rel = str(f.relative_to(root))
                if any(x in rel for x in ("/tests/", "/benches/", "/examples/")):
                    continue
                if f.name.endswith("_tests.rs") or f.name == "tests.rs":
                    continue
                out.append(f)
    # dedup
    seen: set[str] = set()
    uniq: list[Path] = []
    for f in out:
        k = str(f)
        if k in seen:
            continue
        seen.add(k)
        uniq.append(f)
    return uniq


def in_cfg_test(lines: list[tuple[int, str]], idx: int) -> bool:
    for j in range(idx, max(-1, idx - 30), -1):
        if j < 0:
            break
        if "#[cfg(test)]" in lines[j][1].replace(" ", ""):
            return True
    return False


def scan_file(path: Path) -> dict[str, int]:
    text = path.read_text(encoding="utf-8", errors="replace")
    lines = iter_code_lines(text)
    counts: dict[str, int] = defaultdict(int)
    for i, (_ln, t) in enumerate(lines):
        if in_cfg_test(lines, i):
            continue
        for name, pat in PATTERN_RES.items():
            if pat.search(t):
                counts[name] += 1
    return dict(counts)


def run(root: Path) -> list[Finding]:
    zones, baseline_rows = load_hot_path_baseline(root)
    by_key: dict[tuple[str, str], dict[str, Any]] = {}
    for row in baseline_rows:
        key = (str(row["file"]), str(row["pattern_class"]))
        if key in by_key:
            raise SystemExit(f"{EXIT_CONFIG}: duplicate hot-path baseline {key}")
        by_key[key] = row

    findings: list[Finding] = []
    observed: dict[tuple[str, str], int] = {}

    for fpath in zone_files(root, zones):
        rel = str(fpath.relative_to(root))
        counts = scan_file(fpath)
        for pname, count in counts.items():
            observed[(rel, pname)] = count
            row = by_key.get((rel, pname))
            if row is None:
                # new file/pattern
                if pname in CRITICAL:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=pname,
                            file=rel,
                            line=0,
                            symbol=pname,
                            message=(
                                f"new critical hot-path pattern {pname!r} "
                                f"(count={count}) outside baseline"
                            ),
                            policy_source=POLICY,
                            suggested_action="Do not expand; remediate or authorize new baseline row",
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_WARN,
                            status=STATUS_REVIEW,
                            provider=pname,
                            file=rel,
                            line=0,
                            symbol=pname,
                            message=(
                                f"new/ambiguous hot-path pattern {pname!r} "
                                f"(count={count}) not in baseline — ARCHITECTURAL_RISK / PERFORMANCE_RISK "
                                f"(MEASURED_REGRESSION=NOT_EVALUATED)"
                            ),
                            policy_source=POLICY,
                            suggested_action="Human review; do not treat as measured regression",
                        )
                    )
                continue

            expected = int(row["observed_count"])
            if count > expected:
                if pname in CRITICAL:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=pname,
                            file=rel,
                            line=0,
                            symbol=str(row.get("symbol_or_line_scope") or pname),
                            message=(
                                f"hot-path count increased for {pname}: "
                                f"observed={count} baseline={expected}"
                            ),
                            policy_source=POLICY,
                            suggested_action="Revert expansion or authorize budget increase",
                            baseline_id=str(row["id"]),
                            exception_id=str(row.get("exception_id") or ""),
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_WARN,
                            status=STATUS_REVIEW,
                            provider=pname,
                            file=rel,
                            line=0,
                            symbol=str(row.get("symbol_or_line_scope") or pname),
                            message=(
                                f"hot-path count increased for {pname}: "
                                f"observed={count} baseline={expected} "
                                f"(MEASURED_REGRESSION=NOT_EVALUATED)"
                            ),
                            policy_source=POLICY,
                            suggested_action="Review whether clone/lock/path is hot",
                            baseline_id=str(row["id"]),
                            exception_id=str(row.get("exception_id") or ""),
                        )
                    )
            else:
                # exact or lower — report as known debt / info once per critical class lightly
                status = str(row.get("status") or "known-debt")
                sev = SEVERITY_INFO
                st = STATUS_KNOWN_DEBT
                if status in {"temporary", "known-debt"}:
                    sev = SEVERITY_WARN
                    st = STATUS_KNOWN_DEBT
                elif status == "review-required":
                    sev = SEVERITY_WARN
                    st = STATUS_REVIEW
                elif status == "accepted":
                    sev = SEVERITY_INFO
                    st = "ACCEPTED"
                # Only emit for critical/temporary to avoid 76 INFO spam — emit all as INFO collapsed?
                # User wants known debt visible. Emit WARN for temporary/critical; INFO skip for noise.
                if pname in CRITICAL or status == "temporary":
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=sev if sev != SEVERITY_INFO else SEVERITY_WARN,
                            status=st,
                            provider=pname,
                            file=rel,
                            line=0,
                            symbol=str(row.get("symbol_or_line_scope") or pname),
                            message=(
                                f"baselined hot-path {pname} count={count} "
                                f"(ARCHITECTURAL_RISK pre-existing; MEASURED_REGRESSION=NOT_EVALUATED)"
                            ),
                            policy_source=str(row.get("policy_reference") or POLICY),
                            suggested_action="Do not expand budget",
                            baseline_id=str(row["id"]),
                            exception_id=str(row.get("exception_id") or ""),
                        )
                    )

    # Baseline rows with zero observed (pattern removed) — INFO only
    for (rel, pname), row in sorted(by_key.items()):
        if (rel, pname) not in observed:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_INFO,
                    status="INFO",
                    provider=pname,
                    file=rel,
                    line=0,
                    symbol=str(row.get("symbol_or_line_scope") or pname),
                    message="baseline pattern no longer observed (debt may have shrunk)",
                    policy_source=POLICY,
                    suggested_action="Optionally shrink baseline in a later authorized phase",
                    baseline_id=str(row["id"]),
                    exception_id=str(row.get("exception_id") or ""),
                )
            )

    findings.sort(
        key=lambda f: (
            0 if f.severity == "ERROR" else 1 if f.status == STATUS_REVIEW else 2 if f.status == STATUS_KNOWN_DEBT else 3,
            f.file,
            f.dependency,
            f.message,
        )
    )
    return findings


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", type=Path, default=None)
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)
    try:
        root = find_repo_root(args.root) if args.root else find_repo_root()
        findings = run(root)
    except SystemExit as e:
        code = int(str(e).split(":", 1)[0]) if str(e)[:1].isdigit() else EXIT_CONFIG
        print(str(e), file=sys.stderr)
        return code if code in (1, 2) else EXIT_CONFIG
    if args.json:
        emit_json(
            {
                "gate": GATE,
                "exit": worst_exit(findings),
                "findings": [f.to_dict() for f in findings],
            }
        )
    else:
        emit_human(findings, "GATE-DEP-007: hot-path pattern budget")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
