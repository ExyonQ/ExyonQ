#!/usr/bin/env python3
"""Check 11 — product path vs benchmark path shortcuts."""
from __future__ import annotations

import re
from typing import List, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

PRODUCT_RULES: List[Tuple[str, re.Pattern[str]]] = [
    ("BENCH_API_CACHE_PATHS", re.compile(r"\bBENCH_API_CACHE_PATHS\b")),
    (
        "BENCH_IMPLICIT_CACHE",
        re.compile(r"\bBENCH_(?:API_CACHE|CACHE_PATHS)\b"),
    ),
    (
        "LOADGEN_UA_DETECT",
        re.compile(
            r"(?i)user[-_]?agent.*(?:wrk2?|vegeta|rewrk|h2load|\bhey\b|k6)|"
            r"(?:wrk2?|vegeta|rewrk|h2load|k6).*user[-_]?agent"
        ),
    ),
    (
        "BENCH_ENV_SEMANTIC",
        re.compile(
            r"""(?:env::var|std::env::var|getenv)\s*\(\s*["'](?:BENCHMARK|DEMO|EXYONQ_BENCH|BENCH_|PERF_TEST)"""
        ),
    ),
    (
        "BENCHMARK_MODE_BRANCH",
        re.compile(r"(?i)if\s+.*benchmark[_ ]?mode"),
    ),
]


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        text = read_text(path)
        for name, pat in PRODUCT_RULES:
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                snippet = text.splitlines()[line_no - 1].strip()[:200]
                # Observational bench tracing (metrics only) is not a product semantic shortcut.
                observational = bool(
                    re.search(r"(?i)BENCH_TRACE|PERF_TRACE|bench_trace", snippet)
                    or "bench_trace" in rel
                )
                prohibition_doc = bool(
                    re.search(
                        r"(?i)former|absent|must\s+not|no\s+implicit|forbidden|"
                        r"do\s+not|without\s+\[\[cache_policy\]\]|test comment only|"
                        r"remains in|do not .fix. the gate",
                        snippet,
                    )
                    or "AUDITOR_NEGATIVE_FIXTURE" in text[:2000]
                )
                if observational:
                    sev = "INFO"
                    klass = "REVIEW_REQUIRED"
                elif zone == "PRODUCT" and "/tests/" not in rel and not rel.endswith("_test.rs"):
                    sev = "CRITICAL"
                    klass = "CRITICAL_INTEGRITY_VIOLATION"
                elif (
                    zone == "DOC"
                    or rel.endswith(".md")
                    or "gates/" in rel
                    or "integrity" in rel
                    or prohibition_doc
                ):
                    # Policy/status/gate docs and negative tests that name the ban.
                    sev = "INFO"
                    klass = "CONTAINED_POLICY_PATH"
                elif zone in {"BENCH", "TOOL"}:
                    sev = "REVIEW"
                    klass = "REVIEW_REQUIRED"
                elif zone == "TEST" or "/tests/" in rel or rel.endswith("_test.rs"):
                    sev = "INFO"
                    klass = "CONTAINED_POLICY_PATH"
                else:
                    sev = "HIGH"
                    klass = "PRODUCT_PATH_FAKE"
                findings.append(
                    Finding(
                        id=ctx.next_id("BENCH"),
                        severity=sev,
                        category="PRODUCT_VS_BENCH_PATH",
                        path=rel,
                        line=line_no,
                        claim=f"Benchmark shortcuts do not alter product semantics ({name})",
                        evidence=snippet,
                        why_it_matters=(
                            "BENCHMARK_CODE_MUST_NOT_CHANGE_PRODUCT_SEMANTICS. "
                            "Normal execution must equal benchmark execution."
                        ),
                        confidence="HIGH",
                        recommended_action="Remove product-path bench branch; use public config only.",
                        classification=klass,
                        check="bench_path",
                    )
                )
    return findings
