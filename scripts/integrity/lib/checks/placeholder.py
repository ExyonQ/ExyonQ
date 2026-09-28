#!/usr/bin/env python3
"""Check 2 — placeholder / incomplete code."""
from __future__ import annotations

import re
from typing import List, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

# (name, pattern, product_severity, test_severity, classification)
RULES: List[Tuple[str, re.Pattern[str], str, str, str]] = [
    (
        "TODO_MACRO",
        re.compile(r"\btodo!\s*\("),
        "HIGH",
        "LOW",
        "PRODUCT_PATH_FAKE",
    ),
    (
        "UNIMPLEMENTED_MACRO",
        re.compile(r"\bunimplemented!\s*\("),
        "CRITICAL",
        "MEDIUM",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "PANIC_NOT_IMPLEMENTED",
        re.compile(r"""panic!\s*\(\s*["'][^"']*(?:not\s+implemented|TODO|FIXME)""", re.I),
        "CRITICAL",
        "MEDIUM",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "TODO_FIXME_COMMENT",
        re.compile(r"(?://|#|/\*)\s*(TODO|FIXME)\b"),
        "MEDIUM",
        "INFO",
        "REVIEW_REQUIRED",
    ),
    (
        "NOT_IMPLEMENTED_TOKEN",
        # Exclude HTTP StatusCode::NOT_IMPLEMENTED (legitimate protocol response).
        re.compile(
            r"(?<!StatusCode::)(?<!status::)\bNotImplemented\b|"
            r"(?<!StatusCode::)(?<!STATUS_)\bNOT_IMPLEMENTED\b"
        ),
        "HIGH",
        "LOW",
        "PRODUCT_PATH_FAKE",
    ),
    (
        "PLACEHOLDER_TOKEN",
        re.compile(r"(?i)\bplaceholder\b"),
        "MEDIUM",
        "INFO",
        "REVIEW_REQUIRED",
    ),
]


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone in {"FIXTURE"}:
            continue
        # Historical docs TODOs: INFO only
        text = read_text(path)
        for i, line in enumerate(text.splitlines(), 1):
            stripped = line.strip()
            if not stripped or stripped.startswith("//!") and zone == "DOC":
                pass
            for name, pat, prod_sev, test_sev, klass in RULES:
                if not pat.search(line):
                    continue
                # HTTP 501 / StatusCode::NOT_IMPLEMENTED is not a placeholder defect.
                if name == "NOT_IMPLEMENTED_TOKEN" and (
                    "InertUnavailable" in line
                    or "InertTransport" in line
                    or "StatusCode::NOT_IMPLEMENTED" in line
                    or "status::NOT_IMPLEMENTED" in line
                    or '"not implemented"' in line.lower()
                    or "'not implemented'" in line.lower()
                ):
                    continue
                if zone == "DOC":
                    # Docs TODO/FIXME are not product blockers
                    if name in {"TODO_FIXME_COMMENT", "PLACEHOLDER_TOKEN"}:
                        sev = "INFO"
                        klass = "REVIEW_REQUIRED"
                    else:
                        sev = "REVIEW"
                elif zone in {"TEST", "SMOKE"}:
                    sev = test_sev
                    if name in {"TODO_MACRO", "TODO_FIXME_COMMENT"}:
                        klass = "REVIEW_REQUIRED"
                elif zone in {"TOOL", "BENCH"}:
                    sev = "MEDIUM" if prod_sev in {"CRITICAL", "HIGH"} else "LOW"
                else:
                    sev = prod_sev
                findings.append(
                    Finding(
                        id=ctx.next_id("PH"),
                        severity=sev,
                        category="PLACEHOLDER_INCOMPLETE",
                        path=rel,
                        line=i,
                        claim=f"No unresolved placeholder ({name}) on product path",
                        evidence=stripped[:200],
                        why_it_matters=(
                            "Placeholder markers can mean the advertised behavior is absent "
                            "even though the code compiles."
                        ),
                        confidence="HIGH" if name.endswith("MACRO") else "MEDIUM",
                        recommended_action=(
                            "Implement, remove from product path, or mark claim NOT_IMPLEMENTED."
                        ),
                        classification=klass,
                        check="placeholder",
                    )
                )
    return findings
