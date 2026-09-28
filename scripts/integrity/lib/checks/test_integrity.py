#!/usr/bin/env python3
"""Check 5 — test integrity / misleading E2E labels."""
from __future__ import annotations

import re
from typing import List, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding
from checks.zero_fake import _is_containment_path, _is_contained_smoke_detector
from token_context import file_has_executable_token

NAME_CLAIM_RE = re.compile(
    r"(?i)(e2e|end[_-]?to[_-]?end|real[_-]?production|full[_-]?stack|"
    r"integration|production[_-]?ready)"
)
PROCESS_RE = re.compile(
    r"(?i)(Command::new\s*\(|std::process::Command|cargo_bin!?\s*\(|"
    r"assert_cmd::|tokio::process::|/\./target/(?:debug|release)/)"
)
HTTP_CLIENT_RE = re.compile(r"(?i)(reqwest|hyper::Client|ureq|curl\b|httpx|HttpClient)")
SLEEP_ONLY_RE = re.compile(r"(?i)(thread::sleep|tokio::time::sleep|time\.sleep)\(")
TRIVIAL_ASSERT_RE = re.compile(r"assert!\s*\(\s*true\s*\)|assert_eq!\s*\(\s*1\s*,\s*1\s*\)")


def _strip_comments(text: str) -> str:
    """Remove //, #, and /* */ comments so prose does not fake process evidence."""
    no_block = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
    lines = []
    for line in no_block.splitlines():
        stripped = line.lstrip()
        if stripped.startswith("#") and not stripped.startswith("#!"):
            continue
        if "//" in line:
            line = line.split("//", 1)[0]
        # shell inline comments
        if " #" in line:
            line = line.split(" #", 1)[0]
        lines.append(line)
    return "\n".join(lines)


def classify_test(text: str, rel: str) -> Tuple[str, str]:
    """Return (class, rationale)."""
    name = rel.lower()
    code = _strip_comments(text)
    # Executable mock/fake/stub only — policy docstrings / comments do not count.
    has_mock = file_has_executable_token(text, rel)
    has_process = bool(PROCESS_RE.search(code))
    has_http = bool(HTTP_CLIENT_RE.search(code))
    claims_e2e = bool(NAME_CLAIM_RE.search(name) or NAME_CLAIM_RE.search(text[:2000]))

    if has_mock and claims_e2e:
        return "SIMULATION", "name claims e2e/integration but executable mock/fake/stub present"
    if has_mock:
        return "FORBIDDEN_DOUBLE", "executable mock/fake/stub present (SAFE_TEST_DOUBLE=NOT_ALLOWED)"
    if "unit" in name and not claims_e2e:
        return "UNIT", "path suggests unit test"
    if has_process and has_http and not has_mock:
        return "REAL_E2E", "process + external HTTP client, no executable mock tokens"
    if has_process and not has_mock:
        return "PROCESS_INTEGRATION", "spawns process without executable mock tokens"
    if has_http and not has_mock and ("integration" in name or "e2e" in name):
        return "INTEGRATION", "HTTP client without executable mock; may still be in-process"
    if claims_e2e:
        return "UNKNOWN", "name claims e2e/integration without process/binary evidence"
    if "integration" in name:
        return "COMPONENT", "integration name without process evidence"
    return "UNKNOWN", "insufficient evidence for stronger class"


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone not in {"TEST", "SMOKE", "TOOL", "BENCH", "OTHER", "PRODUCT"}:
            continue
        # Focus on test-like files
        if not (
            zone in {"TEST", "SMOKE"}
            or "test" in rel.lower()
            or NAME_CLAIM_RE.search(rel)
            or rel.endswith("_test.rs")
            or "/tests/" in rel
        ):
            continue
        if not rel.endswith((".rs", ".py", ".sh")):
            continue

        text = read_text(path)
        # Containment: policy/auditor/detector surfaces are not product remediation targets.
        # Selftest must keep AUDITOR_NEGATIVE_FIXTURE cases actionable (HIGH).
        if _is_containment_path(rel):
            if zone == "SMOKE":
                if _is_contained_smoke_detector(rel):
                    findings.append(
                        Finding(
                            id=ctx.next_id("TEST"),
                            severity="INFO",
                            category="TEST_INTEGRITY",
                            path=rel,
                            line=1,
                            claim="CONTAINED_DETECTOR: smoke-ban gate path",
                            evidence="EXECUTED_AS_PRODUCT_PATH=NO; detector/policy surface",
                            why_it_matters="Smoke-ban gates intentionally match smoke path patterns.",
                            confidence="MEDIUM",
                            recommended_action="Keep as detector; do not treat as product smoke.",
                            classification="CONTAINED_DETECTOR",
                            check="test_integrity",
                        )
                    )
            continue
        if "AUDITOR_NEGATIVE_FIXTURE" in text and ctx.mode != "selftest":
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="INFO",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="CONTAINED_POLICY_PATH: AUDITOR_NEGATIVE_FIXTURE",
                    evidence="AUDITOR_NEGATIVE_FIXTURE marker; EXECUTED_AS_PRODUCT_PATH=NO",
                    why_it_matters="Negative fixtures exercise the auditor; not product evidence.",
                    confidence="MEDIUM",
                    recommended_action="Keep marker; do not treat as product remediation.",
                    classification="CONTAINED_POLICY_PATH",
                    check="test_integrity",
                )
            )
            continue
        klass, rationale = classify_test(text, rel)
        claims_e2e = bool(NAME_CLAIM_RE.search(rel))

        if zone == "SMOKE":
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="HIGH",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="ZERO_FAKE: smoke category must not exist in workspace",
                    evidence="path matches smoke pattern; SMOKE=FORBIDDEN (absolute)",
                    why_it_matters="Even real flows must not live under smoke/; rename to e2e or delete.",
                    confidence="HIGH",
                    recommended_action="If 100% real: move to scripts/e2e/ and delete smoke tree entry.",
                    classification="FORBIDDEN_SMOKE",
                    check="test_integrity",
                )
            )

        # E2E entrypoints that exec into smoke/ are not REAL_E2E evidence (rule 127).
        if claims_e2e and re.search(r"(?i)scripts/smoke/|smoke/", text) and rel.endswith(".sh"):
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="CRITICAL",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="E2E entrypoint does not delegate to smoke scripts",
                    evidence="path/name claims e2e but script references scripts/smoke/",
                    why_it_matters="SMOKE_AS_ACCEPTANCE_PROOF is forbidden; CLAIM != EVIDENCE.",
                    confidence="HIGH",
                    recommended_action="Implement real E2E without smoke delegation, or rename/demote claim.",
                    classification="CRITICAL_INTEGRITY_VIOLATION",
                    check="test_integrity",
                )
            )
        elif klass == "SIMULATION" and claims_e2e:
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="CRITICAL",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="E2E must not use mock/fake/stub components",
                    evidence=f"class={klass}; {rationale}",
                    why_it_matters="E2E_USING_FAKE_COMPONENT / MOCK_USED_AS_INTEGRATION_EVIDENCE.",
                    confidence="HIGH",
                    recommended_action="Use real peers/backends; delete doubles.",
                    classification="FORBIDDEN_MOCK",
                    check="test_integrity",
                )
            )
        elif klass == "FORBIDDEN_DOUBLE":
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="HIGH",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="ZERO_FAKE: mocks/stubs forbidden even in unit tests",
                    evidence=f"class={klass}; {rationale}",
                    why_it_matters="REVIEW_REQUIRED = NOT_ALLOWED under ZERO_FAKE policy.",
                    confidence="HIGH",
                    recommended_action="Rewrite test against real component or delete.",
                    classification="FORBIDDEN_MOCK",
                    check="test_integrity",
                )
            )
        elif klass == "UNKNOWN" and claims_e2e:
            findings.append(
                Finding(
                    id=ctx.next_id("TEST"),
                    severity="REVIEW",
                    category="TEST_INTEGRITY",
                    path=rel,
                    line=1,
                    claim="E2E/integration label backed by executable evidence",
                    evidence=f"class={klass}; {rationale}",
                    why_it_matters="Misleading labels inflate confidence in incomplete coverage.",
                    confidence="MEDIUM",
                    recommended_action="Add process/binary evidence or downgrade naming.",
                    classification="REVIEW_REQUIRED",
                    check="test_integrity",
                )
            )

        for i, line in enumerate(text.splitlines(), 1):
            if TRIVIAL_ASSERT_RE.search(line):
                findings.append(
                    Finding(
                        id=ctx.next_id("TEST"),
                        severity="MEDIUM",
                        category="TEST_INTEGRITY",
                        path=rel,
                        line=i,
                        claim="Tests contain meaningful assertions",
                        evidence=line.strip()[:200],
                        why_it_matters="Trivial asserts can yield green CI without validating behavior.",
                        confidence="HIGH",
                        recommended_action="Assert observable outcomes of the claimed behavior.",
                        classification="FORBIDDEN_SHORTCUT",
                        check="test_integrity",
                    )
                )
            if SLEEP_ONLY_RE.search(line) and claims_e2e:
                findings.append(
                    Finding(
                        id=ctx.next_id("TEST"),
                        severity="REVIEW",
                        category="TEST_INTEGRITY",
                        path=rel,
                        line=i,
                        claim="E2E synchronization uses real readiness, not sleep alone",
                        evidence=line.strip()[:200],
                        why_it_matters="Sleep-based sync often hides races and flaky false PASS.",
                        confidence="LOW",
                        recommended_action="Poll readiness / use deterministic sync.",
                        classification="REVIEW_REQUIRED",
                        check="test_integrity",
                    )
                )
    return findings
