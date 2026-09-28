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
    load_toml,
    worst_exit,
)

GATE = "GATE-DEP-001"
POLICY = "DB1 §5/§24 + dependency-owner-registry.toml"
REGISTRY_FILE = "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
REQUIRED_FIELDS = (
    "name",
    "class",
    "authorized_owners",
    "authorized_public_surface",
    "forbidden_zones",
    "error_policy",
    "hot_path_policy",
    "version_policy",
    "status",
)
ALLOWED_CLASSES = frozenset({"A", "B", "C", "D", "E", "F", "I"})
ALLOWED_STATUS = frozenset({"BINDING", "KNOWN_DEBT", "TEMPORARY"})


def run(root: Path) -> list[Finding]:
    registry_path = root / REGISTRY_FILE
    raw = load_toml(registry_path)
    try:
        registry = load_owner_registry(root)
    except SystemExit:
        raise
    occ = inventory_direct_externals(root)
    observed = sorted({o.package for o in occ})
    registered = sorted(registry.keys())
    findings: list[Finding] = []

    if raw.get("schema_version") != 1:
        findings.append(
            Finding(
                gate_id=GATE,
                severity=SEVERITY_ERROR,
                dependency="",
                file=REGISTRY_FILE,
                section="schema_version",
                message="owner registry must declare schema_version = 1",
                policy_source=POLICY,
                suggested_action="Restore the versioned registry schema",
            )
        )

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
            continue

        row = registry[pkg]
        for field in REQUIRED_FIELDS:
            if field not in row or row[field] in (None, ""):
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        dependency=pkg,
                        file=REGISTRY_FILE,
                        section=field,
                        message=f"registry row missing required field {field!r}",
                        policy_source=POLICY,
                        suggested_action="Complete the dependency policy row with a real value",
                    )
                )

        owners = row.get("authorized_owners")
        forbidden = row.get("forbidden_zones")
        if not isinstance(owners, list) or not owners:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="authorized_owners",
                    message="authorized_owners must be a non-empty explicit crate list",
                    policy_source=POLICY,
                    suggested_action="List the direct manifest owners; wildcards and empty lists are forbidden",
                )
            )
            owners = []
        if not isinstance(forbidden, list):
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="forbidden_zones",
                    message="forbidden_zones must be an explicit list",
                    policy_source=POLICY,
                    suggested_action="Use an empty or concrete crate-name list, never a wildcard",
                )
            )
            forbidden = []

        expected = {o.crate for o in occ if o.package == pkg}
        actual = set(owners)
        if actual != expected:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="authorized_owners",
                    message=(
                        f"owner registry differs from direct manifest owners: "
                        f"registered={sorted(actual)} observed={sorted(expected)}"
                    ),
                    policy_source=POLICY,
                    suggested_action="Remove blanket owners or reconcile the actual manifest dependency",
                )
            )
        if "*" in actual or "*" in set(forbidden):
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="authorized_owners/forbidden_zones",
                    message="wildcard dependency zones are forbidden",
                    policy_source=POLICY,
                    suggested_action="Name every owner and forbidden zone explicitly",
                )
            )
        overlap = actual & set(forbidden)
        if overlap:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="forbidden_zones",
                    message=f"crate cannot be both authorized and forbidden: {sorted(overlap)}",
                    policy_source=POLICY,
                    suggested_action="Resolve the contradictory dependency policy",
                )
            )
        debt = set(row.get("known_debt_owners") or [])
        if not debt <= actual:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="known_debt_owners",
                    message=f"known debt owners must be authorized direct owners: {sorted(debt - actual)}",
                    policy_source=POLICY,
                    suggested_action="Remove stale debt owners or restore the direct manifest edge",
                )
            )
        if row.get("class") not in ALLOWED_CLASSES:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="class",
                    message=f"unknown dependency class {row.get('class')!r}",
                    policy_source=POLICY,
                    suggested_action=f"Use one of {sorted(ALLOWED_CLASSES)}",
                )
            )
        if row.get("status") not in ALLOWED_STATUS:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
                    section="status",
                    message=f"unknown registry status {row.get('status')!r}",
                    policy_source=POLICY,
                    suggested_action=f"Use one of {sorted(ALLOWED_STATUS)}",
                )
            )

    for pkg in registered:
        if pkg not in observed:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=pkg,
                    file=REGISTRY_FILE,
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
