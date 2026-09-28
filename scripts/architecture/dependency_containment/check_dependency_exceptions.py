#!/usr/bin/env python3
"""GATE-DEP-006 — DBEX allowlist validation."""

from __future__ import annotations

import argparse
import re
import sys
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
    STATUS_VIOLATION,
    load_baseline,
    surface_finding,
)

GATE = "GATE-DEP-006"
POLICY = "DB1 §19; dependency-exceptions.toml"
REQUIRED_IDS = [f"DBEX-{i:03d}" for i in range(1, 9)]
VALID_STATUS = {"accepted", "temporary", "rejected", "superseded"}
BROAD_SCOPE_RE = re.compile(
    r"(^|\s)(core/\*\*|all modules|\*\*/\*|\ball\b\s+crates)(\s|$)",
    re.I,
)
REQUIRED_FIELDS = (
    "id",
    "dependency",
    "status",
    "exact_scope",
    "allowed_type_or_pattern",
    "owner",
    "policy_reference",
    "performance_reason",
    "security_impact",
    "replacement_required",
    "review_trigger",
    "expansion_allowed",
    "test_or_bench_evidence",
)


def load_exceptions(root: Path) -> list[dict[str, Any]]:
    path = root / "docs/architecture/dependency-boundaries/dependency-exceptions.toml"
    data = load_toml(path)
    rows = data.get("exception", [])
    if not isinstance(rows, list):
        raise SystemExit(f"{EXIT_CONFIG}: {path}: [[exception]] missing")
    return rows


def run(root: Path) -> list[Finding]:
    findings: list[Finding] = []
    try:
        rows = load_exceptions(root)
        surface = load_baseline(root)
    except SystemExit:
        raise

    by_id: dict[str, dict[str, Any]] = {}
    for row in rows:
        eid = row.get("id")
        if not eid:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider="",
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol="",
                    message="exception row missing id",
                    policy_source=POLICY,
                    suggested_action="Add id = DBEX-NNN",
                )
            )
            continue
        if eid in by_id:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row.get("dependency", "")),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=str(eid),
                    message=f"duplicate exception id {eid}",
                    policy_source=POLICY,
                    suggested_action="Keep exactly one row per DBEX id",
                    exception_id=str(eid),
                )
            )
            continue
        by_id[str(eid)] = row

    # Required coverage
    for rid in REQUIRED_IDS:
        if rid not in by_id:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider="",
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=rid,
                    message=f"required DB1 exception {rid} missing",
                    policy_source=POLICY,
                    suggested_action="Add the DBEX row from DB1 §19",
                    exception_id=rid,
                )
            )

    # Unknown IDs
    for eid, row in sorted(by_id.items()):
        if eid not in REQUIRED_IDS and not str(row.get("policy_reference", "")).startswith("DB1"):
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row.get("dependency", "")),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message=f"unknown exception id {eid} without normative DB1 reference",
                    policy_source=POLICY,
                    suggested_action="Remove or cite DB1 section authorizing it",
                    exception_id=eid,
                )
            )

        missing = [f for f in REQUIRED_FIELDS if f not in row]
        if missing:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row.get("dependency", "")),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message=f"missing required fields: {', '.join(missing)}",
                    policy_source=POLICY,
                    suggested_action="Complete DBEX schema fields",
                    exception_id=eid,
                )
            )
            continue

        status = str(row["status"]).lower()
        if status not in VALID_STATUS:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message=f"invalid status {status!r}",
                    policy_source=POLICY,
                    suggested_action="Use accepted|temporary|rejected|superseded",
                    exception_id=eid,
                )
            )

        scope = str(row.get("exact_scope") or "").strip()
        if not scope:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message="exact_scope is empty",
                    policy_source=POLICY,
                    suggested_action="Provide concrete scope from DB1",
                    exception_id=eid,
                )
            )
        elif BROAD_SCOPE_RE.search(scope) and eid not in {"DBEX-001", "DBEX-002", "DBEX-004"}:
            # DB1 expressly uses owner-table / workspace wording for 001/002/004
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message=f"exact_scope too broad: {scope!r}",
                    policy_source=POLICY,
                    suggested_action="Narrow to concrete files/symbols as in DB1",
                    exception_id=eid,
                )
            )

        if row.get("expansion_allowed") is True:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message="expansion_allowed=true without authorizing policy revision",
                    policy_source=POLICY + "; EXISTING_DEBT != AUTHORIZATION_TO_EXPAND",
                    suggested_action="Keep expansion_allowed = false",
                    exception_id=eid,
                )
            )

        if status == "temporary" and not str(row.get("review_trigger") or "").strip():
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message="TEMPORARY exception missing review_trigger",
                    policy_source=POLICY,
                    suggested_action="Add review_trigger from DB1",
                    exception_id=eid,
                )
            )

        if status == "accepted" and not str(row.get("owner") or "").strip():
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=str(row["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message="ACCEPTED exception missing owner",
                    policy_source=POLICY,
                    suggested_action="Set owner from DB1",
                    exception_id=eid,
                )
            )

    # Coherence with provider-surface baseline
    known_dbex = set(by_id)
    for be in surface:
        refs = re.findall(r"DBEX-\d{3}", be.policy_reference + " " + be.id)
        for rid in refs:
            if rid not in known_dbex:
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider=be.provider,
                        file=be.file,
                        line=0,
                        symbol=be.id,
                        message=f"provider-surface baseline references missing exception {rid}",
                        policy_source=POLICY,
                        suggested_action="Add DBEX row or fix baseline reference",
                        baseline_id=be.id,
                        exception_id=rid,
                    )
                )

    # Hot-path baseline exception_id must resolve to a known DBEX (N18 / coherence)
    hp_path = root / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml"
    if hp_path.is_file():
        try:
            hp = load_toml(hp_path)
        except SystemExit:
            raise
        for row in hp.get("pattern", []) or []:
            if not isinstance(row, dict):
                continue
            eid = str(row.get("exception_id") or "").strip()
            if not eid:
                continue
            if eid not in known_dbex:
                findings.append(
                    surface_finding(
                        gate_id=GATE,
                        severity=SEVERITY_ERROR,
                        status=STATUS_VIOLATION,
                        provider=str(row.get("pattern_class") or ""),
                        file=str(row.get("file") or hp_path.name),
                        line=0,
                        symbol=str(row.get("id") or ""),
                        message=f"hot-path baseline references missing exception {eid}",
                        policy_source=POLICY,
                        suggested_action="Fix exception_id or add DBEX row",
                        baseline_id=str(row.get("id") or ""),
                        exception_id=eid,
                    )
                )

    # DBEX-003 scope: only static_wire
    if "DBEX-003" in by_id:
        scope = str(by_id["DBEX-003"]["exact_scope"])
        if "static_wire" not in scope:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider="tokio",
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol="DBEX-003",
                    message="DBEX-003 exact_scope must name module-api/src/static_wire.rs",
                    policy_source=POLICY,
                    suggested_action="Restore DB1 exact scope",
                    exception_id="DBEX-003",
                )
            )
        # INFO: accepted exact
        findings.append(
            surface_finding(
                gate_id=GATE,
                severity=SEVERITY_INFO,
                status="ACCEPTED",
                provider="tokio",
                file="module-api/src/static_wire.rs",
                line=0,
                symbol="tokio::net::TcpStream",
                message="DBEX-003 scope validated (static_wire TcpStream only)",
                policy_source=POLICY,
                suggested_action="Do not expand",
                exception_id="DBEX-003",
            )
        )

    # DBEX-005 / DBEX-008 presence markers
    for eid, msg in (
        ("DBEX-005", "TEMPORARY BoxBody/hyper::Error exception present"),
        ("DBEX-006", "TEMPORARY socket2 dual-version exception present"),
        ("DBEX-008", "TEMPORARY wasm-host Wasmtime leaf exception present"),
    ):
        if eid in by_id:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_WARN,
                    status=STATUS_KNOWN_DEBT if by_id[eid]["status"] == "temporary" else "ACCEPTED",
                    provider=str(by_id[eid]["dependency"]),
                    file="docs/architecture/dependency-boundaries/dependency-exceptions.toml",
                    line=0,
                    symbol=eid,
                    message=msg,
                    policy_source=str(by_id[eid]["policy_reference"]),
                    suggested_action="Honor review_trigger; no expansion",
                    exception_id=eid,
                )
            )

    # Sort: errors, review, known debt, info
    order = {"ERROR": 0, "WARN": 1, "INFO": 2}
    findings.sort(
        key=lambda f: (
            order.get(f.severity, 9),
            0 if f.status == STATUS_VIOLATION else 1 if f.status == "REVIEW_REQUIRED" else 2 if f.status == STATUS_KNOWN_DEBT else 3,
            f.exception_id,
            f.file,
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
        emit_human(findings, "GATE-DEP-006: DBEX allowlist validation")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
