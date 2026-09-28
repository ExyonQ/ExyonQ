#!/usr/bin/env python3
"""Check 4 — configuration effectiveness heuristics."""
from __future__ import annotations

import re
from typing import Dict, List, Set, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

# serde field patterns in Rust config structs
FIELD_RE = re.compile(
    r"""^\s*(?:#\[.*?\]\s*)*pub\s+([a-z][a-z0-9_]*)\s*:\s*""",
    re.M,
)
SERDE_RENAME = re.compile(r'#\[serde\([^)]*rename\s*=\s*"([^"]+)"[^)]*\)\]')


def run(ctx: ScanContext) -> List[Finding]:
    """Detect config fields that appear parsed but never read outside definition files.

    Stages (reported in evidence):
      CONFIG_DECLARED → CONFIG_PARSED → CONFIG_PROPAGATED → CONFIG_CONSUMED
    Behavior verification is never claimed from static scan alone.
    """
    findings: List[Finding] = []
    root = ctx.root

    # Collect candidate config field names from known config locations
    declared: Dict[str, Tuple[str, int]] = {}
    config_globs = (
        "core/src/",
        "crates/",
        "modules/",
        "config/",
        "compat/",
    )
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        if not rel.endswith(".rs"):
            continue
        if not any(rel.startswith(p) for p in config_globs):
            continue
        if "config" not in rel.lower() and "/cfg/" not in rel and "settings" not in rel.lower():
            continue
        text = read_text(path)
        # Only structs that look like config
        if "Deserialize" not in text and "serde" not in text:
            continue
        lines = text.splitlines()
        for i, line in enumerate(lines, 1):
            m = re.match(r"^\s*pub\s+([a-z][a-z0-9_]*)\s*:", line)
            if not m:
                continue
            name = m.group(1)
            if name in {"id", "name", "path", "enabled", "version"}:
                continue
            declared[name] = (rel, i)

    if not declared:
        findings.append(
            Finding(
                id=ctx.next_id("CFG"),
                severity="INFO",
                category="CONFIG_EFFECTIVENESS",
                path=".",
                line=0,
                claim="Config field inventory available",
                evidence="No serde config fields matched heuristics in scoped paths",
                why_it_matters="Without declared fields, effectiveness cannot be scored.",
                confidence="LOW",
                recommended_action="Extend claims file for critical config options.",
                classification="REVIEW_REQUIRED",
                check="config_effect",
            )
        )
    else:
        # Build corpus of non-definition references
        corpus_hits: Dict[str, int] = {k: 0 for k in declared}
        for path in iter_files(root, None):
            rel = rel_of(root, path)
            if not rel.endswith(".rs"):
                continue
            zone = ctx.classify(rel).zone
            if zone in {"DOC", "FIXTURE"}:
                continue
            text = read_text(path)
            for name in declared:
                if re.search(rf"\.{re.escape(name)}\b", text):
                    decl_path, _ = declared[name]
                    if rel == decl_path:
                        if len(re.findall(rf"\.{re.escape(name)}\b", text)) > 1:
                            corpus_hits[name] += 1
                    else:
                        corpus_hits[name] += text.count(f".{name}")

        # Report fields with zero external reads — REVIEW (heuristic; low confidence)
        unused = [n for n, hits in corpus_hits.items() if hits == 0]
        for name in sorted(unused)[:40]:
            rel, line = declared[name]
            findings.append(
                Finding(
                    id=ctx.next_id("CFG"),
                    severity="REVIEW",
                    category="CONFIG_EFFECTIVENESS",
                    path=rel,
                    line=line,
                    claim=f"Config field '{name}' reaches CONFIG_CONSUMED",
                    evidence=(
                        f"CONFIG_DECLARED=YES CONFIG_PARSED=LIKELY CONFIG_PROPAGATED=UNKNOWN "
                        f"CONFIG_CONSUMED=UNPROVEN CONFIG_BEHAVIOR_VERIFIED=NO "
                        f"(no '.{name}' reads outside declaration heuristic)"
                    ),
                    why_it_matters=(
                        "CONFIG_ACCEPTED != CONFIG_EFFECTIVE. Parsed-but-ignored options "
                        "create false confidence that operators control behavior."
                    ),
                    confidence="LOW",
                    recommended_action=(
                        "Trace field to runtime decision site or mark claim UNVERIFIED/DEFERRED."
                    ),
                    classification="REVIEW_REQUIRED",
                    check="config_effect",
                )
            )

    # Strong signal: explicit "parsed but unused" markers (also used by selftest fixtures)
    marker = re.compile(r"CONFIG_PARSED_BUT_UNUSED|parsed\s+but\s+never\s+used", re.I)
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        text = read_text(path)
        for i, line in enumerate(text.splitlines(), 1):
            if marker.search(line):
                findings.append(
                    Finding(
                        id=ctx.next_id("CFG"),
                        severity="HIGH",
                        category="CONFIG_EFFECTIVENESS",
                        path=rel,
                        line=i,
                        claim="No explicit parsed-but-unused configuration",
                        evidence=line.strip()[:200],
                        why_it_matters="Explicit unused-config markers indicate known effectiveness gaps.",
                        confidence="HIGH",
                        recommended_action="Wire config into runtime or remove the option.",
                        classification="PRODUCT_PATH_FAKE",
                        check="config_effect",
                    )
                )
    return findings
