#!/usr/bin/env python3
"""GATE-DEP-005 — provider error containment on stable façades."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

from common import (
    EXIT_CONFIG,
    Finding,
    SEVERITY_ERROR,
    SEVERITY_WARN,
    emit_human,
    emit_json,
    find_repo_root,
    worst_exit,
)
from provider_common import (
    ANYHOW_ALLOWED_EXTRA,
    STABLE_FACADE_CRATES,
    STATUS_KNOWN_DEBT,
    STATUS_REVIEW,
    STATUS_VIOLATION,
    build_crate_index,
    collect_pub_item_regions,
    find_baseline,
    load_baseline,
    scan_provider_hits,
    surface_finding,
)

GATE = "GATE-DEP-005"
POLICY = "DB1 §14 / §19; provider errors on stable façades"

ERROR_PATH_MARKERS = (
    "hyper::Error",
    "rustls::Error",
    "wasmtime::Error",
    "notify::Error",
    "notify::EventKind",
    "quinn::ConnectionError",
    "quinn::WriteError",
    "h3::Error",
    "anyhow::Error",
    "anyhow::Result",
    "Box<dyn std::error::Error",
    "Box<dyn Error",
)


def run(root: Path) -> list[Finding]:
    baseline = load_baseline(root)
    findings: list[Finding] = []
    focus = STABLE_FACADE_CRATES | {"exyonq-module-api", "exyonq-runtime-plan"}

    for cf in build_crate_index(root):
        if cf.kind != "prod" or cf.crate not in focus:
            continue
        source = cf.abs_path.read_text(encoding="utf-8", errors="replace")

        # Direct provider error paths anywhere in façade crate prod sources
        for h in scan_provider_hits(source):
            if h.path in ERROR_PATH_MARKERS or any(h.path.endswith(m.split("::")[-1]) and m.startswith(h.rust_root) for m in ERROR_PATH_MARKERS):
                pass
            interesting = None
            for marker in ERROR_PATH_MARKERS:
                if h.path == marker or h.path.startswith(marker):
                    interesting = marker
                    break
            if interesting is None and h.provider in {"hyper", "rustls", "wasmtime", "notify", "quinn", "h3", "anyhow"}:
                if h.symbol in {"Error", "Result", "EventKind"} or h.path.endswith("::Error"):
                    interesting = h.path
            if interesting is None:
                continue

            provider = h.provider
            be = find_baseline(
                baseline,
                provider=provider,
                file=cf.rel_path,
                symbol_or_pattern=h.path,
                kinds=("error", "pub_type", "import"),
            )
            if be:
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_WARN,
                        status=STATUS_KNOWN_DEBT,
                        provider=provider,
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message=f"baselined provider error surface ({be.id})",
                        policy_source=be.policy_reference,
                        suggested_action="Do not expand error coupling",
                        baseline_id=be.id,
                    )
                )
                continue

            # anyhow zoning
            if provider == "anyhow":
                if cf.crate in ANYHOW_ALLOWED_EXTRA:
                    continue
                # runtime-plan / module-api: require baseline; else ERROR
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider="anyhow",
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message="anyhow error/result on stable façade without baseline",
                        policy_source=POLICY,
                        suggested_action="Map to ExyonQ error type or restrict to CLI/xtask",
                    )
                )
                continue

            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=provider,
                    file=cf.rel_path,
                    line=h.line,
                    symbol=h.path,
                    message="provider error type referenced in stable façade crate",
                    policy_source=POLICY,
                    suggested_action="Map at boundary to ExyonQ error",
                )
            )

        # pub signatures containing Box<dyn Error> / anyhow::Result
        for line, vis, item_kind, sig in collect_pub_item_regions(source):
            if vis != "pub":
                continue
            if "Box<dyn" in sig and "Error" in sig:
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_WARN,
                        status=STATUS_REVIEW,
                        provider="std",
                        file=cf.rel_path,
                        line=line,
                        symbol=item_kind,
                        message="Box<dyn Error> on public façade item (may hide provider errors)",
                        policy_source=POLICY,
                        suggested_action="Prefer concrete ExyonQ error enums",
                    )
                )
            if "anyhow::" in sig:
                be = find_baseline(
                    baseline,
                    provider="anyhow",
                    file=cf.rel_path,
                    symbol_or_pattern="anyhow::Result",
                    kinds=("error",),
                )
                if be:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_WARN,
                            status=STATUS_KNOWN_DEBT,
                            provider="anyhow",
                            file=cf.rel_path,
                            line=line,
                            symbol="anyhow::Result",
                            message=f"baselined anyhow on pub item ({be.id})",
                            policy_source=be.policy_reference,
                            suggested_action="Do not expand",
                            baseline_id=be.id,
                        )
                    )
                elif cf.crate not in ANYHOW_ALLOWED_EXTRA:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="anyhow",
                            file=cf.rel_path,
                            line=line,
                            symbol="anyhow",
                            message=f"anyhow in public {item_kind} on stable façade",
                            policy_source=POLICY,
                            suggested_action="Replace with ExyonQ error type",
                        )
                    )

    # Dedup
    uniq: list[Finding] = []
    seen: set[tuple] = set()
    for f in findings:
        key = (f.gate_id, f.file, f.line, f.dependency, f.symbol, f.status)
        if key in seen:
            continue
        seen.add(key)
        uniq.append(f)
    uniq.sort(key=lambda f: (f.file, f.line, f.dependency, f.symbol))
    return uniq


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
        emit_human(findings, "GATE-DEP-005: provider error containment")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
