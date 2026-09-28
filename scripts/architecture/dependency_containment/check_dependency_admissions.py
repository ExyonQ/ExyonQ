#!/usr/bin/env python3
"""GATE-DEP-012 — Dependency admission presence (compact registry)."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from common import (
    EXIT_CONFIG,
    Finding,
    SEVERITY_ERROR,
    emit_human,
    emit_json,
    find_repo_root,
    inventory_direct_externals,
    load_admissions,
    worst_exit,
)

GATE = "GATE-DEP-012"
POLICY = "DB1 §20 + dependency-admissions.toml"

REQUIRED = (
    "dependency",
    "problem",
    "category",
    "owner",
    "authorized_zones",
    "public_type_impact",
    "error_model",
    "hot_path_impact",
    "version_policy",
    "status",
)

ALLOWED_STATUS = {"accepted", "temporary", "review-required", "build-test-only"}


def run(root: Path) -> list[Finding]:
    admissions = load_admissions(root)
    occ = inventory_direct_externals(root)
    observed = sorted({o.package for o in occ})
    findings: list[Finding] = []

    for pkg in observed:
        row = admissions.get(pkg)
        if not row:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file="docs/architecture/dependency-boundaries/dependency-admissions.toml",
                    section="[[admission]]",
                    message="missing admission entry for direct dependency",
                    policy_source=POLICY,
                    suggested_action="Add compact [[admission]] row covering required fields",
                )
            )
            continue
        for field in REQUIRED:
            if field not in row or row[field] in (None, "", []):
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        dependency=pkg,
                        file="docs/architecture/dependency-boundaries/dependency-admissions.toml",
                        section=field,
                        message=f"admission missing required field {field!r}",
                        policy_source=POLICY,
                        suggested_action="Fill required admission fields",
                    )
                )
        st = row.get("status")
        if st and st not in ALLOWED_STATUS:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file="docs/architecture/dependency-boundaries/dependency-admissions.toml",
                    section="status",
                    message=f"invalid admission status {st!r}",
                    policy_source=POLICY,
                    suggested_action=f"Use one of {sorted(ALLOWED_STATUS)}",
                )
            )
        zones = row.get("authorized_zones") or []
        if isinstance(zones, list) and pkg in observed:
            # basic coherence: at least one observed crate should appear in zones
            crates = {o.crate for o in occ if o.package == pkg}
            if zones and crates.isdisjoint(set(zones)):
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        dependency=pkg,
                        file="docs/architecture/dependency-boundaries/dependency-admissions.toml",
                        section="authorized_zones",
                        message=f"admission zones {zones} disjoint from observed crates {sorted(crates)}",
                        policy_source=POLICY,
                        suggested_action="Align authorized_zones with actual consuming crates",
                    )
                )

    for pkg in sorted(admissions.keys()):
        if pkg not in observed:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file="docs/architecture/dependency-boundaries/dependency-admissions.toml",
                    section="[[admission]]",
                    message="admission entry for dependency not observed in workspace manifests",
                    policy_source=POLICY,
                    suggested_action="Remove stale admission or restore dependency",
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
        emit_human(findings, f"{GATE}: admission presence")
    return worst_exit(findings)


if __name__ == "__main__":
    sys.exit(main())
