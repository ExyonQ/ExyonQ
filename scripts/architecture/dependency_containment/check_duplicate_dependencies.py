#!/usr/bin/env python3
"""GATE-DEP-008 — duplicate dependency report (cargo tree -d)."""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

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
    STATUS_REVIEW,
    STATUS_VIOLATION,
    surface_finding,
)

GATE = "GATE-DEP-008"
POLICY = "DB1 §19 DBEX-006 / §21; cargo tree -d"
PKG_LINE = re.compile(r"^([a-zA-Z0-9_-]+) v([0-9][^ ]*)")
SOCKET2_ALLOWED = frozenset({"0.5.10", "0.6.4"})
# Strategic packages where unexpected extra versions are ERROR
STRATEGIC = frozenset({"socket2", "wasmtime", "tokio", "hyper", "rustls", "quinn"})


def load_socket2_policy(root: Path) -> frozenset[str]:
    """Prefer DBEX-006 / owner registry allowed_lock_versions when present."""
    reg = root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
    data = load_toml(reg)
    for row in data.get("dependency", []):
        if row.get("name") == "socket2":
            allowed = row.get("allowed_lock_versions")
            if isinstance(allowed, list) and allowed:
                return frozenset(str(x) for x in allowed)
    return SOCKET2_ALLOWED


def run_cargo_tree_d(root: Path, timeout: int = 120) -> str:
    env = os.environ.copy()
    env["CARGO_TERM_COLOR"] = "never"
    try:
        proc = subprocess.run(
            ["cargo", "tree", "-d", "--target", "all", "--prefix", "none"],
            cwd=root,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=env,
            check=False,
        )
    except FileNotFoundError as exc:
        raise SystemExit(f"{EXIT_CONFIG}: cargo not found on PATH") from exc
    except subprocess.TimeoutExpired as exc:
        raise SystemExit(f"{EXIT_CONFIG}: cargo tree -d timed out after {timeout}s") from exc
    if proc.returncode != 0:
        err = (proc.stderr or proc.stdout or "").strip().splitlines()
        tail = err[-3:] if err else ["(no stderr)"]
        raise SystemExit(
            f"{EXIT_CONFIG}: cargo tree -d failed (exit {proc.returncode}): " + " | ".join(tail)
        )
    return proc.stdout


def parse_duplicates(text: str) -> dict[str, set[str]]:
    pkgs: dict[str, set[str]] = defaultdict(set)
    for line in text.splitlines():
        m = PKG_LINE.match(line.strip())
        if m:
            pkgs[m.group(1)].add(m.group(2))
    return {k: v for k, v in pkgs.items() if len(v) > 1}


def run(root: Path) -> list[Finding]:
    allowed_socket2 = load_socket2_policy(root)
    text = run_cargo_tree_d(root)
    dups = parse_duplicates(text)
    findings: list[Finding] = []

    # socket2 special
    sock_vers = dups.get("socket2") or set()
    # Also check lock even if cargo tree omits
    if not sock_vers:
        lock = (root / "Cargo.lock").read_text(encoding="utf-8")
        sock_vers = set(re.findall(r'name = "socket2"\nversion = "([^"]+)"', lock))

    if sock_vers:
        unexpected = sorted(sock_vers - allowed_socket2)
        missing = sorted(allowed_socket2 - sock_vers)
        if unexpected:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider="socket2",
                    file="Cargo.lock",
                    line=0,
                    symbol=",".join(sorted(sock_vers)),
                    message=f"socket2 versions outside DBEX-006 allowlist: {unexpected}",
                    policy_source=POLICY,
                    suggested_action="Do not add a third socket2 version; restore DBEX-006 pins",
                    exception_id="DBEX-006",
                )
            )
        elif len(sock_vers) > 1:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_WARN,
                    status=STATUS_KNOWN_DEBT,
                    provider="socket2",
                    file="Cargo.lock",
                    line=0,
                    symbol="+".join(sorted(sock_vers)),
                    message=(
                        f"socket2 dual-version {sorted(sock_vers)} "
                        f"(STATUS=KNOWN_DEBT EXCEPTION=DBEX-006 EXPANSION_ALLOWED=NO)"
                    ),
                    policy_source=POLICY,
                    suggested_action="Unification candidate later; do not expand",
                    exception_id="DBEX-006",
                )
            )
        if missing and not unexpected:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_WARN,
                    status=STATUS_REVIEW,
                    provider="socket2",
                    file="Cargo.lock",
                    line=0,
                    symbol=",".join(sorted(sock_vers)),
                    message=f"expected DBEX-006 versions missing from graph: {missing}",
                    policy_source=POLICY,
                    suggested_action="Confirm target resolution; do not silently drop dual-version",
                    exception_id="DBEX-006",
                )
            )

    for name, vers in sorted(dups.items()):
        if name == "socket2" or name.startswith("exyonq"):
            continue
        if name.startswith("windows"):
            # platform triples — INFO only
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_INFO,
                    status="INFO",
                    provider=name,
                    file="Cargo.lock",
                    line=0,
                    symbol="+".join(sorted(vers)),
                    message=f"platform duplicate versions: {sorted(vers)}",
                    policy_source=POLICY,
                    suggested_action="Informational; no action unless ABI concern",
                )
            )
            continue
        if name in STRATEGIC:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_ERROR,
                    status=STATUS_VIOLATION,
                    provider=name,
                    file="Cargo.lock",
                    line=0,
                    symbol="+".join(sorted(vers)),
                    message=f"strategic package has multiple versions: {sorted(vers)}",
                    policy_source=POLICY,
                    suggested_action="Investigate pin/features; unauthorized dual-version",
                )
            )
        else:
            findings.append(
                surface_finding(
                    gate_id=GATE,
                    severity=SEVERITY_WARN,
                    status=STATUS_REVIEW,
                    provider=name,
                    file="Cargo.lock",
                    line=0,
                    symbol="+".join(sorted(vers)),
                    message=f"duplicate dependency versions: {sorted(vers)} (not auto-fail)",
                    policy_source=POLICY,
                    suggested_action="Review only if security/ABI/size concern",
                )
            )

    if not findings:
        findings.append(
            surface_finding(
                gate_id=GATE,
                severity=SEVERITY_INFO,
                status="INFO",
                provider="",
                file="Cargo.lock",
                line=0,
                symbol="",
                message="no duplicate packages reported by cargo tree -d",
                policy_source=POLICY,
                suggested_action="None",
            )
        )

    findings.sort(
        key=lambda f: (
            0 if f.severity == "ERROR" else 1 if f.status == STATUS_REVIEW else 2 if f.status == STATUS_KNOWN_DEBT else 3,
            f.dependency,
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
        emit_human(findings, "GATE-DEP-008: duplicate dependency report")
    return worst_exit(findings)


if __name__ == "__main__":
    raise SystemExit(main())
