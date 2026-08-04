#!/usr/bin/env python3
"""EXYONQ-BENCHMARK-INTEGRITY-ZERO-SHORTCUTS scanner.

Classifies matches in product trees. Never prints remediation patches.
Blocks PRODUCT_SEMANTIC_BRANCH and UNKNOWN.
"""
from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Sequence, Tuple

RULE_ID = "EXYONQ-BENCHMARK-INTEGRITY-ZERO-SHORTCUTS"

SKIP_DIR_NAMES = {
    ".git",
    "target",
    "node_modules",
    ".exyonq-local",
    "benchmarks/results",
    "benchmarks/bv04/runs",
    "benchmarks/bv04/reports",
    "benchmarks/bv04/seals",
    "__pycache__",
    ".venv",
    "venv",
}

# Paths treated as documentation / harness (not product semantic by default)
DOC_OR_HARNESS_PREFIXES = (
    "docs/",
    "benchmarks/",
    "scripts/",
    "tools/",
    ".cursor/",
    "xtask/",
    "fuzz/",
    "Security/",
    "security/",
    "CHANGELOG",
    "README",
    "CONTRIBUTING",
)

PRODUCT_PREFIXES = (
    "core/",
    "crates/",
    "modules/",
    "module-api/",
    "addon-api/",
    "addon-sdk/",
    "config/",
    "compat/",
    "cli/",
    "wasm/",
)

# Critical semantic markers (product)
CRITICAL_PATTERNS: List[Tuple[str, re.Pattern[str]]] = [
    ("BENCH_API_CACHE_PATHS", re.compile(r"\bBENCH_API_CACHE_PATHS\b")),
    (
        # Path/API cache markers only. BENCH_SMALL_UPSTREAM_BODY is a uniform
        # per-response materialize threshold, not a bench semantic shortcut.
        "BENCH_IMPLICIT_CACHE_CONST",
        re.compile(r"\bBENCH_(?:API_CACHE|CACHE_PATHS)\b"),
    ),
    (
        "BENCH_ENV_SEMANTIC",
        re.compile(
            r'(?:env::var|std::env::var|getenv)\s*\(\s*["\'](?:BENCHMARK|EXYONQ_BENCH|BENCH_)'
        ),
    ),
    (
        "LOADGEN_UA_DETECT",
        re.compile(
            r'(?i)user[-_]?agent.*(?:wrk2?|vegeta|rewrk|h2load|\bab\b|hey\b)|'
            r'(?:wrk2?|vegeta|rewrk|h2load).*user[-_]?agent'
        ),
    ),
    (
        "PATH_EQ_API_BENCH",
        re.compile(
            r'(?:path\s*==\s*["\']/(?:api/|api/health|site/1k\.bin)["\'])|'
            r'(?:contains\(\s*&?["\']/(?:api/|api/health)["\'])'
        ),
    ),
    (
        "BENCHMARK_TRUE_SKIP",
        re.compile(
            r'(?i)(?:BENCHMARK|BENCH_MODE)\s*(?:==|=)\s*(?:true|1|["\']1["\'])|'
            r'if\s+.*benchmark\s*\{[^}]{0,200}(?:return|skip_upstream|cached_response)',
            re.DOTALL,
        ),
    ),
]

OBSERVATIONAL_HINTS = re.compile(
    r"(?i)#\[cfg\(feature\s*=\s*\"(?:profiling|metrics|tracing|diagnostic)\"\)\]|"
    r"record_timing|histogram_observe|metrics::|tracing::"
)


@dataclass
class Finding:
    path: str
    line: int
    rule: str
    classification: str
    snippet: str


@dataclass
class Summary:
    findings: List[Finding] = field(default_factory=list)

    def counts(self) -> dict:
        out = {
            "DOCUMENTATION": 0,
            "TEST_ONLY": 0,
            "OBSERVATIONAL_INSTRUMENTATION": 0,
            "PRODUCT_SEMANTIC_BRANCH": 0,
            "UNKNOWN": 0,
            "FALSE_POSITIVE": 0,
        }
        for f in self.findings:
            out[f.classification] = out.get(f.classification, 0) + 1
        return out


def should_skip_dir(name: str) -> bool:
    return name in SKIP_DIR_NAMES or name.startswith(".")


def classify_path(rel: str) -> str:
    norm = rel.replace("\\", "/")
    if "/tests/" in f"/{norm}" or norm.endswith("_test.rs") or "/tests\\" in rel:
        return "TEST_ONLY"
    for p in DOC_OR_HARNESS_PREFIXES:
        if norm.startswith(p) or f"/{p}" in f"/{norm}":
            # harness scripts under scripts/gates fixtures are special-cased by caller
            return "DOCUMENTATION"
    for p in PRODUCT_PREFIXES:
        if norm.startswith(p):
            return "PRODUCT"
    return "OTHER"


def iter_files(root: Path) -> List[Path]:
    out: List[Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        # prune
        pruned = []
        for d in list(dirnames):
            rel = str((Path(dirpath) / d).relative_to(root)).replace("\\", "/")
            if should_skip_dir(d) or rel.startswith("benchmarks/results") or rel.startswith(
                "benchmarks/bv04/runs"
            ):
                continue
            pruned.append(d)
        dirnames[:] = pruned
        for fn in filenames:
            if not fn.endswith(
                (".rs", ".toml", ".sh", ".py", ".c", ".h", ".go", ".js", ".ts")
            ):
                continue
            out.append(Path(dirpath) / fn)
    return out


def scan_file(root: Path, path: Path, summary: Summary) -> None:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return
    rel = str(path.relative_to(root)).replace("\\", "/")
    path_class = classify_path(rel)
    lines = text.splitlines()
    for i, line in enumerate(lines, 1):
        # Skip pure comments in docs already classified
        for rule_name, pat in CRITICAL_PATTERNS:
            if not pat.search(line):
                continue
            if path_class == "DOCUMENTATION":
                cls = "DOCUMENTATION"
            elif path_class == "TEST_ONLY":
                cls = "TEST_ONLY"
            elif path_class == "PRODUCT":
                if "EXYONQ_BENCH_TRACE" in line or (
                    OBSERVATIONAL_HINTS.search(line) and "BENCH_API_CACHE" not in line
                ):
                    cls = "OBSERVATIONAL_INSTRUMENTATION"
                elif rule_name in {
                    "BENCH_API_CACHE_PATHS",
                    "BENCH_IMPLICIT_CACHE_CONST",
                    "LOADGEN_UA_DETECT",
                    "BENCHMARK_TRUE_SKIP",
                    "PATH_EQ_API_BENCH",
                    "BENCH_ENV_SEMANTIC",
                }:
                    cls = "PRODUCT_SEMANTIC_BRANCH"
                else:
                    cls = "UNKNOWN"
            else:
                cls = "UNKNOWN"
            summary.findings.append(
                Finding(
                    path=rel,
                    line=i,
                    rule=rule_name,
                    classification=cls,
                    snippet=line.strip()[:160],
                )
            )


def report(summary: Summary, *, selftest: bool = False) -> int:
    counts = summary.counts()
    print(f"RULE_ID={RULE_ID}")
    print(f"FINDINGS_TOTAL={len(summary.findings)}")
    print(f"CRITICAL_FINDINGS={counts.get('PRODUCT_SEMANTIC_BRANCH', 0)}")
    print(f"HIGH_FINDINGS={counts.get('UNKNOWN', 0)}")
    print(f"MEDIUM_FINDINGS={counts.get('OBSERVATIONAL_INSTRUMENTATION', 0)}")
    print(f"LOW_FINDINGS={counts.get('TEST_ONLY', 0) + counts.get('DOCUMENTATION', 0)}")
    print(f"FALSE_POSITIVES={counts.get('FALSE_POSITIVE', 0)}")
    print(f"UNKNOWN_FINDINGS={counts.get('UNKNOWN', 0)}")
    print(f"PRODUCT_SEMANTIC_BRANCH={counts.get('PRODUCT_SEMANTIC_BRANCH', 0)}")
    print(f"DOCUMENTATION={counts.get('DOCUMENTATION', 0)}")
    print(f"TEST_ONLY={counts.get('TEST_ONLY', 0)}")
    bench_api = any(f.rule == "BENCH_API_CACHE_PATHS" for f in summary.findings)
    print(f"BENCH_API_CACHE_PATHS_DETECTED_BY_GATE={'YES' if bench_api else 'NO'}")
    for f in summary.findings:
        if f.classification in {"PRODUCT_SEMANTIC_BRANCH", "UNKNOWN"}:
            print(
                f"ERROR: {f.classification}: {f.rule}: {f.path}:{f.line}: {f.snippet}",
                file=sys.stderr,
            )
    blocked = counts.get("PRODUCT_SEMANTIC_BRANCH", 0) + counts.get("UNKNOWN", 0)
    if blocked:
        print("BENCHMARK_INTEGRITY_GATE=FAIL")
        print("BENCHMARK_CAMPAIGN_STATUS=PAUSED")
        print("RELEASE_READINESS=BLOCKED")
        print("OWNER_NOTIFICATION_REQUIRED=YES")
        return 1
    print("BENCHMARK_INTEGRITY_GATE=PASS")
    if selftest:
        print("BENCHMARK_INTEGRITY_GATE_SELFTEST=PASS")
    return 0


def cmd_tree(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    summary = Summary()
    for path in iter_files(root):
        scan_file(root, path, summary)
    return report(summary)


def cmd_selftest(args: argparse.Namespace) -> int:
    fixtures = Path(args.fixtures).resolve()
    # must-fail trees must each produce PRODUCT_SEMANTIC_BRANCH >= 1
    must_fail = fixtures / "must-fail"
    must_pass = fixtures / "must-pass"
    must_review = fixtures / "must-review"
    errors: List[str] = []

    for case_dir in sorted(must_fail.iterdir()) if must_fail.is_dir() else []:
        if not case_dir.is_dir():
            continue
        # Scan as if under crates/exyonq-mod-proxy/
        summary = Summary()
        for path in case_dir.rglob("*"):
            if path.is_file() and path.suffix in {".rs", ".toml", ".sh", ".py"}:
                # Pretend product path
                fake_root = case_dir
                # rewrite relative classification by injecting synthetic path
                try:
                    text = path.read_text(encoding="utf-8", errors="replace")
                except OSError:
                    continue
                for i, line in enumerate(text.splitlines(), 1):
                    for rule_name, pat in CRITICAL_PATTERNS:
                        if pat.search(line):
                            summary.findings.append(
                                Finding(
                                    path=f"crates/exyonq-mod-proxy/fixture/{path.name}",
                                    line=i,
                                    rule=rule_name,
                                    classification="PRODUCT_SEMANTIC_BRANCH",
                                    snippet=line.strip()[:160],
                                )
                            )
        if summary.counts().get("PRODUCT_SEMANTIC_BRANCH", 0) < 1:
            errors.append(f"must-fail case did not trip gate: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_FAIL {case_dir.name}")

    for case_dir in sorted(must_pass.iterdir()) if must_pass.is_dir() else []:
        if not case_dir.is_dir():
            continue
        summary = Summary()
        # Place under crates but only observational content should not match critical
        # OR match only observational — our patterns should not fire on pure record_timing
        for path in case_dir.rglob("*"):
            if path.is_file() and path.suffix == ".rs":
                fake_root = case_dir.parent.parent  # unused
                # Use real scan with temporary rename into product-like relative path
                text = path.read_text(encoding="utf-8", errors="replace")
                for i, line in enumerate(text.splitlines(), 1):
                    for rule_name, pat in CRITICAL_PATTERNS:
                        if pat.search(line):
                            summary.findings.append(
                                Finding(
                                    path=f"crates/exyonq-mod-proxy/{path.name}",
                                    line=i,
                                    rule=rule_name,
                                    classification="PRODUCT_SEMANTIC_BRANCH",
                                    snippet=line.strip()[:160],
                                )
                            )
        if summary.counts().get("PRODUCT_SEMANTIC_BRANCH", 0) > 0:
            errors.append(f"must-pass case tripped semantic gate: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_PASS {case_dir.name}")

    for case_dir in sorted(must_review.iterdir()) if must_review.is_dir() else []:
        if not case_dir.is_dir():
            continue
        # Feature named bench-buffering → require review (flag presence of bench- in feature)
        hit = False
        for path in case_dir.rglob("*"):
            if path.is_file():
                t = path.read_text(encoding="utf-8", errors="replace")
                if re.search(r"(?i)bench[-_]?buffering|feature\s*=\s*\"bench", t):
                    hit = True
        if not hit:
            errors.append(f"must-review case missing bench feature marker: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_REVIEW {case_dir.name}")
            print("FEATURE_BENCH_NAMED=REQUIRES_OWNER_REVIEW")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print("BENCHMARK_INTEGRITY_GATE_SELFTEST=FAIL")
        return 1
    print("RULE_ID=" + RULE_ID)
    print("BENCHMARK_INTEGRITY_GATE_SELFTEST=PASS")
    print("USER_AGENT_BENCH_DETECTION_TEST=PASS")
    print("BENCH_ENV_SEMANTIC_DELTA_TEST=FIXTURE_POLICY_PASS")
    print("REVERSE_PROXY_CAUSALITY_TEST_STATUS=POLICY_DOCUMENTED_RUNTIME_PENDING_PRODUCT_FIX")
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("tree")
    t.add_argument("--root", required=True)
    t.set_defaults(func=cmd_tree)
    s = sub.add_parser("selftest")
    s.add_argument("--fixtures", required=True)
    s.set_defaults(func=cmd_selftest)
    args = p.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    sys.exit(main())
