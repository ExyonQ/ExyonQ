#!/usr/bin/env python3
"""Check 12 — dependency present != feature integrated."""
from __future__ import annotations

import re
from typing import Dict, List, Set, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

# Strategic deps that often get mistaken for integrated features
WATCH = {
    "wasmtime": ("wasm", "wasm-host", "exyonq-wasm"),
    "quinn": ("http3", "quic", "mod-http3"),
    "h3": ("http3", "mod-http3"),
    "rustls": ("tls", "mod-tls"),
    "brotli": ("compression", "brotli"),
    "async-compression": ("compression",),
}


def _deps_from_cargo(text: str) -> Set[str]:
    deps: Set[str] = set()
    section = None
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            section = s.strip("[]")
            continue
        if section in {"dependencies", "dev-dependencies", "build-dependencies", "workspace.dependencies"} or (
            section and section.endswith("dependencies")
        ):
            m = re.match(r"^([A-Za-z0-9_-]+)\s*=", s)
            if m:
                deps.add(m.group(1).replace("_", "-"))
    return deps


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    root = ctx.root
    declared: Dict[str, str] = {}

    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        if not rel.endswith("Cargo.toml"):
            continue
        deps = _deps_from_cargo(read_text(path))
        for d in deps:
            declared[d] = rel

    # Usage via extern/use/crate names in rs
    used: Set[str] = set()
    for path in iter_files(root, None):
        rel = rel_of(root, path)
        if not rel.endswith(".rs"):
            continue
        text = read_text(path)
        for d in WATCH:
            crate = d.replace("-", "_")
            if re.search(rf"\b(?:use|extern\s+crate)\s+{crate}\b|{crate}::", text):
                used.add(d)

    for dep, hints in WATCH.items():
        if dep not in declared and dep.replace("_", "-") not in declared:
            continue
        loc = declared.get(dep) or declared.get(dep.replace("_", "-"))
        if dep not in used:
            findings.append(
                Finding(
                    id=ctx.next_id("DEP"),
                    severity="REVIEW",
                    category="DEPENDENCY_FEATURE_REALITY",
                    path=loc or "Cargo.toml",
                    line=1,
                    claim=f"Dependency '{dep}' is integrated into product path",
                    evidence=(
                        f"declared in {loc}; no use/extern/{dep.replace('-', '_')}:: hit in .rs scan; "
                        f"hints={','.join(hints)}"
                    ),
                    why_it_matters="Dependency present != feature integrated/runtime-connected.",
                    confidence="LOW",
                    recommended_action="Confirm wiring + behavior test or remove unused dep.",
                    classification="REVIEW_REQUIRED",
                    check="dependency_reality",
                )
            )
        else:
            findings.append(
                Finding(
                    id=ctx.next_id("DEP"),
                    severity="INFO",
                    category="DEPENDENCY_FEATURE_REALITY",
                    path=loc or "Cargo.toml",
                    line=1,
                    claim=f"Dependency '{dep}' has source references",
                    evidence="use/extern/path references found (not proof of runtime feature completeness)",
                    why_it_matters="Source reference is necessary but not sufficient for VERIFIED claims.",
                    confidence="MEDIUM",
                    recommended_action="Bind to feature-claims with behavior evidence.",
                    classification="REVIEW_REQUIRED",
                    check="dependency_reality",
                )
            )
    return findings
