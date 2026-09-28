#!/usr/bin/env python3
"""Finding / report model for EXYONQ CODE INTEGRITY AUDITOR."""
from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional


SEVERITIES = ("CRITICAL", "HIGH", "MEDIUM", "LOW", "INFO", "REVIEW")
CONFIDENCES = ("HIGH", "MEDIUM", "LOW")
CLAIM_STATUSES = (
    "VERIFIED",
    "PARTIAL",
    "UNVERIFIED",
    "DEFERRED",
    "NOT_IMPLEMENTED",
    "BROKEN",
)


@dataclass
class Finding:
    id: str
    severity: str
    category: str
    path: str
    line: int
    claim: str
    evidence: str
    why_it_matters: str
    confidence: str
    recommended_action: str
    classification: str = ""
    check: str = ""

    def to_dict(self) -> Dict[str, Any]:
        return asdict(self)


@dataclass
class ClaimRecord:
    id: str
    description: str
    status: str
    entrypoint: str = ""
    evidence: List[str] = field(default_factory=list)
    notes: str = ""


@dataclass
class AuditReport:
    mode: str
    root: str
    started_at: str
    finished_at: str = ""
    head: str = ""
    branch: str = ""
    dirty: bool = False
    findings: List[Finding] = field(default_factory=list)
    claims: List[ClaimRecord] = field(default_factory=list)
    dimensions: Dict[str, str] = field(default_factory=dict)
    selftest_status: str = "NOT_RUN"
    auditor_error: Optional[str] = None
    zero_fake_counts: Dict[str, int] = field(default_factory=dict)
    root_cause_summary: Dict[str, Any] = field(default_factory=dict)

    def counts(self) -> Dict[str, int]:
        out = {s: 0 for s in SEVERITIES}
        for f in self.findings:
            out[f.severity] = out.get(f.severity, 0) + 1
        return out

    def claim_counts(self) -> Dict[str, int]:
        out = {s: 0 for s in CLAIM_STATUSES}
        for c in self.claims:
            out[c.status] = out.get(c.status, 0) + 1
        return out

    def overall_status(self) -> str:
        if self.auditor_error:
            return "ERROR"
        c = self.counts()
        if c.get("CRITICAL", 0) > 0 or c.get("HIGH", 0) > 0:
            return "FAIL"
        if c.get("REVIEW", 0) > 0:
            return "REVIEW_REQUIRED"
        return "PASS"

    def exit_code(self) -> int:
        status = self.overall_status()
        if status == "ERROR":
            return 2
        if status == "FAIL":
            return 1
        if status == "REVIEW_REQUIRED":
            return 3
        return 0


def utc_now() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def write_artifacts(report: AuditReport, out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    evidence_dir = out_dir / "evidence"
    evidence_dir.mkdir(exist_ok=True)

    findings = [f.to_dict() for f in report.findings]
    payload = {
        "mode": report.mode,
        "root": report.root,
        "started_at": report.started_at,
        "finished_at": report.finished_at,
        "head": report.head,
        "branch": report.branch,
        "dirty": report.dirty,
        "status": report.overall_status(),
        "exit_code": report.exit_code(),
        "counts": report.counts(),
        "claim_counts": report.claim_counts(),
        "dimensions": report.dimensions,
        "zero_fake_counts": report.zero_fake_counts,
        "root_cause_summary": {
            k: v
            for k, v in report.root_cause_summary.items()
            if k != "root_causes"
        },
        "root_causes": report.root_cause_summary.get("root_causes", []),
        "selftest_status": report.selftest_status,
        "auditor_error": report.auditor_error,
        "findings": findings,
        "claims": [asdict(c) for c in report.claims],
        "policy": "ZERO_FAKE_REAL_FLOW_ONLY",
        "safe_test_double": "NOT_ALLOWED",
    }
    # Full root-cause table as separate artifact
    (evidence_dir / "root_causes.json").write_text(
        json.dumps(report.root_cause_summary, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    (out_dir / "findings.json").write_text(
        json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    tsv_lines = [
        "ID\tSEVERITY\tCATEGORY\tPATH\tLINE\tCLAIM\tEVIDENCE\tWHY_IT_MATTERS\tCONFIDENCE\tRECOMMENDED_ACTION\tCLASSIFICATION\tCHECK"
    ]
    for f in report.findings:
        tsv_lines.append(
            "\t".join(
                [
                    f.id,
                    f.severity,
                    f.category,
                    f.path,
                    str(f.line),
                    _tsv(f.claim),
                    _tsv(f.evidence),
                    _tsv(f.why_it_matters),
                    f.confidence,
                    _tsv(f.recommended_action),
                    f.classification,
                    f.check,
                ]
            )
        )
    (out_dir / "findings.tsv").write_text("\n".join(tsv_lines) + "\n", encoding="utf-8")

    summary = render_summary(report)
    (out_dir / "summary.txt").write_text(summary + "\n", encoding="utf-8")
    (evidence_dir / "report_meta.json").write_text(
        json.dumps(
            {
                "root": report.root,
                "head": report.head,
                "branch": report.branch,
                "mode": report.mode,
                "started_at": report.started_at,
                "finished_at": report.finished_at,
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )


def render_summary(report: AuditReport) -> str:
    c = report.counts()
    cc = report.claim_counts()
    dims = report.dimensions
    lines = [
        "EXYONQ CODE INTEGRITY AUDITOR",
        f"MODE={report.mode}",
        f"ROOT={report.root}",
        f"BRANCH={report.branch}",
        f"HEAD={report.head}",
        f"DIRTY={'YES' if report.dirty else 'NO'}",
        f"STARTED_AT={report.started_at}",
        f"FINISHED_AT={report.finished_at}",
        "",
        f"EXYONQ_CODE_INTEGRITY_AUDIT = {report.overall_status()}",
        f"ZERO_FAKE_WORKSPACE_GATE = {dims.get('ZERO_FAKE_WORKSPACE_GATE', 'UNKNOWN')}",
        f"CRITICAL_FINDINGS = {c.get('CRITICAL', 0)}",
        f"HIGH_FINDINGS = {c.get('HIGH', 0)}",
        f"MEDIUM_FINDINGS = {c.get('MEDIUM', 0)}",
        f"LOW_FINDINGS = {c.get('LOW', 0)}",
        f"REVIEW_FINDINGS = {c.get('REVIEW', 0)}",
        "",
        f"UNIQUE_ROOT_CAUSES = {report.root_cause_summary.get('UNIQUE_ROOT_CAUSES', 0)}",
        f"INDIVIDUAL_OCCURRENCES = {report.root_cause_summary.get('INDIVIDUAL_OCCURRENCES', 0)}",
        "",
        f"PRODUCT_PATH_INTEGRITY = {dims.get('PRODUCT_PATH_INTEGRITY', 'UNKNOWN')}",
        f"TEST_INTEGRITY = {dims.get('TEST_INTEGRITY', 'UNKNOWN')}",
        f"E2E_INTEGRITY = {dims.get('E2E_INTEGRITY', 'UNKNOWN')}",
        f"BENCHMARK_INTEGRITY = {dims.get('BENCHMARK_INTEGRITY', 'UNKNOWN')}",
        f"CONFIG_EFFECTIVENESS = {dims.get('CONFIG_EFFECTIVENESS', 'UNKNOWN')}",
        f"FEATURE_CLAIM_INTEGRITY = {dims.get('FEATURE_CLAIM_INTEGRITY', 'UNKNOWN')}",
        f"SECURITY_BYPASS_INTEGRITY = {dims.get('SECURITY_BYPASS_INTEGRITY', 'UNKNOWN')}",
        "",
        f"FORBIDDEN_SMOKE_COUNT = {report.zero_fake_counts.get('FORBIDDEN_SMOKE_COUNT', 0)}",
        f"FORBIDDEN_SIMULATION_COUNT = {report.zero_fake_counts.get('FORBIDDEN_SIMULATION_COUNT', 0)}",
        f"FORBIDDEN_MOCK_COUNT = {report.zero_fake_counts.get('FORBIDDEN_MOCK_COUNT', 0)}",
        f"FORBIDDEN_FAKE_COUNT = {report.zero_fake_counts.get('FORBIDDEN_FAKE_COUNT', 0)}",
        f"FORBIDDEN_STUB_COUNT = {report.zero_fake_counts.get('FORBIDDEN_STUB_COUNT', 0)}",
        f"FORBIDDEN_DUMMY_COUNT = {report.zero_fake_counts.get('FORBIDDEN_DUMMY_COUNT', 0)}",
        f"FORBIDDEN_PLACEHOLDER_COUNT = {report.zero_fake_counts.get('FORBIDDEN_PLACEHOLDER_COUNT', 0)}",
        f"FORBIDDEN_SHORTCUT_COUNT = {report.zero_fake_counts.get('FORBIDDEN_SHORTCUT_COUNT', 0)}",
        f"REAL_TEST_INPUT_COUNT = {report.zero_fake_counts.get('REAL_TEST_INPUT_COUNT', 0)}",
        "",
        f"UNVERIFIED_CLAIMS = {cc.get('UNVERIFIED', 0)}",
        f"PARTIAL_CLAIMS = {cc.get('PARTIAL', 0)}",
        f"VERIFIED_CLAIMS = {cc.get('VERIFIED', 0)}",
        f"NOT_IMPLEMENTED_CLAIMS = {cc.get('NOT_IMPLEMENTED', 0)}",
        f"BROKEN_CLAIMS = {cc.get('BROKEN', 0)}",
        f"DEFERRED_CLAIMS = {cc.get('DEFERRED', 0)}",
        "",
        f"AUDITOR_SELFTEST = {report.selftest_status}",
        f"EXIT_CODE = {report.exit_code()}",
        "",
        "TOP_FINDINGS:",
    ]
    ranked = sorted(
        report.findings,
        key=lambda f: (SEVERITIES.index(f.severity) if f.severity in SEVERITIES else 99, f.id),
    )
    for f in ranked[:25]:
        lines.append(
            f"  [{f.severity}/{f.confidence}] {f.id} {f.path}:{f.line} — {f.claim}"
        )
    if not report.findings:
        lines.append("  (none)")
    if report.auditor_error:
        lines.append("")
        lines.append(f"AUDITOR_ERROR = {report.auditor_error}")
    return "\n".join(lines)


def _tsv(value: str) -> str:
    return value.replace("\t", " ").replace("\n", " ").strip()
