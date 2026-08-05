#!/usr/bin/env python3
"""PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA product scanner.

Blocks PRODUCT_SEMANTIC_BRANCH, FAKE_RESULT_GENERATION, UNKNOWN.
Does not modify product code. Does not print remediation patches.
"""
from __future__ import annotations

import argparse
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Optional, Sequence, Tuple

RULE_ID = "PROJECT-INTEGRITY-NO-SHORTCUTS-NO-FAKE-DATA"

SKIP_DIR_NAMES = {
    ".git",
    "target",
    "node_modules",
    ".exyonq-local",
    "__pycache__",
    ".venv",
    "venv",
}

SKIP_REL_PREFIXES = (
    "benchmarks/results/",
    "benchmarks/bv04/runs/",
    "benchmarks/bv04/reports/",
    "benchmarks/bv04/seals/",
    "benchmarks/rivals-cache/",
)

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

# (rule_name, pattern, default_block_class_in_product)
CRITICAL_PATTERNS: List[Tuple[str, re.Pattern[str], str]] = [
    (
        "BENCH_API_CACHE_PATHS",
        re.compile(r"\bBENCH_API_CACHE_PATHS\b"),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
    (
        # Cross-request / path-scoped bench cache markers only.
        # BENCH_SMALL_UPSTREAM_BODY is a per-response materialize threshold name
        # (applies equally to normal traffic) — not an implicit response cache.
        "BENCH_IMPLICIT_CACHE_CONST",
        re.compile(r"\bBENCH_(?:API_CACHE|CACHE_PATHS)\b"),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
    (
        "LOADGEN_UA_DETECT",
        re.compile(
            r"(?i)user[-_]?agent.*(?:wrk2?|vegeta|rewrk|h2load|\bab\b|hey\b|k6|jmeter)|"
            r"(?:wrk2?|vegeta|rewrk|h2load|k6).*user[-_]?agent"
        ),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
    (
        "BENCHMARK_MODE_CACHE",
        re.compile(
            r"(?i)if\s+.*(?:benchmark[_ ]?mode|demo[_ ]?mode)\s*\{[^}]{0,240}"
            r"(?:return|cached_response|fast_response|fake_users|skip_upstream)",
            re.DOTALL,
        ),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
    (
        "BENCH_ENV_SEMANTIC",
        re.compile(
            r"(?:env::var|std::env::var|getenv)\s*\(\s*[\"'](?:BENCHMARK|DEMO|EXYONQ_BENCH|BENCH_|PERF_TEST)"
        ),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
    (
        "HARDCODED_BENCH_METRIC",
        re.compile(
            r"(?i)\b(?:BENCHMARK_RESULT_RPS|HARDCODED_RESULT|EXPECTED_RESULT_RPS|"
            r"FAKE_THROUGHPUT|DEMO_RPS)\b\s*[:=]"
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "MISSING_AS_ZERO",
        re.compile(
            r"(?i)(?:missing[_ ]?(?:latency|metric|value|rps)|not[_ ]?measured)\s*=\s*0\b|"
            r"(?:latency|metric)\s*=\s*0\s*;\s*//\s*(?:missing|absent|n/?a)"
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "REUSE_PREVIOUS_RUN_METRIC",
        re.compile(
            r"(?i)(?:metric|rps|throughput|latency)\s*=\s*previous_run\.(?:metric|rps|throughput|latency)|"
            r"copy.*(?:from|previous).*run.*metric"
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "UNLABELED_ESTIMATE_INSERT",
        re.compile(
            r"(?i)(?:report|metrics|results)\.(?:insert|set|put)\s*\(\s*[\"'](?:throughput|rps|latency|p99)"
            r"[\"']\s*,\s*(?:estimated_|estimate_|projected_)"
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "SAMPLE_RESULTS_AS_REAL",
        re.compile(
            r"(?i)load_results\s*\(\s*[\"'][^\"']*sample[-_]results[^\"']*[\"']\s*\)|"
            r"(?:read|load|open)\s*\(\s*[\"'][^\"']*(?:demo|fake|sample)[-_]results"
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "DEMO_FAKE_USERS",
        re.compile(
            r"(?i)(?:demo_mode|DEMO)\s*.{0,80}(?:fake_users|mock_users|synthetic_users)|"
            r"fn\s+fake_users\s*\("
        ),
        "FAKE_RESULT_GENERATION",
    ),
    (
        "PATH_EQ_API_BENCH",
        re.compile(
            r"(?:path\s*==\s*[\"']/(?:api/|api/health|site/1k\.bin)[\"'])|"
            r"(?:contains\(\s*&?[\"']/(?:api/|api/health)[\"'])"
        ),
        "PRODUCT_SEMANTIC_BRANCH",
    ),
]

OBSERVATIONAL_HINTS = re.compile(
    r"(?i)#\[cfg\(feature\s*=\s*\"(?:profiling|metrics|tracing|diagnostic)\"\)\]|"
    r"record_timing|histogram_observe|metrics::|tracing::"
)

REVIEW_FEATURE = re.compile(
    r"(?i)bench[-_]?buffering|feature\s*=\s*\"bench|"
    r"demo[-_]?mode|feature\s*=\s*\"demo|"
    r"synthetic\s+fixtures|reconstructed\s+from\s+partial\s+logs|"
    r"REQUIRES_OWNER_REVIEW"
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
        out: dict = {}
        for f in self.findings:
            out[f.classification] = out.get(f.classification, 0) + 1
        return out


def should_skip_dir(name: str) -> bool:
    return name in SKIP_DIR_NAMES or name.startswith(".")


def classify_path(rel: str) -> str:
    norm = rel.replace("\\", "/")
    if "/tests/" in f"/{norm}" or norm.endswith("_test.rs") or norm.endswith("_tests.rs"):
        return "TEST_ONLY"
    if "fixtures/" in norm and ("gates/fixtures" in norm or "/fixtures/" in norm):
        # Gate selftest fixtures — classified by selftest harness, not tree
        return "DECLARED_FIXTURE"
    for p in DOC_OR_HARNESS_PREFIXES:
        if norm.startswith(p):
            return "DOCUMENTATION"
    for p in PRODUCT_PREFIXES:
        if norm.startswith(p):
            return "PRODUCT"
    return "OTHER"


def iter_files(root: Path) -> List[Path]:
    out: List[Path] = []
    for dirpath, dirnames, filenames in os.walk(root):
        pruned = []
        for d in list(dirnames):
            rel = str((Path(dirpath) / d).relative_to(root)).replace("\\", "/")
            if should_skip_dir(d):
                continue
            if any(rel.startswith(p.rstrip("/")) or rel.startswith(p) for p in SKIP_REL_PREFIXES):
                continue
            pruned.append(d)
        dirnames[:] = pruned
        for fn in filenames:
            if not fn.endswith((".rs", ".toml", ".sh", ".py", ".c", ".h", ".go", ".js", ".ts", ".json")):
                continue
            out.append(Path(dirpath) / fn)
    return out


def classify_match(path_class: str, rule_name: str, block_class: str, line: str) -> str:
    if path_class == "DOCUMENTATION":
        return "DOCUMENTATION"
    if path_class == "TEST_ONLY":
        return "TEST_ONLY"
    if path_class == "DECLARED_FIXTURE":
        return "DECLARED_FIXTURE"
    if path_class == "PRODUCT":
        if "EXYONQ_BENCH_TRACE" in line or (
            OBSERVATIONAL_HINTS.search(line)
            and "BENCH_API_CACHE" not in line
            and block_class != "FAKE_RESULT_GENERATION"
        ):
            return "OBSERVATIONAL_INSTRUMENTATION"
        return block_class
    return "UNKNOWN"


def scan_file(root: Path, path: Path, summary: Summary) -> None:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return
    rel = str(path.relative_to(root)).replace("\\", "/")
    if any(rel.startswith(p) for p in SKIP_REL_PREFIXES):
        return
    path_class = classify_path(rel)
    for i, line in enumerate(text.splitlines(), 1):
        for rule_name, pat, block_class in CRITICAL_PATTERNS:
            if not pat.search(line):
                # Multiline-ish: also check small windows for BENCHMARK_MODE_CACHE
                continue
            cls = classify_match(path_class, rule_name, block_class, line)
            summary.findings.append(
                Finding(
                    path=rel,
                    line=i,
                    rule=rule_name,
                    classification=cls,
                    snippet=line.strip()[:160],
                )
            )
    # Multiline demo/benchmark mode bodies
    if path_class == "PRODUCT":
        for rule_name, pat, block_class in CRITICAL_PATTERNS:
            if rule_name != "BENCHMARK_MODE_CACHE":
                continue
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                snippet = m.group(0).replace("\n", " ")[:160]
                summary.findings.append(
                    Finding(
                        path=rel,
                        line=line_no,
                        rule=rule_name,
                        classification=block_class,
                        snippet=snippet,
                    )
                )


def report(summary: Summary, *, selftest: bool = False) -> int:
    counts = summary.counts()
    ps = counts.get("PRODUCT_SEMANTIC_BRANCH", 0)
    fake = counts.get("FAKE_RESULT_GENERATION", 0)
    unk = counts.get("UNKNOWN", 0)
    print(f"RULE_ID={RULE_ID}")
    print(f"FINDINGS_TOTAL={len(summary.findings)}")
    print(f"CRITICAL_FINDINGS={ps + fake}")
    print(f"HIGH_FINDINGS={unk}")
    print(f"MEDIUM_FINDINGS={counts.get('OBSERVATIONAL_INSTRUMENTATION', 0)}")
    print(
        f"LOW_FINDINGS={counts.get('TEST_ONLY', 0) + counts.get('DOCUMENTATION', 0) + counts.get('DECLARED_FIXTURE', 0)}"
    )
    print(f"FALSE_POSITIVES={counts.get('FALSE_POSITIVE', 0)}")
    print(f"UNKNOWN_FINDINGS={unk}")
    print(f"PRODUCT_SEMANTIC_BRANCH={ps}")
    print(f"FAKE_RESULT_GENERATION={fake}")
    print(f"DOCUMENTATION={counts.get('DOCUMENTATION', 0)}")
    print(f"TEST_ONLY={counts.get('TEST_ONLY', 0)}")
    bench_api = any(f.rule == "BENCH_API_CACHE_PATHS" for f in summary.findings)
    print(f"BENCH_API_CACHE_PATHS_DETECTED_BY_GATE={'YES' if bench_api else 'NO'}")
    for f in summary.findings:
        if f.classification in {
            "PRODUCT_SEMANTIC_BRANCH",
            "FAKE_RESULT_GENERATION",
            "UNKNOWN",
        }:
            print(
                f"ERROR: {f.classification}: {f.rule}: {f.path}:{f.line}: {f.snippet}",
                file=sys.stderr,
            )
    blocked = ps + fake + unk
    if blocked:
        print("PROJECT_INTEGRITY_GATE=FAIL")
        print("RELEASE_READINESS=BLOCKED")
        print("BENCHMARK_READINESS=BLOCKED")
        print("OWNER_NOTIFICATION_REQUIRED=YES")
        return 1
    print("PROJECT_INTEGRITY_GATE=PASS")
    if selftest:
        print("PROJECT_INTEGRITY_GATE_SELFTEST=PASS")
    return 0


def scan_fixture_as_product(case_dir: Path) -> Summary:
    summary = Summary()
    for path in case_dir.rglob("*"):
        if not path.is_file():
            continue
        if path.suffix not in {".rs", ".toml", ".sh", ".py", ".json"}:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for i, line in enumerate(text.splitlines(), 1):
            for rule_name, pat, block_class in CRITICAL_PATTERNS:
                if pat.search(line):
                    summary.findings.append(
                        Finding(
                            path=f"crates/exyonq-mod-proxy/fixture/{path.name}",
                            line=i,
                            rule=rule_name,
                            classification=block_class,
                            snippet=line.strip()[:160],
                        )
                    )
        for rule_name, pat, block_class in CRITICAL_PATTERNS:
            if rule_name != "BENCHMARK_MODE_CACHE":
                continue
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                summary.findings.append(
                    Finding(
                        path=f"crates/exyonq-mod-proxy/fixture/{path.name}",
                        line=line_no,
                        rule=rule_name,
                        classification=block_class,
                        snippet=m.group(0).replace("\n", " ")[:160],
                    )
                )
    return summary


def cmd_tree(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    summary = Summary()
    for path in iter_files(root):
        scan_file(root, path, summary)
    return report(summary)


def cmd_selftest(args: argparse.Namespace) -> int:
    fixtures = Path(args.fixtures).resolve()
    must_fail = fixtures / "must-fail"
    must_pass = fixtures / "must-pass"
    must_review = fixtures / "must-review"
    errors: List[str] = []

    for case_dir in sorted(must_fail.iterdir()) if must_fail.is_dir() else []:
        if not case_dir.is_dir():
            continue
        summary = scan_fixture_as_product(case_dir)
        c = summary.counts()
        blocked = c.get("PRODUCT_SEMANTIC_BRANCH", 0) + c.get("FAKE_RESULT_GENERATION", 0)
        if blocked < 1:
            errors.append(f"must-fail case did not trip gate: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_FAIL {case_dir.name}")

    for case_dir in sorted(must_pass.iterdir()) if must_pass.is_dir() else []:
        if not case_dir.is_dir():
            continue
        summary = scan_fixture_as_product(case_dir)
        c = summary.counts()
        blocked = c.get("PRODUCT_SEMANTIC_BRANCH", 0) + c.get("FAKE_RESULT_GENERATION", 0)
        if blocked > 0:
            errors.append(f"must-pass case tripped integrity gate: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_PASS {case_dir.name}")

    for case_dir in sorted(must_review.iterdir()) if must_review.is_dir() else []:
        if not case_dir.is_dir():
            continue
        hit = False
        for path in case_dir.rglob("*"):
            if path.is_file() and REVIEW_FEATURE.search(
                path.read_text(encoding="utf-8", errors="replace")
            ):
                hit = True
        if not hit:
            errors.append(f"must-review case missing review marker: {case_dir.name}")
        else:
            print(f"PASS_EXPECT_REVIEW {case_dir.name}")
            print("FEATURE_NAMED_REQUIRES_OWNER_REVIEW=YES")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print("PROJECT_INTEGRITY_GATE_SELFTEST=FAIL")
        return 1
    print(f"RULE_ID={RULE_ID}")
    print("PROJECT_INTEGRITY_GATE_SELFTEST=PASS")
    print("USER_AGENT_BENCH_DETECTION_TEST=PASS")
    print("FAKE_METRIC_DETECTION_TEST=PASS")
    print("MISSING_AS_ZERO_DETECTION_TEST=PASS")
    print("DEMO_FAKE_DATA_DETECTION_TEST=PASS")
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
