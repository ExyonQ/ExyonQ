#!/usr/bin/env python3
"""GATE-DEP-003 / GATE-DEP-004 — public re-exports and provider types on façades."""

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
    CRITICAL_PROVIDERS,
    STABLE_FACADE_CRATES,
    STATUS_ACCEPTED,
    STATUS_KNOWN_DEBT,
    STATUS_VIOLATION,
    _PATH_RE,
    ROOT_TO_PROVIDER,
    build_crate_index,
    bytes_path_accepted,
    collect_pub_item_regions,
    find_baseline,
    http_path_accepted,
    load_baseline,
    scan_provider_hits,
    surface_finding,
)
from rust_scan import iter_code_lines

GATE_003 = "GATE-DEP-003"
GATE_004 = "GATE-DEP-004"
POLICY_003 = "DB1 §9–§12 / §19; forbidden public provider re-exports"
POLICY_004 = "DB1 §7–§12 / §19; stable façade provider-type leakage"


def _check_reexports(root: Path, baseline, findings: list[Finding]) -> None:
    for cf in build_crate_index(root):
        if cf.kind != "prod":
            continue
        source = cf.abs_path.read_text(encoding="utf-8", errors="replace")
        for h in scan_provider_hits(source):
            if h.kind == "core_tls_reexport":
                be = find_baseline(
                    baseline,
                    provider="rustls",
                    file=cf.rel_path,
                    symbol_or_pattern="pub use exyonq_mod_tls as tls",
                    kinds=("reexport",),
                )
                line_txt = next((t for ln, t in iter_code_lines(source) if ln == h.line), "")
                if be and "pub use exyonq_mod_tls as tls" in " ".join(line_txt.split()):
                    findings.append(
                        surface_finding(
                            gate_id=GATE_003,
                            severity=SEVERITY_WARN,
                            status=STATUS_KNOWN_DEBT,
                            provider="rustls",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="core::tls re-export known policy violation (exact baseline)",
                            policy_source=be.policy_reference,
                            suggested_action="Do not add further TLS/rustls re-exports via core",
                            baseline_id=be.id,
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE_003,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="rustls",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="core::tls-style re-export not matching exact baseline",
                            policy_source=POLICY_003,
                            suggested_action="Remove re-export; do not expand core::tls debt",
                        )
                    )
                continue

            if h.kind not in {"pub_use", "extern_crate"}:
                continue
            if h.provider not in CRITICAL_PROVIDERS:
                continue
            if h.visibility != "pub":
                continue
            be = find_baseline(
                baseline,
                provider=h.provider,
                file=cf.rel_path,
                symbol_or_pattern=h.path,
                kinds=("reexport", "pub_use"),
            )
            if be:
                findings.append(
                    surface_finding(
                        gate_id=GATE_003,
                        severity=SEVERITY_WARN,
                        status=STATUS_KNOWN_DEBT,
                        provider=h.provider,
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message=f"baselined public re-export ({be.id})",
                        policy_source=be.policy_reference,
                        suggested_action="Do not expand re-export surface",
                        baseline_id=be.id,
                    )
                )
            else:
                findings.append(
                    surface_finding(
                        gate_id=GATE_003,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider=h.provider,
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message="forbidden public re-export of critical provider",
                        policy_source=POLICY_003,
                        suggested_action="Make private/pub(crate) or move behind ExyonQ type",
                    )
                )


def _check_pub_types(root: Path, baseline, findings: list[Finding]) -> None:
    """
    Conservative source-level check:
    - DBEX-005 BoxBody files (baseline vs expansion)
    - Stable façades + module-api public signatures with provider paths
    - DBEX-003 TcpStream accepted on static_wire
    Limitations documented in DB2C2 report (no rustdoc-json).
    """
    facade = STABLE_FACADE_CRATES | {"exyonq-module-api"}
    boxbody_reported: set[str] = set()
    dbex003_reported = False

    for cf in build_crate_index(root):
        if cf.kind != "prod":
            continue
        source = cf.abs_path.read_text(encoding="utf-8", errors="replace")

        if "BoxBody" in source and "hyper::Error" in source:
            be = find_baseline(
                baseline,
                provider="hyper",
                file=cf.rel_path,
                symbol_or_pattern="BoxBody",
                kinds=("pub_type",),
            )
            if cf.rel_path not in boxbody_reported:
                boxbody_reported.add(cf.rel_path)
                if be:
                    findings.append(
                        surface_finding(
                            gate_id=GATE_004,
                            severity=SEVERITY_WARN,
                            status=STATUS_KNOWN_DEBT,
                            provider="hyper",
                            file=cf.rel_path,
                            line=1,
                            symbol="BoxBody",
                            message=f"DBEX-005 BoxBody/hyper::Error surface ({be.id})",
                            policy_source=be.policy_reference,
                            suggested_action="Do not add BoxBody surfaces in new files",
                            baseline_id=be.id,
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE_004,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="hyper",
                            file=cf.rel_path,
                            line=1,
                            symbol="BoxBody",
                            message="new BoxBody/hyper::Error surface outside DBEX-005 baseline files",
                            policy_source=POLICY_004 + "; DBEX-005",
                            suggested_action="Remove or obtain authorized baseline expansion (not by analogy)",
                        )
                    )

        if cf.crate not in facade:
            continue

        for line, vis, item_kind, sig in collect_pub_item_regions(source):
            if vis != "pub":
                continue
            for m in _PATH_RE.finditer(sig):
                path = m.group(0)
                provider = ROOT_TO_PROVIDER.get(m.group(1))
                if not provider:
                    continue
                if provider == "bytes" and bytes_path_accepted(path):
                    continue
                if provider == "http" and http_path_accepted(path):
                    continue
                if provider == "tokio" and "TcpStream" in path:
                    be = find_baseline(
                        baseline,
                        provider="tokio",
                        file=cf.rel_path,
                        symbol_or_pattern="tokio::net::TcpStream",
                        kinds=("pub_type",),
                    )
                    if be and not dbex003_reported:
                        dbex003_reported = True
                        findings.append(
                            surface_finding(
                                gate_id=GATE_004,
                                severity=SEVERITY_WARN,
                                status=STATUS_ACCEPTED,
                                provider="tokio",
                                file=cf.rel_path,
                                line=line,
                                symbol=path,
                                message=f"DBEX-003 accepted TcpStream surface ({be.id})",
                                policy_source=be.policy_reference,
                                suggested_action="Do not expand beyond static_wire TcpStream",
                                baseline_id=be.id,
                            )
                        )
                    elif not be:
                        findings.append(
                            surface_finding(
                                gate_id=GATE_004,
                                severity=SEVERITY_ERROR,
                                status=STATUS_VIOLATION,
                                provider="tokio",
                                file=cf.rel_path,
                                line=line,
                                symbol=path,
                                message="Tokio type on public façade outside DBEX-003 baseline",
                                policy_source=POLICY_004 + "; DBEX-003",
                                suggested_action="Remove or confine to static_wire TcpStream",
                            )
                        )
                    continue

                if provider not in CRITICAL_PROVIDERS and provider != "anyhow":
                    continue
                # BoxBody handled at file level
                if "BoxBody" in sig:
                    continue
                be = find_baseline(
                    baseline,
                    provider=provider,
                    file=cf.rel_path,
                    symbol_or_pattern=path,
                    kinds=("pub_type", "error", "import"),
                )
                if be:
                    findings.append(
                        surface_finding(
                            gate_id=GATE_004,
                            severity=SEVERITY_WARN,
                            status=STATUS_KNOWN_DEBT,
                            provider=provider,
                            file=cf.rel_path,
                            line=line,
                            symbol=path,
                            message=f"baselined public provider type ({be.id})",
                            policy_source=be.policy_reference,
                            suggested_action="Do not expand",
                            baseline_id=be.id,
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE_004,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=provider,
                            file=cf.rel_path,
                            line=line,
                            symbol=path,
                            message=f"provider type in public {item_kind} on stable façade",
                            policy_source=POLICY_004,
                            suggested_action="Wrap behind ExyonQ type or restrict visibility",
                        )
                    )


def run(root: Path) -> list[Finding]:
    baseline = load_baseline(root)
    findings: list[Finding] = []
    _check_reexports(root, baseline, findings)
    _check_pub_types(root, baseline, findings)
    findings.sort(key=lambda f: (f.gate_id, f.file, f.line, f.dependency, f.symbol))
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
                "gate": "GATE-DEP-003+004",
                "exit": worst_exit(findings),
                "findings": [f.to_dict() for f in findings],
            }
        )
    else:
        emit_human([f for f in findings if f.gate_id == GATE_003], "GATE-DEP-003: public provider re-exports")
        emit_human([f for f in findings if f.gate_id == GATE_004], "GATE-DEP-004: public provider types (façades)")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
