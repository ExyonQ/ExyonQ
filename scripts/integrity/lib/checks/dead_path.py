#!/usr/bin/env python3
"""Check 3 — dead / unreachable feature path heuristics."""
from __future__ import annotations

import re
from pathlib import Path
from typing import Dict, List, Set

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

CFG_FEATURE_RE = re.compile(r'#\[cfg\(.*?feature\s*=\s*"([^"]+)".*?\)\]')


def run(ctx: ScanContext) -> List[Finding]:
    """Approximate reachability: public modules never referenced outside their crate.

    This is intentionally HEURISTIC. Confidence is MEDIUM/LOW — never claims
    formal unreachable proof.
    """
    findings: List[Finding] = []
    root = ctx.root

    # Collect feature flags declared in workspace Cargo.toml files
    declared_features: Dict[str, str] = {}
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        if not rel.endswith("Cargo.toml"):
            continue
        text = read_text(path)
        in_features = False
        for i, line in enumerate(text.splitlines(), 1):
            if line.strip() == "[features]":
                in_features = True
                continue
            if in_features and line.startswith("["):
                in_features = False
            if in_features:
                m = re.match(r"^([A-Za-z0-9_-]+)\s*=", line)
                if m and m.group(1) not in {"default"}:
                    declared_features[m.group(1)] = f"{rel}:{i}"

    # Features referenced in source
    used_features: Set[str] = set()
    for path in iter_files(root, None):
        rel = rel_of(root, path)
        if not rel.endswith(".rs"):
            continue
        text = read_text(path)
        for m in CFG_FEATURE_RE.finditer(text):
            used_features.add(m.group(1))

    for feat, loc in sorted(declared_features.items()):
        if feat in used_features or feat in {"std", "default"}:
            continue
        # Many crates declare optional features consumed only via deps — REVIEW not FAIL
        path, _, line_s = loc.partition(":")
        findings.append(
            Finding(
                id=ctx.next_id("DEAD"),
                severity="REVIEW",
                category="DEAD_UNREACHABLE_PATH",
                path=path,
                line=int(line_s or "1"),
                claim=f"Feature flag '{feat}' has a declared consumer in source",
                evidence=f"declared at {loc}; no cfg(feature=...) hit in scanned .rs",
                why_it_matters=(
                    "Unused feature flags may hide unfinished work or imply optional "
                    "capabilities that never activate."
                ),
                confidence="LOW",
                recommended_action="Confirm feature is consumed via deps or remove/document as UNVERIFIED.",
                classification="REVIEW_REQUIRED",
                check="dead_path",
            )
        )

    # Heuristic: modules under product prefixes named *stub* / *dummy* / *fake*
    stub_re = re.compile(r"(^|/)(stub|dummy|fake|mock|placeholder)([_/]|$)", re.I)
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        zone = ctx.classify(rel).zone
        if zone != "PRODUCT":
            continue
        if stub_re.search(rel) and rel.endswith(".rs"):
            findings.append(
                Finding(
                    id=ctx.next_id("DEAD"),
                    severity="HIGH",
                    category="DEAD_UNREACHABLE_PATH",
                    path=rel,
                    line=1,
                    claim="Product path does not contain stub/fake/mock modules",
                    evidence=f"filename suggests non-real implementation: {Path(rel).name}",
                    why_it_matters="Stub modules on product path often substitute for real behavior.",
                    confidence="MEDIUM",
                    recommended_action="Move to tests/ or replace with real implementation + claim update.",
                    classification="PRODUCT_PATH_FAKE",
                    check="dead_path",
                )
            )

    # Binary/workspace membership note — informational when no [[bin]]/workspace members drift
    ws = root / "Cargo.toml"
    if ws.is_file():
        text = read_text(ws)
        if "[workspace]" not in text:
            findings.append(
                Finding(
                    id=ctx.next_id("DEAD"),
                    severity="INFO",
                    category="DEAD_UNREACHABLE_PATH",
                    path="Cargo.toml",
                    line=1,
                    claim="Workspace manifest present for crate graph analysis",
                    evidence="[workspace] section not found in root Cargo.toml",
                    why_it_matters="Without workspace metadata, reachability analysis is weaker.",
                    confidence="HIGH",
                    recommended_action="Ensure auditor runs on workspace root.",
                    classification="REVIEW_REQUIRED",
                    check="dead_path",
                )
            )

    return findings
