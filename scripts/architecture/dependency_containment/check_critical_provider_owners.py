#!/usr/bin/env python3
"""GATE-DEP-011 — critical provider owner rules."""

from __future__ import annotations

import argparse
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
    STATUS_ACCEPTED,
    STATUS_KNOWN_DEBT,
    STATUS_VIOLATION,
    build_crate_index,
    effective_file_kind,
    find_baseline,
    load_baseline,
    scan_provider_hits,
    surface_finding,
)
from rust_scan import iter_code_lines

GATE = "GATE-DEP-011"
POLICY = "DB1 §10–§13 / §19; critical provider ownership"


def run(root: Path) -> list[Finding]:
    baseline = load_baseline(root)
    findings: list[Finding] = []
    seen: set[tuple] = set()
    # Collapse noisy ACCEPTED/KNOWN_DEBT repeats to one row per (file, provider, status, baseline)
    accepted_once: set[tuple[str, str, str, str]] = set()

    def add(f: Finding) -> None:
        if f.status in {STATUS_ACCEPTED, STATUS_KNOWN_DEBT} and f.baseline_id:
            k = (f.file, f.dependency, f.status, f.baseline_id)
            if k in accepted_once:
                return
            accepted_once.add(k)
        key = (f.file, f.line, f.dependency, f.symbol, f.status, f.message)
        if key in seen:
            return
        seen.add(key)
        findings.append(f)

    for cf in build_crate_index(root):
        source = cf.abs_path.read_text(encoding="utf-8", errors="replace")
        hits = scan_provider_hits(source)

        for h in hits:
            kind = effective_file_kind(cf, source, h.line)

            # --- Tokio / DBEX-003 ---
            if h.provider == "tokio" and cf.crate == "exyonq-module-api" and kind == "prod":
                if cf.rel_path == "module-api/src/static_wire.rs" and "TcpStream" in h.path:
                    be = find_baseline(
                        baseline,
                        provider="tokio",
                        file=cf.rel_path,
                        symbol_or_pattern="tokio::net::TcpStream",
                        kinds=("pub_type",),
                    )
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_WARN,
                            status=STATUS_ACCEPTED,
                            provider="tokio",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="DBEX-003 accepted Tokio TcpStream in static_wire",
                            policy_source=(be.policy_reference if be else POLICY + "; DBEX-003"),
                            suggested_action="Do not expand Tokio types beyond this seam",
                            baseline_id=be.id if be else "",
                        )
                    )
                else:
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="tokio",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="Tokio usage in module-api outside DBEX-003 exact seam",
                            policy_source=POLICY + "; DBEX-003",
                            suggested_action="Keep only tokio::net::TcpStream in static_wire.rs",
                        )
                    )

            # --- Rustls ---
            if h.provider in {"rustls", "tokio-rustls"}:
                if h.kind == "core_tls_reexport":
                    be = find_baseline(
                        baseline,
                        provider="rustls",
                        file=cf.rel_path,
                        symbol_or_pattern="pub use exyonq_mod_tls as tls",
                        kinds=("reexport",),
                    )
                    if be:
                        add(
                            surface_finding(
                                gate_id=GATE,
                                severity=SEVERITY_WARN,
                                status=STATUS_KNOWN_DEBT,
                                provider="rustls",
                                file=cf.rel_path,
                                line=h.line,
                                symbol=h.path,
                                message="core::tls known debt (exact)",
                                policy_source=be.policy_reference,
                                suggested_action="No additional TLS re-exports",
                                baseline_id=be.id,
                            )
                        )
                    else:
                        add(
                            surface_finding(
                                gate_id=GATE,
                                severity=SEVERITY_ERROR,
                                status=STATUS_VIOLATION,
                                provider="rustls",
                                file=cf.rel_path,
                                line=h.line,
                                symbol=h.path,
                                message="TLS re-export without exact baseline",
                                policy_source=POLICY,
                                suggested_action="Remove re-export",
                            )
                        )
                elif cf.crate == "exyonq-mod-tls":
                    continue
                elif cf.crate == "exyonq-core" and kind in {"test", "bench"}:
                    continue
                elif cf.crate == "exyonq-core" and kind == "prod":
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=h.provider,
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="direct rustls/tokio-rustls in core prod outside baselined re-export",
                            policy_source=POLICY,
                            suggested_action="Keep TLS behind exyonq-mod-tls; do not expand core debt",
                        )
                    )
                elif cf.crate not in {"exyonq-mod-tls"} and kind == "prod":
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=h.provider,
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message=f"{h.provider} outside authorized TLS owner",
                            policy_source=POLICY,
                            suggested_action="Route through exyonq-mod-tls",
                        )
                    )

            # --- Quinn / H3 ---
            if h.provider in {"quinn", "h3", "h3-quinn", "quinn-proto"}:
                if cf.crate == "exyonq-mod-http3":
                    continue
                if cf.crate == "exyonq-core" and kind in {"test", "bench", "dev"}:
                    continue
                if kind == "prod":
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=h.provider,
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="Quinn/H3 usage outside exyonq-mod-http3 (prod)",
                            policy_source=POLICY,
                            suggested_action="Keep H3 stack inside mod-http3",
                        )
                    )

            # --- Wasmtime ---
            if h.provider == "wasmtime":
                if cf.crate == "exyonq-wasm-host":
                    be = find_baseline(
                        baseline,
                        provider="wasmtime",
                        file=cf.rel_path,
                        symbol_or_pattern="wasmtime::",
                        kinds=("import",),
                    )
                    if be:
                        add(
                            surface_finding(
                                gate_id=GATE,
                                severity=SEVERITY_WARN,
                                status=STATUS_ACCEPTED,
                                provider="wasmtime",
                                file=cf.rel_path,
                                line=h.line,
                                symbol=h.path,
                                message="DBEX-008 wasm-host leaf usage",
                                policy_source=be.policy_reference,
                                suggested_action="Do not wire into core/module-api/runtime-plan",
                                baseline_id=be.id,
                            )
                        )
                    continue
                if cf.crate in {"exyonq-core", "exyonq-module-api", "exyonq-runtime-plan"}:
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="wasmtime",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="wasmtime usage in core/module-api/runtime-plan forbidden",
                            policy_source=POLICY + "; DBEX-008",
                            suggested_action="Keep Wasmtime in wasm-host until capsule admission",
                        )
                    )
                elif kind == "prod":
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="wasmtime",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="wasmtime outside wasm-host owner",
                            policy_source=POLICY,
                            suggested_action="Move to exyonq-wasm-host",
                        )
                    )

            # --- Hyper ---
            if h.provider == "hyper":
                if cf.crate == "exyonq-config-ir":
                    be = find_baseline(
                        baseline,
                        provider="hyper",
                        file=cf.rel_path,
                        symbol_or_pattern=h.path,
                        kinds=("import",),
                    )
                    if be:
                        add(
                            surface_finding(
                                gate_id=GATE,
                                severity=SEVERITY_WARN,
                                status=STATUS_KNOWN_DEBT,
                                provider="hyper",
                                file=cf.rel_path,
                                line=h.line,
                                symbol=h.path,
                                message="hyper in config-ir known debt (exact)",
                                policy_source=be.policy_reference,
                                suggested_action="Do not add further hyper usage in config/*",
                                baseline_id=be.id,
                            )
                        )
                    else:
                        add(
                            surface_finding(
                                gate_id=GATE,
                                severity=SEVERITY_ERROR,
                                status=STATUS_VIOLATION,
                                provider="hyper",
                                file=cf.rel_path,
                                line=h.line,
                                symbol=h.path,
                                message="hyper usage in config-ir not on exact baseline pattern",
                                policy_source=POLICY,
                                suggested_action="Remove expansion of config-ir hyper debt",
                            )
                        )
                elif cf.rel_path.startswith("config/") and cf.crate != "exyonq-config-ir":
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="hyper",
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message="new hyper usage in config/ control plane outside known debt",
                            policy_source=POLICY,
                            suggested_action="Keep hyper out of config control plane",
                        )
                    )

            # --- socket2 / libc public re-export ---
            if h.provider in {"socket2", "libc"} and h.kind == "pub_use" and h.visibility == "pub":
                add(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider=h.provider,
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message=f"public re-export of {h.provider} forbidden",
                        policy_source=POLICY,
                        suggested_action="Keep OS types private to owners/seams",
                    )
                )

        # Extra: expansion of core::tls — any additional pub use of mod_tls
        if cf.rel_path != "core/src/lib.rs":
            for ln, text in iter_code_lines(source):
                if "pub use exyonq_mod_tls" in text or (
                    "pub use" in text and "as tls" in text and "mod_tls" in text
                ):
                    add(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider="rustls",
                            file=cf.rel_path,
                            line=ln,
                            symbol="tls",
                            message="additional TLS crate re-export beyond core::tls baseline",
                            policy_source=POLICY,
                            suggested_action="Remove re-export",
                        )
                    )

    findings.sort(key=lambda f: (f.file, f.line, f.dependency, f.symbol, f.status))
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
        emit_human(findings, "GATE-DEP-011: critical provider owners")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
