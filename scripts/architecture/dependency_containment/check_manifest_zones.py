#!/usr/bin/env python3
"""GATE-DEP-002 — Authorized dependency zones (manifest layer only)."""

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
    inventory_direct_externals,
    load_owner_registry,
    worst_exit,
)

GATE = "GATE-DEP-002"
POLICY = "DB1 §6/§24 + dependency-owner-registry.toml authorized_owners"


def run(root: Path) -> list[Finding]:
    registry = load_owner_registry(root)
    occ = inventory_direct_externals(root)
    findings: list[Finding] = []

    for o in occ:
        row = registry.get(o.package)
        if not row:
            # GATE-001 owns unregistered; skip duplicate noise
            continue
        owners = set(row.get("authorized_owners") or [])
        debt = set(row.get("known_debt_owners") or [])
        if o.crate not in owners:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section + (f"@{o.target}" if o.target else ""),
                    message=f"crate {o.crate!r} not in authorized_owners for {o.package}",
                    policy_source=POLICY,
                    suggested_action="Update DB1/owner registry or remove the dependency from this crate",
                )
            )
            continue
        if o.crate in debt:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_WARN,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section + (f"@{o.target}" if o.target else ""),
                    message=(
                        f"known_debt_owners hit: {o.crate} uses {o.package} "
                        "(EXISTING_DEBT != AUTHORIZATION_TO_EXPAND)"
                    ),
                    policy_source=POLICY + "; DB1 known policy debts",
                    suggested_action="Do not expand this debt; schedule remediation under a later authorized phase",
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
        print(str(e), file=sys.stderr)
        return EXIT_CONFIG

    if args.json:
        emit_json({"gate": GATE, "findings": [f.to_dict() for f in findings], "exit": worst_exit(findings)})
    else:
        emit_human(findings, f"{GATE}: authorized zones (manifest)")
    return worst_exit(findings)


if __name__ == "__main__":
    sys.exit(main())
