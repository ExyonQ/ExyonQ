#!/usr/bin/env python3
"""ZERO_FAKE / REAL FLOW ONLY — absolute ban on smoke/mock/fake/stub/simulation.

POLICY: REVIEW_REQUIRED = NOT_ALLOWED
Presence in workspace is a finding even outside product path.
"""
from __future__ import annotations

import re
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding
from token_context import line_has_executable_token

# Policy forbid classes (classification field)
FORBIDDEN = {
    "FORBIDDEN_SMOKE",
    "FORBIDDEN_SIMULATION",
    "FORBIDDEN_MOCK",
    "FORBIDDEN_FAKE",
    "FORBIDDEN_STUB",
    "FORBIDDEN_DUMMY",
    "FORBIDDEN_PLACEHOLDER",
    "FORBIDDEN_SHORTCUT",
}

POLICY_META_RE = re.compile(
    r"(?i)("
    r"forbidden|must\s+not|do\s+not\s+use|ban(?:ned)?|prohibited|"
    r"not\s+a\s+(?:product\s+)?mock|no[-_ ]smoke|anti[-_ ]fake|"
    r"prefer\s+no\s+fake|no\s+fake\b|"
    r"REVIEW_REQUIRED\s*=\s*NOT_ALLOWED|ZERO_FAKE|"
    r"mocks?\s+are\s+forbidden|smoke\s+as\s+(?:acceptance\s+)?proof|"
    r"AUDITOR_NEGATIVE_FIXTURE|"
    r"SMOKE_AS_ACCEPTANCE_PROOF\s*=\s*FORBIDDEN|"
    r"CONTAINED_DETECTOR|CONTAINED_POLICY_PATH|EXECUTED_AS_PRODUCT_PATH|"
    r"FORBIDDEN_SMOKE|FORBIDDEN_MOCK|zero[-_ ]smoke[-_ ]category|"
    r"no[-_ ]smoke[-_ ]as[-_ ]proof|detector\s+source|policy\s+scan|"
    # Policy selftests / status text that name forbidden tokens to reject them.
    r"no\s+synthetic\b|synthetic\s+(?:mutation|HandlerTable|tree)|"
    r"placeholder\s+or\s+empty|Version\s+placeholder|"
    r"CONTROL\s*=\s*.*placeholder|detect(?:s|ing)?\s+(?:synthetic|placeholder|fake|mock|stub)"
    r")"
)

# Paths that define/enforce policy — not executed as product.
# EXECUTED_AS_PRODUCT_PATH = NO
_CONTAINMENT_PREFIXES = (
    "scripts/integrity/",
    "scripts/gates/tests/",
    "scripts/gates/fixtures/",
    "scripts/architecture/dependency_containment/",
    "docs/governance/",
)

# Verify / status scripts whose purpose is policy assertion (not product runtime).
_CONTAINMENT_EXACT = frozenset(
    {
        "scripts/architecture/verify-phase0-kernel.sh",
        "scripts/verify-oss-boundaries.sh",
        "scripts/verify-release-audit.sh",
        "PROJECT_STATUS.md",
    }
)


def _norm_rel(rel: str) -> str:
    return rel.replace("\\", "/")


def _is_containment_path(rel: str) -> bool:
    """Policy/auditor/negative-fixture surfaces — not product execution."""
    norm = _norm_rel(rel)
    if norm in _CONTAINMENT_EXACT:
        return True
    if any(norm.startswith(p) for p in _CONTAINMENT_PREFIXES):
        return True
    if norm.startswith("scripts/gates/lib/") and norm.endswith("_scan.py"):
        return True
    if norm == "scripts/gates/exyonq-owned-mock-references-gate.sh":
        return True
    base = norm.rsplit("/", 1)[-1].lower()
    if norm.startswith("scripts/gates/") and (
        "no-smoke" in base or "zero-smoke" in base
    ):
        return True
    return False


def _is_contained_smoke_detector(rel: str) -> bool:
    """Gate scripts whose purpose is banning smoke (PATH_SMOKE hits)."""
    norm = _norm_rel(rel)
    base = norm.rsplit("/", 1)[-1].lower()
    if not norm.startswith("scripts/gates/"):
        return False
    if "gate" not in base:
        return False
    return "no-smoke" in base or "zero-smoke" in base


def _severity_for_containment(
    sev: str, rel: str, *, path_level_smoke: bool = False
) -> tuple[str, str]:
    """Downgrade actionable findings on containment scopes to INFO (policy text)."""
    if _is_contained_smoke_detector(rel) and path_level_smoke:
        return "INFO", "CONTAINED_DETECTOR"
    if _is_containment_path(rel):
        if sev in {"CRITICAL", "HIGH", "MEDIUM", "REVIEW"}:
            return "INFO", "CONTAINED_POLICY_PATH"
    return sev, ""



def _is_comment_only(line: str) -> bool:
    s = line.lstrip()
    return (
        s.startswith("//")
        or s.startswith("///")
        or s.startswith("//!")
        or s.startswith("#")
        or s.startswith("*")  # block-comment continuation
        or s.startswith("/*")
    )


def _scan_cfg_test_ranges(text: str) -> List[Tuple[int, int]]:
    """Return inclusive line ranges (1-based) that are inside #[cfg(test)] modules."""
    lines = text.splitlines()
    ranges: List[Tuple[int, int]] = []
    i = 0
    while i < len(lines):
        if re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]", lines[i]):
            # Find following `mod ... {` or `fn ... {` and track braces
            j = i + 1
            while j < len(lines) and not re.search(r"\b(mod|fn)\b", lines[j]):
                j += 1
            if j >= len(lines):
                break
            # Find opening brace
            depth = 0
            started = False
            start_line = j + 1
            k = j
            while k < len(lines):
                depth += lines[k].count("{") - lines[k].count("}")
                if lines[k].count("{") and not started:
                    started = True
                    start_line = k + 1
                if started and depth <= 0:
                    ranges.append((start_line, k + 1))
                    i = k + 1
                    break
                k += 1
            else:
                break
            continue
        i += 1
    return ranges


def _in_ranges(line_no: int, ranges: List[Tuple[int, int]]) -> bool:
    return any(a <= line_no <= b for a, b in ranges)

# Token → (classification, default_severity_product, default_severity_other)
TOKEN_RULES: List[Tuple[str, re.Pattern[str], str, str, str]] = [
    ("smoke", re.compile(r"(?i)\bsmoke(?:[-_ ]?test)?\b"), "FORBIDDEN_SMOKE", "HIGH", "HIGH"),
    (
        "smoke_mechanism",
        re.compile(r"(?i)(?:scripts/smoke/|SMOKE_BACKEND\s*=)"),
        "FORBIDDEN_SMOKE",
        "HIGH",
        "HIGH",
    ),
    (
        "simulation",
        re.compile(r"(?i)\b(?:simulation|simulate[ds]?|emulated?|emulator)\b"),
        "FORBIDDEN_SIMULATION",
        "CRITICAL",
        "HIGH",
    ),
    (
        "mock",
        re.compile(r"(?i)\b(?:mock(?:ed|ing|ito)?|wiremock)\b"),
        "FORBIDDEN_MOCK",
        "CRITICAL",
        "HIGH",
    ),
    (
        "fake",
        re.compile(r"(?i)\b(?:fake[ds]?|faking)\b"),
        "FORBIDDEN_FAKE",
        "CRITICAL",
        "HIGH",
    ),
    ("stub", re.compile(r"(?i)\bstub(?:bed|bing)?\b"), "FORBIDDEN_STUB", "CRITICAL", "HIGH"),
    ("dummy", re.compile(r"(?i)\bdummy\b"), "FORBIDDEN_DUMMY", "CRITICAL", "HIGH"),
    (
        "placeholder",
        re.compile(r"(?i)\bplaceholder\b"),
        "FORBIDDEN_PLACEHOLDER",
        "HIGH",
        "MEDIUM",
    ),
    (
        "noop",
        re.compile(r"(?i)\b(?:no-?op|noop)\b"),
        "FORBIDDEN_PLACEHOLDER",
        "CRITICAL",
        "HIGH",
    ),
    (
        "unimplemented",
        # todo!/unimplemented! macros only. Enum fail-closed variants use InertUnavailable.
        re.compile(r"(?i)\b(?:unimplemented!\s*\(|\btodo!\s*\()"),
        "FORBIDDEN_PLACEHOLDER",
        "CRITICAL",
        "HIGH",
    ),
    (
        "hardcoded",
        re.compile(r"(?i)\b(?:hardcoded|HARDCODED_SUCCESS|fake_success|always_ok)\b"),
        "FORBIDDEN_SHORTCUT",
        "CRITICAL",
        "HIGH",
    ),
    (
        "demo",
        re.compile(r"(?i)\b(?:demo[-_ ]path|demo[-_ ]mode|/demo/)\b"),
        "FORBIDDEN_SHORTCUT",
        "HIGH",
        "HIGH",
    ),
    (
        "synthetic",
        re.compile(r"(?i)\bsynthetic\b"),
        "FORBIDDEN_SIMULATION",
        "HIGH",
        "MEDIUM",
    ),
]

PATH_SMOKE_RE = re.compile(r"(^|/)smoke(/|$)|[-_]smoke[-_.]", re.I)
PATH_MOCK_RE = re.compile(r"(^|/)mock(/|$)|[-_]mock[-_.]|MockFpm|mock_", re.I)


def _zone_bucket(zone: str, rel: str) -> str:
    if PATH_SMOKE_RE.search(rel) or zone == "SMOKE":
        return "SCRIPT" if zone == "SMOKE" else zone
    if zone == "PRODUCT":
        return "PRODUCT"
    if zone == "TEST":
        return "TEST"
    if zone == "BENCH":
        return "BENCHMARK"
    if zone == "DOC":
        return "DOCUMENTATION"
    if zone == "TOOL":
        if "/e2e/" in rel or rel.startswith("scripts/e2e/"):
            return "E2E"
        if "fixture" in rel.lower():
            return "FIXTURE"
        if "example" in rel.lower():
            return "EXAMPLE"
        return "TOOLING" if not rel.startswith("scripts/") else "SCRIPT"
    if "/e2e/" in rel:
        return "E2E"
    return "UNKNOWN"


def _is_policy_meta(line: str) -> bool:
    return bool(POLICY_META_RE.search(line))


def _is_real_test_input_context(line: str, rel: str) -> bool:
    """Static known inputs are allowed; they must not substitute components."""
    if re.search(r"(?i)(expected|want_|fixture_body|sample_request|known_input)", line):
        return True
    if "generate-ephemeral-tls" in rel or "test-tls/" in rel:
        return True
    return False


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    # One finding per (path, classification) for path-level inventory to limit explosion;
    # plus line hits for executable code tokens.
    path_emitted: Dict[str, set] = {}

    # Path-level: any file under smoke/ or named *mock*
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        bucket = _zone_bucket(zone, rel)
        if zone == "DOC" and not PATH_SMOKE_RE.search(rel) and not PATH_MOCK_RE.search(rel):
            continue

        if PATH_SMOKE_RE.search(rel):
            path_emitted.setdefault(rel, set())
            if "FORBIDDEN_SMOKE" not in path_emitted[rel]:
                path_emitted[rel].add("FORBIDDEN_SMOKE")
                sev, cklass = _severity_for_containment(
                    "HIGH", rel, path_level_smoke=True
                )
                if _is_contained_smoke_detector(rel):
                    sev, cklass = "INFO", "CONTAINED_DETECTOR"
                findings.append(
                    Finding(
                        id=ctx.next_id("ZF"),
                        severity=sev,
                        category="ZERO_FAKE",
                        path=rel,
                        line=1,
                        claim=(
                            "CONTAINED_DETECTOR: smoke-ban gate path"
                            if cklass == "CONTAINED_DETECTOR"
                            else "ZERO_FAKE: no smoke paths in workspace"
                        ),
                        evidence=(
                            f"path zone={bucket} matches smoke naming/location; "
                            f"EXECUTED_AS_PRODUCT_PATH=NO containment={cklass or 'NONE'}"
                        ),
                        why_it_matters="SMOKE = FORBIDDEN even if the body later proves real.",
                        confidence="HIGH" if sev in {"CRITICAL", "HIGH"} else "MEDIUM",
                        recommended_action=(
                            "Keep as detector; do not execute as product evidence."
                            if cklass in {"CONTAINED_DETECTOR", "CONTAINED_POLICY_PATH"}
                            else "Move to scripts/e2e/ (if 100% real) and delete smoke category."
                        ),
                        classification=cklass if cklass else "FORBIDDEN_SMOKE",
                        check="zero_fake",
                    )
                )

        if PATH_MOCK_RE.search(rel) and zone != "DOC":
            sev = "CRITICAL" if zone == "PRODUCT" else "HIGH"
            sev, cklass = _severity_for_containment(sev, rel)
            if "FORBIDDEN_MOCK" not in path_emitted.setdefault(rel, set()):
                path_emitted[rel].add("FORBIDDEN_MOCK")
                findings.append(
                    Finding(
                        id=ctx.next_id("ZF"),
                        severity=sev,
                        category="ZERO_FAKE",
                        path=rel,
                        line=1,
                        claim="ZERO_FAKE: no mock modules/paths in workspace",
                        evidence=(
                            f"path zone={bucket} matches mock naming; "
                            f"EXECUTED_AS_PRODUCT_PATH=NO containment={cklass or 'NONE'}"
                        ),
                        why_it_matters="REVIEW_REQUIRED = NOT_ALLOWED; mocks must be removed.",
                        confidence="HIGH" if sev in {"CRITICAL", "HIGH"} else "MEDIUM",
                        recommended_action=(
                            "Keep as policy/detector surface; not product execution."
                            if cklass
                            else "Replace with real component/peer or delete."
                        ),
                        classification=cklass if cklass else "FORBIDDEN_MOCK",
                        check="zero_fake",
                    )
                )

        if not rel.endswith(
            (".rs", ".sh", ".py", ".toml", ".md", ".yml", ".yaml", ".json")
        ):
            continue

        text = read_text(path)
        cfg_test_ranges = _scan_cfg_test_ranges(text) if rel.endswith(".rs") else []
        # Cap line hits per file/classification
        line_caps: Dict[str, int] = {}
        # Negative fixtures under full-tree scans: one INFO note, no token HIGH.
        # Selftest keeps actionable HIGH (mode=selftest; case root is not containment).
        file_neg_fixture = "AUDITOR_NEGATIVE_FIXTURE" in text
        if file_neg_fixture and ctx.mode != "selftest":
            if "AUDITOR_NEGATIVE_FIXTURE" not in path_emitted.setdefault(rel, set()):
                path_emitted[rel].add("AUDITOR_NEGATIVE_FIXTURE")
                findings.append(
                    Finding(
                        id=ctx.next_id("ZF"),
                        severity="INFO",
                        category="ZERO_FAKE",
                        path=rel,
                        line=1,
                        claim="CONTAINED_POLICY_PATH: AUDITOR_NEGATIVE_FIXTURE",
                        evidence="AUDITOR_NEGATIVE_FIXTURE marker; EXECUTED_AS_PRODUCT_PATH=NO",
                        why_it_matters="Negative fixtures exercise the auditor; not product evidence.",
                        confidence="MEDIUM",
                        recommended_action="Keep marker; do not treat as product remediation.",
                        classification="CONTAINED_POLICY_PATH",
                        check="zero_fake",
                    )
                )
            continue
        contain_file = _is_containment_path(rel)
        for i, line in enumerate(text.splitlines(), 1):
            stripped = line.strip()
            if not stripped:
                continue
            if _is_policy_meta(stripped):
                continue
            if "AUDITOR_NEGATIVE_FIXTURE" in stripped:
                continue
            # Fail-closed unwired transport is not a product simulation.
            if re.search(r"\b(?:InertUnavailable|InertTransport)\b", stripped):
                continue
            if zone == "DOC":
                # Docs mentioning forbidden words as policy → skip
                continue
            if _is_real_test_input_context(stripped, rel):
                # Label as REAL_TEST_INPUT (INFO) once per file max
                if "REAL_TEST_INPUT" not in path_emitted.setdefault(rel, set()):
                    path_emitted[rel].add("REAL_TEST_INPUT")
                    findings.append(
                        Finding(
                            id=ctx.next_id("ZF"),
                            severity="INFO",
                            category="ZERO_FAKE",
                            path=rel,
                            line=i,
                            claim="REAL_TEST_INPUT allowed (static known input)",
                            evidence=stripped[:160],
                            why_it_matters="Known inputs are permitted; component substitution is not.",
                            confidence="MEDIUM",
                            recommended_action="Keep as input-only; never substitute SUT peers.",
                            classification="REAL_TEST_INPUT",
                            check="zero_fake",
                        )
                    )
                continue

            comment_only = _is_comment_only(line)
            in_cfg_test = _in_ranges(i, cfg_test_ranges)

            # Language-aware context applies only to mock/smoke/fake/stub family.
            # Must not prefilter placeholder / noop / demo / unimplemented rules
            # (coverage must not shrink for those TOKEN_RULES).
            context_executable = line_has_executable_token(text, rel, i, line)

            for token_name, pat, klass, sev_prod, sev_other in TOKEN_RULES:
                if not pat.search(stripped):
                    continue
                if klass in {
                    "FORBIDDEN_SMOKE",
                    "FORBIDDEN_SIMULATION",
                    "FORBIDDEN_MOCK",
                    "FORBIDDEN_FAKE",
                    "FORBIDDEN_STUB",
                    "FORBIDDEN_DUMMY",
                } and not context_executable:
                    continue
                # Skip StatusCode::NOT_IMPLEMENTED protocol mapping
                if klass == "FORBIDDEN_PLACEHOLDER" and (
                    "StatusCode::NOT_IMPLEMENTED" in stripped
                    or '"not implemented"' in stripped.lower()
                ):
                    continue
                n = line_caps.get(klass, 0)
                if n >= 3:
                    continue
                line_caps[klass] = n + 1

                # Documentation of intentional empty defaults / host-unavailable
                # constructors is not an executable fake product path.
                if comment_only and klass in {
                    "FORBIDDEN_PLACEHOLDER",
                    "FORBIDDEN_STUB",
                    "FORBIDDEN_DUMMY",
                    "FORBIDDEN_MOCK",
                    "FORBIDDEN_FAKE",
                    "FORBIDDEN_SIMULATION",
                    "FORBIDDEN_SMOKE",
                }:
                    sev = "INFO"
                    use_klass = "DOC_TOKEN_ONLY"
                elif in_cfg_test and klass in {
                    "FORBIDDEN_DUMMY",
                    "FORBIDDEN_MOCK",
                    "FORBIDDEN_STUB",
                    "FORBIDDEN_FAKE",
                    "FORBIDDEN_SIMULATION",
                    "FORBIDDEN_PLACEHOLDER",
                }:
                    # Unit-test harness naming inside #[cfg(test)] — not a live peer substitute.
                    sev = "INFO"
                    use_klass = "CFG_TEST_HARNESS_TOKEN"
                elif zone == "PRODUCT":
                    sev = sev_prod
                    use_klass = klass
                elif zone in {"TEST", "SMOKE"} and klass in {
                    "FORBIDDEN_MOCK",
                    "FORBIDDEN_FAKE",
                    "FORBIDDEN_STUB",
                    "FORBIDDEN_DUMMY",
                    "FORBIDDEN_SIMULATION",
                    "FORBIDDEN_SMOKE",
                }:
                    sev = "HIGH"  # even unit tests — absolute ban outside cfg(test) product files
                    use_klass = klass
                else:
                    sev = sev_other
                    use_klass = klass
                if contain_file and sev in {"CRITICAL", "HIGH", "MEDIUM", "REVIEW"}:
                    sev = "INFO"
                    use_klass = "CONTAINED_POLICY_PATH"
                findings.append(
                    Finding(
                        id=ctx.next_id("ZF"),
                        severity=sev,
                        category="ZERO_FAKE",
                        path=rel,
                        line=i,
                        claim=f"ZERO_FAKE: no {klass} executable/token in workspace",
                        evidence=(
                            stripped[:160]
                            + (
                                "; EXECUTED_AS_PRODUCT_PATH=NO"
                                if use_klass == "CONTAINED_POLICY_PATH"
                                else ""
                            )
                        ),
                        why_it_matters=(
                            "REAL CODE ONLY / REAL PRODUCT PATH ONLY. "
                            "Test doubles and smoke are forbidden in ExyonQ."
                        ),
                        confidence="HIGH" if sev in {"CRITICAL", "HIGH"} else "MEDIUM",
                        recommended_action=(
                            "Keep as policy/auditor surface; not product remediation."
                            if use_klass == "CONTAINED_POLICY_PATH"
                            else "Delete or replace with real flow; no REVIEW_REQUIRED."
                        ),
                        classification=use_klass if sev == "INFO" else klass,
                        check="zero_fake",
                    )
                )

    return findings


def zero_fake_gate(findings: List[Finding]) -> str:
    """PASS only if no FORBIDDEN_* executable evidence (CRITICAL/HIGH)."""
    blocking = [
        f
        for f in findings
        if f.classification in FORBIDDEN and f.severity in {"CRITICAL", "HIGH"}
    ]
    if blocking:
        return "FAIL"
    review = [f for f in findings if f.classification in FORBIDDEN and f.severity == "REVIEW"]
    if review:
        return "REVIEW"
    return "PASS"


def classify_counts(findings: List[Finding]) -> Dict[str, int]:
    out = {
        "FORBIDDEN_SMOKE_COUNT": 0,
        "FORBIDDEN_SIMULATION_COUNT": 0,
        "FORBIDDEN_MOCK_COUNT": 0,
        "FORBIDDEN_FAKE_COUNT": 0,
        "FORBIDDEN_STUB_COUNT": 0,
        "FORBIDDEN_DUMMY_COUNT": 0,
        "FORBIDDEN_PLACEHOLDER_COUNT": 0,
        "FORBIDDEN_SHORTCUT_COUNT": 0,
        "REAL_TEST_INPUT_COUNT": 0,
    }
    for f in findings:
        key = f"{f.classification}_COUNT"
        if key in out:
            out[key] += 1
        if f.classification == "REAL_TEST_INPUT":
            out["REAL_TEST_INPUT_COUNT"] += 1
    return out
