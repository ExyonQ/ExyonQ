#!/usr/bin/env python3
"""Check 10 — security / bypass integrity."""
from __future__ import annotations

import re
from typing import List, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

RULES: List[Tuple[str, re.Pattern[str], str, str]] = [
    (
        "TLS_VERIFY_DISABLE",
        re.compile(
            r"(?i)(danger_accept_invalid_certs\s*\(\s*true\s*\)|"
            r"insecure_skip_verify\s*=\s*true|"
            r"SSL_CERT_NONE|verify\s*=\s*False|"
            r"set_verify\([^)]*NONE)"
        ),
        "CRITICAL",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "ALLOW_ALL",
        re.compile(r"(?i)\ballow_all\b|\bAUTH_BYPASS\b|\bdisable_auth\b\s*=\s*true"),
        "CRITICAL",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "CFG_TEST_SECURITY",
        # Only flag when cfg(test) wraps an obvious bypass/disable, not mere test modules.
        re.compile(
            r"#\[cfg\(test\)\][^\n]{0,120}\n(?:[^\n]*\n){0,6}[^\n]*"
            r"(?:allow_all|disable_auth|skip_verify|insecure_skip|AUTH_BYPASS)",
            re.I,
        ),
        "HIGH",
        "REVIEW_REQUIRED",
    ),
    (
        "DEV_FLAG_DEFAULT",
        re.compile(r"(?i)(?:dev_mode|debug_auth|skip_security)\s*[:=]\s*true"),
        "HIGH",
        "PRODUCT_PATH_FAKE",
    ),
]


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone in {"DOC", "FIXTURE"}:
            continue
        text = read_text(path)
        for name, pat, sev, klass in RULES:
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                snippet = text.splitlines()[line_no - 1].strip()[:200]
                use_sev = sev
                use_klass = klass
                if zone == "TEST" and name in {"TLS_VERIFY_DISABLE", "ALLOW_ALL"}:
                    use_sev = "MEDIUM"
                    use_klass = "TEST_ONLY_SIMULATION"
                findings.append(
                    Finding(
                        id=ctx.next_id("SEC"),
                        severity=use_sev,
                        category="SECURITY_BYPASS",
                        path=rel,
                        line=line_no,
                        claim=f"No security bypass ({name}) on operational paths",
                        evidence=snippet or m.group(0)[:200],
                        why_it_matters="Security checks that exist only in tests or are disabled hide exposure.",
                        confidence="HIGH" if use_sev == "CRITICAL" else "MEDIUM",
                        recommended_action="Remove bypass or confine to explicit test-only with labeling.",
                        classification=use_klass,
                        check="security_bypass",
                    )
                )

    # Cap unsafe inventory noise (INFO only; does not fail the gate alone).
    unsafe_count = 0
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if not (rel.endswith(".rs") and zone == "PRODUCT"):
            continue
        text = read_text(path)
        for i, line in enumerate(text.splitlines(), 1):
            if unsafe_count >= 25:
                break
            if re.search(r"\bunsafe\b", line) and not line.strip().startswith("//"):
                unsafe_count += 1
                findings.append(
                    Finding(
                        id=ctx.next_id("SEC"),
                        severity="INFO",
                        category="SECURITY_BYPASS",
                        path=rel,
                        line=i,
                        claim="unsafe usage inventoried",
                        evidence=line.strip()[:200],
                        why_it_matters="unsafe requires documented invariant; inventory aids review.",
                        confidence="HIGH",
                        recommended_action="Ensure audit/Miri coverage for modified unsafe.",
                        classification="REVIEW_REQUIRED",
                        check="security_bypass",
                    )
                )
        if unsafe_count >= 25:
            findings.append(
                Finding(
                    id=ctx.next_id("SEC"),
                    severity="INFO",
                    category="SECURITY_BYPASS",
                    path=".",
                    line=0,
                    claim="unsafe inventory truncated",
                    evidence="more than 25 product unsafe lines; see cargo/geiger or audit for full set",
                    why_it_matters="Full unsafe census is out of scope for this static pass.",
                    confidence="HIGH",
                    recommended_action="Run dedicated unsafe audit when modifying hot path.",
                    classification="REVIEW_REQUIRED",
                    check="security_bypass",
                )
            )
            break
    return findings
