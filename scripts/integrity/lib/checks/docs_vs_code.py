#!/usr/bin/env python3
"""Check 9 — documentation vs code strong claims."""
from __future__ import annotations

import re
from typing import List

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

STRONG_CLAIM_RE = re.compile(
    r"(?i)\b("
    r"fully\s+supported|production\s+ready|production-ready|completely\s+implemented|"
    r"fully\s+implemented|end-to-end\s+supported|real\s+production|"
    r"complete\s+compatibility|100%\s+compatible|feature\s+complete|"
    r"verified\s+real\s+production"
    r")\b"
)


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    claims = getattr(ctx, "claims", []) or []
    verified_ids = {c.id for c in claims if c.status == "VERIFIED"}

    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone not in {"DOC", "OTHER"} and not rel.endswith(".md"):
            continue
        if not rel.endswith((".md", ".html", ".txt")):
            continue
        # Skip auditor README examples
        if rel.startswith("scripts/integrity/"):
            continue
        text = read_text(path)
        for i, line in enumerate(text.splitlines(), 1):
            m = STRONG_CLAIM_RE.search(line)
            if not m:
                continue
            # If a VERIFIED claim id appears on same line, soften
            if any(cid in line for cid in verified_ids):
                continue
            findings.append(
                Finding(
                    id=ctx.next_id("DOC"),
                    severity="REVIEW",
                    category="DOCUMENTATION_VS_CODE",
                    path=rel,
                    line=i,
                    claim="Strong documentation claims are backed by feature-claims evidence",
                    evidence=line.strip()[:200],
                    why_it_matters=(
                        "Docs can claim completeness without runtime evidence. "
                        "Automated semantics are imperfect → REVIEW_REQUIRED."
                    ),
                    confidence="LOW",
                    recommended_action="Link to claim id + evidence or soften wording.",
                    classification="REVIEW_REQUIRED",
                    check="docs_vs_code",
                )
            )
    return findings
