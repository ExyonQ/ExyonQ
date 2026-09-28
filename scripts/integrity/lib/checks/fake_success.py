#!/usr/bin/env python3
"""Check 6 — hard-coded success / fake results."""
from __future__ import annotations

import re
from typing import List, Tuple

from context import ScanContext, iter_files, read_text, rel_of
from model import Finding

RULES: List[Tuple[str, re.Pattern[str], str, str]] = [
    (
        "OR_TRUE",
        re.compile(r"\|\|\s*true\b"),
        "HIGH",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "EXIT_0_UNCONDITIONAL",
        re.compile(r"^\s*exit\s+0\s*(?:#.*)?$", re.M),
        "HIGH",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "HARDCODED_OK_HANDLER",
        re.compile(
            r"(?i)(?:"
            r"Ok\(\(\)\)\s*;\s*//\s*(?:always|fake|stub|success|placeholder)|"
            r"return\s+Ok\(\(\)\)\s*;\s*//\s*(?:always|fake|stub|success|placeholder|todo|not\s+implemented)|"
            r"\bHARDCODED_SUCCESS\b|"
            r"\balways_ok\s*\(|"
            r"\bfake_success\b|"
            r"status\s*[:=]\s*200\s*;\s*//\s*(?:always|fake|stub|hardcoded)"
            r")"
        ),
        "CRITICAL",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "HARDCODED_METRIC",
        re.compile(
            r"(?i)\b(?:FAKE_THROUGHPUT|DEMO_RPS|HARDCODED_RESULT|BENCHMARK_RESULT_RPS|"
            r"EXPECTED_RESULT_RPS)\b\s*[:=]"
        ),
        "CRITICAL",
        "CRITICAL_INTEGRITY_VIOLATION",
    ),
    (
        "STDERR_DISCARD",
        re.compile(r"2\s*>\s*/dev/null"),
        "MEDIUM",
        "REVIEW_REQUIRED",
    ),
    (
        "SET_E_DISABLED",
        re.compile(r"^\s*set\s+\+e\b", re.M),
        "REVIEW",
        "REVIEW_REQUIRED",
    ),
]


def _is_legitimate_best_effort_or_true(line: str) -> bool:
    """True when `|| true` cannot convert a required gate/assert into false PASS.

    LEGITIMATE_BEST_EFFORT — optional env/tooling, teardown, or grep exit-1-as-empty.
    REQUIRED_STEP_FAILURE_SWALLOWED remains a finding when this returns False.
    """
    s = line.strip()
    # Optional shell env bootstrap (cargo still required via PATH / later checks).
    if re.search(r"(?i)^\s*source\s+.+\|\|\s*true\b", s):
        return True
    if re.search(r"(?i)^\s*unset\s+\S+.+\|\|\s*true\b", s):
        return True
    # Teardown / cleanup after the authoritative assert already ran.
    if re.search(
        r"(?i)(rm\s+-rf|rm\s+-f|git\s+.+\btag\s+-d|docker\s+compose\s+\S+\s+stop|"
        r"docker\s+stop|kill\s+|wait\s+\"?\$|chmod\s+)",
        s,
    ):
        return True
    # grep/find exit 1 = no match (count/filter), not command failure of a required build.
    if re.search(
        r"(?i)\b(grep\s+-c|grep\s+-q|grep\s+-vE|grep\s+-v\b|grep\s+'|grep\s+\"|find\s+)",
        s,
    ):
        return True
    # Optional copy when source may be absent (explicit 2>/dev/null || true).
    if re.search(r"(?i)^\s*cp\s+.+\s+2\s*>\s*/dev/null\s*\|\|\s*true\b", s):
        return True
    # dd/noise fill for negative fixture tamper (test harness teardown/setup aid).
    if re.search(r"(?i)^\s*dd\s+if=/dev/urandom\b.+\|\|\s*true\b", s):
        return True
    # Redirect-only stamp files for redacted logs.
    if re.search(r"(?i)>\s*\"?[^\"]*redacted[^\"]*\"?\s+2\s*>\s*/dev/null\s*\|\|\s*true\b", s):
        return True
    # cosign version display (informational footer).
    if re.search(r"(?i)(\$\{?COSIGN_BIN\}?|cosign).+\bversion\b.+\|\|\s*true\b", s):
        return True
    # done < <(find ...) process substitution failure as empty.
    if re.search(r"(?i)done\s*<\s*<\s*\(.*find\b", s):
        return True
    # Capture tool stdout/stderr when the tool may exit non-zero while still
    # producing the asserted diagnostic (reload --diff/--check, etc.).
    if re.search(r"(?i)\$\(\s*.+\|\|\s*true\s*\)", s):
        return True
    # Assignment form: out="$(cmd 2>&1 || true)" / pw="$(cat ... || true)"
    if re.search(r'(?i)=\s*"\$\([^"]*\|\|\s*true', s):
        return True
    if re.search(r"(?i)\$\([^)]*\|\|\s*true\s*\)", s):
        return True
    return False


def _logical_shell_line(lines: List[str], line_no: int) -> str:
    """Join backslash-continued shell lines so `|| true` sees the full command."""
    i = line_no - 1
    if i < 0 or i >= len(lines):
        return ""
    start = i
    while start > 0 and lines[start - 1].rstrip().endswith("\\"):
        start -= 1
    parts = [lines[j].rstrip().rstrip("\\").strip() for j in range(start, i + 1)]
    return " ".join(parts)


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    for path in iter_files(ctx.root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(ctx.root, path)
        zone = ctx.classify(rel).zone
        if zone in {"FIXTURE", "DOC"}:
            # Still scan fixture trees only in selftest via separate path
            if zone == "DOC":
                continue
        text = read_text(path)
        lines = text.splitlines()
        for name, pat, sev, klass in RULES:
            for m in pat.finditer(text):
                line_no = text[: m.start()].count("\n") + 1
                snippet = lines[line_no - 1].strip()[:200] if line_no <= len(lines) else ""
                logical = _logical_shell_line(lines, line_no) if name == "OR_TRUE" else snippet
                use_sev = sev
                use_klass = klass
                # CI/packaging often use `|| true` for optional install steps — not product fakes.
                if name == "OR_TRUE":
                    if rel.startswith(".github/") or rel.endswith(".yml") or rel.endswith(".yaml"):
                        # Optional CI install steps — skip (not product integrity).
                        continue
                    if _is_legitimate_best_effort_or_true(logical) or _is_legitimate_best_effort_or_true(
                        snippet
                    ):
                        use_sev = "INFO"
                        use_klass = "LEGITIMATE_BEST_EFFORT"
                    elif zone in {"TOOL", "DOC"}:
                        use_sev = "INFO"
                        use_klass = "REVIEW_REQUIRED"
                    elif zone == "TEST":
                        use_sev = "MEDIUM"
                        use_klass = "TEST_ONLY_SIMULATION"
                    elif zone == "PRODUCT":
                        use_sev = "HIGH"
                    else:
                        use_sev = "REVIEW"
                        use_klass = "REVIEW_REQUIRED"
                if zone == "TEST" and name in {"STDERR_DISCARD", "SET_E_DISABLED"}:
                    use_sev = "INFO"
                    use_klass = "REVIEW_REQUIRED"
                elif name in {"STDERR_DISCARD", "SET_E_DISABLED"}:
                    # Extremely common in harnesses; keep as REVIEW unless product path.
                    use_sev = "REVIEW" if zone != "PRODUCT" else "MEDIUM"
                    use_klass = "REVIEW_REQUIRED"
                if name == "EXIT_0_UNCONDITIONAL":
                    # Regression gate embeds intentional soft-pass samples in --selftest heredocs.
                    if rel.endswith("bench-exit-propagation-gate.sh"):
                        continue
                    prev = text[max(0, m.start() - 500) : m.start()]
                    # Help / usage exits are not failure masking.
                    if re.search(
                        r"(?im)(usage:|--help\b|cat\s+<<['\"]?EOF|Modes:\s*$)",
                        prev,
                    ):
                        use_sev = "INFO"
                        use_klass = "LEGITIMATE_HELP_EXIT"
                    # Explicit success after a real gate/assertion.
                    elif re.search(
                        r"(?im)(PASS:|PROTECTOR-PASS|Verdict:|SMOKE PASS|GATE_PASS|"
                        r"\[\[\s*\"\$MATRIX_GATE\"\s*==\s*\"PROTECTOR-PASS\"\s*\]\])",
                        prev,
                    ):
                        use_sev = "INFO"
                        use_klass = "LEGITIMATE_SUCCESS_EXIT"
                    # Soft-skip / missing work returning 0 remains HIGH in BENCH/PRODUCT.
                    elif re.search(
                        r"(?im)(not found|requires Linux|Saving smoke|container not found|"
                        r"bench-runner not found|PROTECTOR_SELECTIVE|platform_not_linux)",
                        prev,
                    ):
                        use_sev = "HIGH"
                        use_klass = "CRITICAL_INTEGRITY_VIOLATION"
                    elif zone == "TOOL":
                        use_sev = "REVIEW"
                        use_klass = "REVIEW_REQUIRED"
                    elif zone == "BENCH":
                        # Bare trailing exit 0 after measured work: review, not auto-HIGH.
                        use_sev = "REVIEW"
                        use_klass = "REVIEW_REQUIRED"
                findings.append(
                    Finding(
                        id=ctx.next_id("FAKE"),
                        severity=use_sev,
                        category="HARDCODED_SUCCESS",
                        path=rel,
                        line=line_no,
                        claim=f"No hard-coded success pattern ({name})",
                        evidence=snippet,
                        why_it_matters=(
                            "Hard-coded success / masked failures convert FAIL into PASS "
                            "without executing the claimed work."
                        ),
                        confidence="HIGH" if use_sev in {"CRITICAL", "HIGH"} else "MEDIUM",
                        recommended_action="Propagate real exit status; remove fake success branches.",
                        classification=use_klass,
                        check="fake_success",
                    )
                )
    return findings
