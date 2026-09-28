#!/usr/bin/env python3
"""Check 7 — error propagation / swallowed failures."""
from __future__ import annotations

import re
from typing import List

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

PATTERNS = [
    (
        "LET_UNDERSCORE_RESULT",
        re.compile(r"let\s+_\s*=\s*[^;]+(?:\?|;)", re.M),
        "REVIEW",
        "REVIEW_REQUIRED",
    ),
    (
        "DISCARD_RESULT",
        re.compile(r"let\s+_\s*=\s*(?:std::|tokio::)?(?:fs|io|net|process)::"),
        "HIGH",
        "PRODUCT_PATH_FAKE",
    ),
    (
        "OK_IGNORE",
        re.compile(r"\.ok\(\)\s*;"),
        "MEDIUM",
        "REVIEW_REQUIRED",
    ),
    (
        "UNWRAP_OR_DEFAULT_CRITICAL",
        re.compile(r"(?i)(?:auth|tls|cert|token|password|signature).{0,40}\.unwrap_or_default\(\)"),
        "HIGH",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "PIPE_STATUS_RISK",
        re.compile(r"\|\s*(?:grep|rg|head|tail|true)\b"),
        "REVIEW",
        "REVIEW_REQUIRED",
    ),
]


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    pipe_emitted = 0
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone in {"DOC", "FIXTURE"}:
            continue
        if not rel.endswith((".rs", ".sh", ".py")):
            continue
        text = read_text(path)
        for name, pat, sev, klass in PATTERNS:
            if name == "PIPE_STATUS_RISK" and not rel.endswith(".sh"):
                continue
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                snippet = text.splitlines()[line_no - 1].strip()[:200]
                use_sev = sev
                use_klass = klass
                if zone == "TEST" and name in {"LET_UNDERSCORE_RESULT", "OK_IGNORE", "DISCARD_RESULT"}:
                    use_sev = "INFO"
                    use_klass = "REVIEW_REQUIRED"
                # Cleanup best-effort removes are normal.
                if name == "DISCARD_RESULT" and re.search(
                    r"remove_file|remove_dir|rename\(|create_dir", snippet
                ):
                    use_sev = "INFO"
                    use_klass = "REVIEW_REQUIRED"
                if name == "PIPE_STATUS_RISK":
                    if pipe_emitted >= 40:
                        continue
                    pipe_emitted += 1
                    use_sev = "REVIEW"
                    use_klass = "REVIEW_REQUIRED"
                if name == "LET_UNDERSCORE_RESULT":
                    use_sev = "INFO" if zone != "PRODUCT" else "REVIEW"
                findings.append(
                    Finding(
                        id=ctx.next_id("ERR"),
                        severity=use_sev,
                        category="ERROR_PROPAGATION",
                        path=rel,
                        line=line_no,
                        claim=f"Errors propagate ({name})",
                        evidence=snippet,
                        why_it_matters=(
                            "Swallowed errors can yield false success and leave incorrect state."
                        ),
                        confidence="LOW" if use_sev == "REVIEW" else "MEDIUM",
                        recommended_action="Handle or propagate Result/exit codes explicitly.",
                        classification=use_klass,
                        check="error_propagation",
                    )
                )
    return findings
