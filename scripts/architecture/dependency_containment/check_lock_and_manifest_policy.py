#!/usr/bin/env python3
"""GATE-DEP-009 — Manifest and Cargo.lock policy (objective checks)."""

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
    lock_packages,
    resolve_lock_for_manifest,
    worst_exit,
)

GATE = "GATE-DEP-009"
POLICY = "DB1 §21 + dependency-owner-registry.toml"


def run(root: Path) -> list[Finding]:
    try:
        registry = load_owner_registry(root)
    except SystemExit:
        raise
    occ = inventory_direct_externals(root)
    findings: list[Finding] = []

    # Cache locks by path
    locks: dict[Path, dict[str, set[str]]] = {}

    def lock_for(manifest_rel: str) -> dict[str, set[str]]:
        lp = resolve_lock_for_manifest(root, manifest_rel)
        if lp not in locks:
            locks[lp] = lock_packages(lp)
        return locks[lp]

    root_lock = lock_packages(root / "Cargo.lock")
    if not root_lock:
        findings.append(
            Finding(
                gate_id=GATE,
                severity=SEVERITY_ERROR,
                dependency="",
                file="Cargo.lock",
                section="",
                message="Cargo.lock missing or unreadable",
                policy_source=POLICY,
                suggested_action="Restore versioned Cargo.lock",
            )
        )
        return findings

    for o in occ:
        lock = lock_for(o.manifest)
        lock_label = str(resolve_lock_for_manifest(root, o.manifest).relative_to(root))

        if o.star:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section,
                    message="version requirement is '*' (forbidden)",
                    policy_source=POLICY,
                    suggested_action="Pin an explicit SemVer requirement and update registry",
                )
            )
        if o.git:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section,
                    message="git dependency without authorized machine-readable exception",
                    policy_source=POLICY,
                    suggested_action="Use crates.io/workspace pin or add an authorized exception in a later phase",
                )
            )
        if o.path_external:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section,
                    message="path dependency outside workspace members",
                    policy_source=POLICY,
                    suggested_action="Vendoring/path overrides require explicit policy authorization",
                )
            )

        if not o.git and o.package not in lock:
            findings.append(
                Finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    dependency=o.package,
                    file=o.manifest,
                    section=o.section,
                    message=f"direct dependency package not present in {lock_label}",
                    policy_source=POLICY,
                    suggested_action="Ensure the relevant lockfile is committed and resolves this package",
                )
            )

        row = registry.get(o.package) or {}
        pin = row.get("strategic_exact_pin")
        if pin:
            vers = lock.get(o.package, set())
            if vers and pin not in vers:
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        dependency=o.package,
                        file=lock_label,
                        section="[[package]]",
                        message=f"strategic exact pin {pin!r} not among locked versions {sorted(vers)}",
                        policy_source=POLICY,
                        suggested_action="Restore exact pin per DB1 / Wasmtime policy",
                    )
                )

        allowed = row.get("allowed_lock_versions")
        if allowed:
            # Dual-version allowlist is evaluated against the root workspace lock
            # (socket2 appears in the main graph, not fuzz).
            vers = root_lock.get(o.package, set())
            unexpected = sorted(vers - set(allowed))
            if unexpected:
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        dependency=o.package,
                        file="Cargo.lock",
                        section="[[package]]",
                        message=f"lock versions outside temporary allowlist {allowed}: {unexpected}",
                        policy_source=POLICY + "; DBEX-006",
                        suggested_action="Do not expand dual-version set without policy review",
                    )
                )
            missing = sorted(set(allowed) - vers)
            if missing:
                findings.append(
                    Finding(
                        gate_id=GATE,
                        severity=SEVERITY_WARN,
                        dependency=o.package,
                        file="Cargo.lock",
                        section="[[package]]",
                        message=f"expected temporary dual versions missing from lock: {missing}",
                        policy_source=POLICY + "; DBEX-006",
                        suggested_action="Confirm target resolution; full DBEX validation in DB2C3",
                    )
                )

    direct_pkgs = {o.package for o in occ}
    for name, vers in sorted((n, sorted(vs)) for n, vs in root_lock.items() if len(vs) > 1):
        if name == "socket2" or name not in direct_pkgs:
            continue
        findings.append(
            Finding(
                gate_id=GATE,
                severity=SEVERITY_WARN,
                dependency=name,
                file="Cargo.lock",
                section="duplicate",
                message=f"direct dependency has multiple locked versions: {vers} (not auto-fail)",
                policy_source=POLICY + "; GATE-DEP-008 deferred detail",
                suggested_action="Review only if security/ABI/size concern; DB2C3 owns duplicate policy",
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
        emit_human(findings, f"{GATE}: manifest/lock policy")
    return worst_exit(findings)


if __name__ == "__main__":
    sys.exit(main())
