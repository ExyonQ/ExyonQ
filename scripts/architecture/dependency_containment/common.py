#!/usr/bin/env python3
# Copyright 2026 Antonio Cantallops Alba — Apache-2.0
"""Shared helpers for DB2C1 dependency containment checkers (stdlib only)."""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    print("dependency_containment: Python 3.11+ with tomllib required", file=sys.stderr)
    sys.exit(2)


SEVERITY_ERROR = "ERROR"
SEVERITY_WARN = "WARN"
SEVERITY_INFO = "INFO"

EXIT_PASS = 0
EXIT_VIOLATION = 1
EXIT_CONFIG = 2


@dataclass(frozen=True)
class Finding:
    gate_id: str
    severity: str
    dependency: str
    file: str
    section: str
    message: str
    policy_source: str
    suggested_action: str
    # DB2C2/DB2C3 surface fields (optional; empty for DB2C1 findings)
    status: str = ""
    line: int = 0
    symbol: str = ""
    baseline_id: str = ""
    exception_id: str = ""

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        # Stable JSON: drop empty optional surface fields for DB2C1 compactness
        if not d.get("status"):
            d.pop("status", None)
            d.pop("line", None)
            d.pop("symbol", None)
            d.pop("baseline_id", None)
            d.pop("exception_id", None)
        elif not d.get("exception_id"):
            d.pop("exception_id", None)
        return d


@dataclass
class DepOccurrence:
    package: str
    dep_key: str
    crate: str
    manifest: str
    section: str
    target: str
    version: str
    workspace: bool
    git: bool
    star: bool
    path_external: bool


def find_repo_root(start: Path | None = None) -> Path:
    cur = (start or Path.cwd()).resolve()
    for p in [cur, *cur.parents]:
        if (p / "Cargo.toml").is_file() and (p / "docs" / "architecture").is_dir():
            return p
    raise SystemExit(f"{EXIT_CONFIG}: cannot locate repo root from {cur}")


def load_toml(path: Path) -> dict[str, Any]:
    try:
        return tomllib.loads(path.read_text(encoding="utf-8"))
    except Exception as exc:  # noqa: BLE001 — surface as config failure
        raise SystemExit(f"{EXIT_CONFIG}: invalid TOML {path}: {exc}") from exc


def workspace_member_paths(root: Path) -> list[Path]:
    ws = load_toml(root / "Cargo.toml")
    members = list(ws.get("workspace", {}).get("members", []))
    # fuzz is a separate Cargo workspace in this repo; still inventory directs.
    fuzz = root / "fuzz"
    if fuzz.is_dir() and (fuzz / "Cargo.toml").is_file() and "fuzz" not in members:
        members.append("fuzz")
    out: list[Path] = []
    for m in members:
        p = root / m / "Cargo.toml"
        if p.is_file():
            out.append(p)
    return sorted(out)


def workspace_path_packages(root: Path) -> set[str]:
    """Package names that are internal path/workspace members."""
    ws = load_toml(root / "Cargo.toml")
    names: set[str] = set()
    for name, spec in ws.get("workspace", {}).get("dependencies", {}).items():
        if isinstance(spec, dict) and spec.get("path"):
            names.add(name)
            if "package" in spec:
                names.add(spec["package"])
    for manifest in workspace_member_paths(root):
        data = load_toml(manifest)
        pkg = data.get("package", {}).get("name")
        if pkg:
            names.add(pkg)
    return names


def _is_internal(name: str, spec: Any, ws_deps: dict[str, Any], path_pkgs: set[str]) -> bool:
    """True for workspace members / path members — not for external path deps."""
    if isinstance(spec, dict):
        if "path" in spec:
            # Only workspace member packages are internal path deps.
            pkg = str(spec.get("package") or name)
            return name in path_pkgs or pkg in path_pkgs
        if spec.get("workspace"):
            base = ws_deps.get(name, {})
            if isinstance(base, dict) and base.get("path"):
                pkg = str(base.get("package") or name)
                return name in path_pkgs or pkg in path_pkgs
    if name.startswith("exyonq-"):
        base = ws_deps.get(name, {})
        if isinstance(base, dict) and base.get("path"):
            return name in path_pkgs or str(base.get("package") or name) in path_pkgs
        if isinstance(spec, dict) and spec.get("workspace") and isinstance(base, dict) and base.get("path"):
            return True
    return False


def _package_name(name: str, spec: Any, ws_deps: dict[str, Any]) -> str:
    if isinstance(spec, dict) and "package" in spec:
        return str(spec["package"])
    if isinstance(spec, dict) and spec.get("workspace"):
        base = ws_deps.get(name, {})
        if isinstance(base, dict) and "package" in base:
            return str(base["package"])
    return name


def _version_and_flags(name: str, spec: Any, ws_deps: dict[str, Any]) -> tuple[str, bool, bool, bool, bool]:
    """Returns version, workspace, git, star, path_external."""
    workspace = False
    git = False
    star = False
    path_external = False
    version = ""

    def from_dict(d: dict[str, Any]) -> None:
        nonlocal version, git, star, path_external
        if "git" in d:
            git = True
            version = f"git:{d['git']}"
        elif "version" in d:
            version = str(d["version"])
            star = version == "*"
        if "path" in d:
            # caller filters internal paths; remaining path = external
            path_external = True

    if isinstance(spec, str):
        version = spec
        star = version == "*"
    elif isinstance(spec, dict):
        if spec.get("workspace"):
            workspace = True
            base = ws_deps.get(name, {})
            if isinstance(base, str):
                version = base
                star = version == "*"
            elif isinstance(base, dict):
                from_dict(base)
        from_dict(spec)
    return version, workspace, git, star, path_external


def iter_dep_tables(data: dict[str, Any]) -> list[tuple[str, str, dict[str, Any]]]:
    """Yield (section, target_cfg, deps_dict). target_cfg '' for non-target."""
    out: list[tuple[str, str, dict[str, Any]]] = []
    for sec in ("dependencies", "dev-dependencies", "build-dependencies"):
        deps = data.get(sec)
        if isinstance(deps, dict):
            out.append((sec, "", deps))
    for tkey, tval in data.get("target", {}).items():
        if not isinstance(tval, dict):
            continue
        for sec in ("dependencies", "dev-dependencies", "build-dependencies"):
            deps = tval.get(sec)
            if isinstance(deps, dict):
                out.append((sec, str(tkey), deps))
    return out


def inventory_direct_externals(root: Path) -> list[DepOccurrence]:
    ws = load_toml(root / "Cargo.toml")
    ws_deps = ws.get("workspace", {}).get("dependencies", {})
    path_pkgs = workspace_path_packages(root)
    occ: list[DepOccurrence] = []

    for manifest in workspace_member_paths(root):
        data = load_toml(manifest)
        crate = data.get("package", {}).get("name", manifest.parent.name)
        rel = str(manifest.relative_to(root))
        for section, target, deps in iter_dep_tables(data):
            for name, spec in deps.items():
                if _is_internal(name, spec, ws_deps, path_pkgs):
                    continue
                pkg = _package_name(name, spec, ws_deps)
                # Skip workspace path members referenced by key
                if pkg in path_pkgs and isinstance(spec, dict) and (
                    "path" in spec
                    or (
                        spec.get("workspace")
                        and isinstance(ws_deps.get(name), dict)
                        and ws_deps[name].get("path")
                    )
                ):
                    continue
                if name.startswith("exyonq-") and isinstance(ws_deps.get(name), dict) and ws_deps[name].get("path"):
                    if isinstance(spec, dict) and spec.get("workspace"):
                        continue
                ver, workspace, git, star, path_ext = _version_and_flags(name, spec, ws_deps)
                # External path deps (non-workspace) are flagged separately
                if path_ext and not (
                    isinstance(spec, dict)
                    and "path" in spec
                    and _is_internal(name, spec, ws_deps, path_pkgs)
                ):
                    # if still here with path, it's unexpected external path
                    pass
                occ.append(
                    DepOccurrence(
                        package=pkg,
                        dep_key=name,
                        crate=crate,
                        manifest=rel,
                        section=section,
                        target=target,
                        version=ver,
                        workspace=workspace,
                        git=git,
                        star=star,
                        path_external=bool(
                            isinstance(spec, dict)
                            and "path" in spec
                            and not _is_internal(name, spec, ws_deps, path_pkgs)
                        ),
                    )
                )
    return sorted(occ, key=lambda o: (o.package, o.crate, o.section, o.target, o.manifest))


def load_owner_registry(root: Path) -> dict[str, dict[str, Any]]:
    path = root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
    data = load_toml(path)
    rows = data.get("dependency", [])
    if not isinstance(rows, list):
        raise SystemExit(f"{EXIT_CONFIG}: {path}: [[dependency]] missing")
    by_name: dict[str, dict[str, Any]] = {}
    for row in rows:
        name = row.get("name")
        if not name or name in by_name:
            raise SystemExit(f"{EXIT_CONFIG}: duplicate or missing dependency name in registry: {name!r}")
        by_name[name] = row
    return by_name


def load_admissions(root: Path) -> dict[str, dict[str, Any]]:
    path = root / "docs/architecture/dependency-boundaries/dependency-admissions.toml"
    data = load_toml(path)
    rows = data.get("admission", [])
    if not isinstance(rows, list):
        raise SystemExit(f"{EXIT_CONFIG}: {path}: [[admission]] missing")
    by_name: dict[str, dict[str, Any]] = {}
    for row in rows:
        name = row.get("dependency")
        if not name or name in by_name:
            raise SystemExit(f"{EXIT_CONFIG}: duplicate or missing admission dependency: {name!r}")
        by_name[name] = row
    return by_name


def lock_packages(lock_path: Path) -> dict[str, set[str]]:
    """Map package name -> set of versions in a Cargo.lock (simple line parser)."""
    if not lock_path.is_file():
        return {}
    text = lock_path.read_text(encoding="utf-8")
    packages: dict[str, set[str]] = defaultdict(set)
    name = None
    for line in text.splitlines():
        if line.startswith("name = "):
            name = line.split("=", 1)[1].strip().strip('"')
        elif line.startswith("version = ") and name:
            ver = line.split("=", 1)[1].strip().strip('"')
            packages[name].add(ver)
            name = None
        elif line.startswith("[[package]]"):
            name = None
    return packages


def resolve_lock_for_manifest(root: Path, manifest_rel: str) -> Path:
    """Nested Cargo workspaces (e.g. fuzz/) use their own Cargo.lock."""
    man = Path(manifest_rel)
    # Walk up from manifest dir looking for Cargo.lock beside a [workspace] root.
    cur = (root / man).parent
    while True:
        candidate = cur / "Cargo.lock"
        cargo_toml = cur / "Cargo.toml"
        if candidate.is_file() and cargo_toml.is_file():
            return candidate
        if cur == root or cur.parent == cur:
            break
        cur = cur.parent
    return root / "Cargo.lock"

def emit_human(findings: list[Finding], title: str) -> None:
    print(title)
    if not findings:
        print("  (no findings)")
        return
    for f in findings:
        loc = f"{f.file}"
        if f.line:
            loc += f":{f.line}"
        elif f.section:
            loc += f":{f.section}"
        status = f" status={f.status}" if f.status else ""
        sym = f" sym={f.symbol}" if f.symbol else ""
        base = f" baseline={f.baseline_id}" if f.baseline_id else ""
        exc = f" exception={f.exception_id}" if f.exception_id else ""
        print(
            f"  [{f.severity}] {f.gate_id} dep={f.dependency}{status}{sym} {loc}{base}{exc}\n"
            f"    {f.message}\n"
            f"    policy: {f.policy_source}\n"
            f"    action: {f.suggested_action}"
        )


def emit_json(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, indent=2, sort_keys=True))


def worst_exit(findings: list[Finding]) -> int:
    if any(f.severity == SEVERITY_ERROR for f in findings):
        return EXIT_VIOLATION
    return EXIT_PASS


def explain_db2c1() -> str:
    return """DB2C1+DB2C2 dependency containment gates
Policy: docs/architecture/dependency-boundaries/DEPENDENCY_CONTAINMENT_POLICY.md
Spec:   docs/architecture/dependency-boundaries/DEPENDENCY_MACHINE_ENFORCEMENT_SPECIFICATION.md
Rule:   .cursor/rules/125-dependency-containment.mdc
Baseline: docs/architecture/dependency-boundaries/dependency-provider-surface-baseline.toml

DB2C1:
  GATE-DEP-001  Direct dependency registry coverage
  GATE-DEP-002  Authorized zones (manifest layer)
  GATE-DEP-009  Manifest/lock policy (*, git, strategic pins, registry integrity)
  GATE-DEP-010  PS3A D1 (existing verify-ps3a-platform-linux-d1.sh)
  GATE-DEP-012  Admission registry presence + required fields

DB2C2:
  GATE-DEP-002  Authorized zones (source/import layer)
  GATE-DEP-003  Forbidden public provider re-exports
  GATE-DEP-004  Stable façade provider-type leakage (conservative source parser)
  GATE-DEP-005  Provider error containment on stable façades
  GATE-DEP-011  Critical provider owner rules (Tokio/Rustls/Quinn/Wasmtime/Hyper/socket2/libc)

DB2C3:
  GATE-DEP-006  DBEX allowlist validation (dependency-exceptions.toml)
  GATE-DEP-007  Hot-path pattern budget (dependency-hot-path-baseline.toml)
  GATE-DEP-008  Duplicate dependency report (cargo tree -d)

Exit codes: 0=PASS 1=POLICY_VIOLATION 2=CONFIGURATION_OR_TOOL_FAILURE
EXISTING_DEBT != AUTHORIZATION_TO_EXPAND
Known debt must match baseline exactly; expansion fails closed.
"""
