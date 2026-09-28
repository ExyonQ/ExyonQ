#!/usr/bin/env python3
"""GATE-DEP-002 — Authorized zones (source / import layer)."""

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
    load_owner_registry,
    worst_exit,
)
from provider_common import (
    STATUS_KNOWN_DEBT,
    STATUS_VIOLATION,
    build_crate_index,
    bytes_path_accepted,
    effective_file_kind,
    find_baseline,
    http_path_accepted,
    load_baseline,
    scan_provider_hits,
    surface_finding,
)

GATE = "GATE-DEP-002"
POLICY = "DB1 §6/§24 + dependency-owner-registry.toml; source/import layer"


def run(root: Path) -> list[Finding]:
    registry = load_owner_registry(root)
    baseline = load_baseline(root)
    findings: list[Finding] = []
    seen: set[tuple[str, str, int, str]] = set()

    for cf in build_crate_index(root):
        source = cf.abs_path.read_text(encoding="utf-8", errors="replace")
        hits = scan_provider_hits(source)
        for h in hits:
            if h.kind in {"core_tls_reexport", "pub_use", "extern_crate"}:
                # Re-exports handled by GATE-DEP-003 / 011
                continue
            key = (cf.rel_path, h.provider, h.line, h.path)
            if key in seen:
                continue
            seen.add(key)

            row = registry.get(h.provider)
            if not row:
                continue
            owners = set(row.get("authorized_owners") or [])
            debt = set(row.get("known_debt_owners") or [])
            forbidden = set(row.get("forbidden_zones") or [])
            kind = effective_file_kind(cf, source, h.line)

            # Shared accepted value types
            if h.provider == "bytes" and bytes_path_accepted(h.path):
                continue
            if h.provider == "http" and http_path_accepted(h.path):
                if cf.crate in owners or kind in {"test", "bench", "example", "fuzz"}:
                    continue

            if cf.crate in forbidden and kind == "prod":
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider=h.provider,
                        file=cf.rel_path,
                        line=h.line,
                        symbol=h.path,
                        message=f"provider import in forbidden zone crate {cf.crate}",
                        policy_source=POLICY + "; forbidden_zones",
                        suggested_action="Remove import or obtain DB1 policy change",
                    )
                )
                continue

            if cf.crate in debt:
                be = find_baseline(
                    baseline,
                    provider=h.provider,
                    file=cf.rel_path,
                    symbol_or_pattern=h.path,
                    kinds=("import", "any"),
                )
                if be is None:
                    # debt owner but pattern not baselined → expansion
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_ERROR,
                            status=STATUS_VIOLATION,
                            provider=h.provider,
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message=(
                                "known_debt_owner import not covered by exact baseline "
                                "(EXISTING_DEBT != AUTHORIZATION_TO_EXPAND)"
                            ),
                            policy_source=POLICY,
                            suggested_action="Do not expand debt; remediate or authorize new baseline row",
                        )
                    )
                else:
                    findings.append(
                        surface_finding(
                            gate_id=GATE,
                            severity=SEVERITY_WARN,
                            status=STATUS_KNOWN_DEBT,
                            provider=h.provider,
                            file=cf.rel_path,
                            line=h.line,
                            symbol=h.path,
                            message=f"known debt import ({be.id})",
                            policy_source=be.policy_reference,
                            suggested_action="Do not expand; schedule remediation",
                            baseline_id=be.id,
                        )
                    )
                continue

            if cf.crate in owners:
                continue

            # test/bench/fuzz of non-owners still fail for critical containment
            # unless crate is owner — already handled. Allow test crates listed as owners only.
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=h.provider,
                    file=cf.rel_path,
                    line=h.line,
                    symbol=h.path,
                    message=f"import of {h.provider} in unauthorized crate {cf.crate} ({kind})",
                    policy_source=POLICY,
                    suggested_action="Move usage to an authorized owner or update DB1 registry",
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
        emit_human(findings, "GATE-DEP-002: authorized zones (source/import)")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
