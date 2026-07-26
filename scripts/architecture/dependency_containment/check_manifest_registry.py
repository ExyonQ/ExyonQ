#!/usr/bin/env python3
"""GATE-DEP-001 — Direct dependency registry coverage."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from common import (
    EXIT_CONFIG,
    EXIT_PASS,
    Finding,
    SEVERITY_ERROR,
    emit_human,
    emit_json,
    find_repo_root,
    inventory_direct_externals,
    load_owner_registry,
    worst_exit,
)

GATE = "GATE-DEP-001"
POLICY = "DB1 §5/§24 + dependency-owner-registry.toml"


def run(root: Path) -> list[Finding]:
    try:
        registry = load_owner_registry(root)
    except SystemExit:
        raise
    occ = inventory_direct_externals(root)
    observed = sorted({o.package for o in occ})
    registered = sorted(registry.keys())
    findings: list[Finding] = []

    for pkg in observed:
        if pkg not in registry:
            samples = [o for o in occ if o.package == pkg][:3]
            where = "; ".join(f"{s.manifest}[{s.section}]" for s in samples)
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=samples[0].manifest if samples else "",
                    section=samples[0].section if samples else "",
                    message=f"direct external dependency not in owner registry (seen at: {where})",
                    policy_source=POLICY,
                    suggested_action="Add [[dependency]] row to dependency-owner-registry.toml and DB1 owner table before merge",
                )
            )

    for pkg in registered:
        if pkg not in observed:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file="docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                    section="[[dependency]]",
                    message="registry row has no corresponding direct workspace dependency",
                    policy_source=POLICY,
                    suggested_action="Remove stale registry row or restore the dependency intentionally",
                )
            )

    # duplicate registry names already rejected at load
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

    occ = inventory_direct_externals(root)
    observed = sorted({o.package for o in occ})
    payload = {
        "gate": GATE,
        "observed_count": len(observed),
        "observed": observed,
        "findings": [f.to_dict() for f in findings],
        "exit": worst_exit(findings),
    }
    if args.json:
        emit_json(payload)
    else:
        emit_human(findings, f"{GATE}: registry coverage (observed={len(observed)})")
    return worst_exit(findings)


if __name__ == "__main__":
    sys.exit(main())
