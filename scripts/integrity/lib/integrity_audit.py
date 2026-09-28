#!/usr/bin/env python3
"""EXYONQ CODE INTEGRITY AUDITOR — orchestration engine.

CLAIM != EVIDENCE
CODE_PRESENT != FEATURE_WORKING
TEST_PRESENT != REAL_END_TO_END_BEHAVIOR
CONFIG_ACCEPTED != CONFIG_EFFECTIVE
"""
from __future__ import annotations

import argparse
import shutil
import sys
from pathlib import Path
from typing import List, Optional, Sequence

LIB_DIR = Path(__file__).resolve().parent
if str(LIB_DIR) not in sys.path:
    sys.path.insert(0, str(LIB_DIR))

from checks import FULL_CHECKS, QUICK_CHECKS  # noqa: E402
from checks.zero_fake import classify_counts, zero_fake_gate  # noqa: E402
from context import ScanContext, changed_files, dim_status, git_meta, iter_files, read_text, rel_of  # noqa: E402
from model import AuditReport, Finding, utc_now, write_artifacts  # noqa: E402
from root_causes import aggregate_root_causes  # noqa: E402


RULE_ID = "EXYONQ-CODE-INTEGRITY-AUDITOR"
POLICY_ID = "ZERO_FAKE_REAL_FLOW_ONLY"

DIM_MAP = {
    "PRODUCT_PATH_INTEGRITY": (
        "DEAD_UNREACHABLE_PATH",
        "PLACEHOLDER_INCOMPLETE",
        "PRODUCT_VS_BENCH_PATH",
        "ZERO_FAKE",
    ),
    "TEST_INTEGRITY": ("TEST_INTEGRITY", "ZERO_FAKE"),
    "E2E_INTEGRITY": ("TEST_INTEGRITY", "ZERO_FAKE"),
    "BENCHMARK_INTEGRITY": ("PRODUCT_VS_BENCH_PATH", "ZERO_FAKE", "HARDCODED_SUCCESS"),
    "CONFIG_EFFECTIVENESS": ("CONFIG_EFFECTIVENESS",),
    "FEATURE_CLAIM_INTEGRITY": ("FEATURE_CLAIM_TRACEABILITY", "DOCUMENTATION_VS_CODE"),
    "SECURITY_BYPASS_INTEGRITY": ("SECURITY_BYPASS",),
}


def pick_out_dir(root: Path, explicit: Optional[str]) -> Path:
    if explicit:
        return Path(explicit).resolve()
    # Prefer worktree-local state root (canonical); artifacts/ is gitignored but not preferred.
    base = root / ".exyonq-local" / "integrity"
    stamp = utc_now().replace(":", "").replace("-", "")
    return (base / stamp).resolve()


def link_latest(out_dir: Path) -> None:
    latest = out_dir.parent / "latest"
    if latest.is_symlink() or latest.exists():
        if latest.is_dir() and not latest.is_symlink():
            shutil.rmtree(latest)
        else:
            latest.unlink()
    try:
        latest.symlink_to(out_dir.name, target_is_directory=True)
    except OSError:
        # fallback copy summary only
        latest.mkdir(parents=True, exist_ok=True)
        for name in ("summary.txt", "findings.json", "findings.tsv"):
            src = out_dir / name
            if src.is_file():
                shutil.copy2(src, latest / name)


def filter_info_for_exit(findings: List[Finding]) -> List[Finding]:
    """INFO findings do not affect FAIL/REVIEW exit alone."""
    return findings


def compute_dimensions(findings: List[Finding]) -> dict:
    dims = {}
    for dim, cats in DIM_MAP.items():
        dims[dim] = dim_status(findings, cats)
    dims["ZERO_FAKE_WORKSPACE_GATE"] = zero_fake_gate(findings)
    return dims


def run_audit(mode: str, root: Path, out_dir: Path, baseline: str = "main") -> AuditReport:
    meta = git_meta(root)
    report = AuditReport(
        mode=mode,
        root=str(root),
        started_at=utc_now(),
        head=meta.get("head", "UNKNOWN"),
        branch=meta.get("branch", "UNKNOWN"),
        dirty=bool(meta.get("dirty")),
    )
    changed = changed_files(root, baseline) if mode == "changed" else set()
    ctx = ScanContext(
        root=root,
        mode=mode,
        changed_only=(mode == "changed"),
        changed_paths=changed,
    )

    checks = QUICK_CHECKS if mode == "quick" else FULL_CHECKS
    # --changed uses full check set on reduced file set
    if mode == "changed":
        checks = FULL_CHECKS

    try:
        for check in checks:
            findings = check(ctx)
            report.findings.extend(findings)
        report.claims = list(getattr(ctx, "claims", []) or [])
        report.dimensions = compute_dimensions(report.findings)
        report.zero_fake_counts = classify_counts(report.findings)
        report.root_cause_summary = aggregate_root_causes(report.findings)
        report.finished_at = utc_now()
    except Exception as exc:  # noqa: BLE001
        report.auditor_error = f"{type(exc).__name__}: {exc}"
        report.finished_at = utc_now()
        report.dimensions = {k: "ERROR" for k in DIM_MAP}
        report.dimensions["ZERO_FAKE_WORKSPACE_GATE"] = "ERROR"
        write_artifacts(report, out_dir)
        link_latest(out_dir)
        print(render_human(report, out_dir))
        return report

    write_artifacts(report, out_dir)
    link_latest(out_dir)
    print(render_human(report, out_dir))
    return report


def render_human(report: AuditReport, out_dir: Path) -> str:
    from model import render_summary

    body = render_summary(report)
    return body + f"\n\nARTIFACTS = {out_dir}\nRULE_ID = {RULE_ID}\n"


def _scan_fixture_tree(case_dir: Path, as_zone: str) -> List[Finding]:
    """Run quick-relevant checks against a fixture directory with synthetic paths."""
    # Build a temporary ScanContext rooted at fixture parent, but rewrite paths
    root = case_dir
    ctx = ScanContext(root=root, mode="selftest")
    # Import individual checks that matter for fixtures
    from checks.placeholder import run as ph
    from checks.fake_success import run as fk
    from checks.test_integrity import run as ti
    from checks.config_effect import run as cfg
    from checks.bench_path import run as bp
    from checks.security_bypass import run as sec
    from checks.zero_fake import run as zf

    findings: List[Finding] = []
    for check in (zf, ph, fk, ti, cfg, bp, sec):
        raw = check(ctx)
        for f in raw:
            f.path = f"fixture/{case_dir.name}/{f.path}"
            findings.append(f)
    return findings


def run_selftest(fixtures: Path) -> int:
    errors: List[str] = []
    must_fail = fixtures / "must-fail"
    must_pass = fixtures / "must-pass"
    must_review = fixtures / "must-review"

    def blocking(findings: List[Finding]) -> int:
        return sum(1 for f in findings if f.severity in {"CRITICAL", "HIGH"})

    def reviewish(findings: List[Finding]) -> int:
        return sum(1 for f in findings if f.severity in {"REVIEW", "HIGH", "CRITICAL", "MEDIUM"})

    for case in sorted(must_fail.iterdir()) if must_fail.is_dir() else []:
        if not case.is_dir():
            continue
        findings = _scan_fixture_tree(case, "PRODUCT")
        tree_blocking = blocking(findings)
        # Overlay covers todo!/|| true/BENCH shortcuts; mock tokens must be
        # detected by the real checks, not by a substring second oracle.
        findings = _force_product_zone_findings(case, findings)
        if tree_blocking < 1:
            errors.append(
                f"must-fail/{case.name}: expected CRITICAL/HIGH from detector, got 0 "
                f"(overlay-only does not count)"
            )
            for f in findings:
                print(f"  debug {f.severity} {f.check} {f.claim} :: {f.evidence[:80]}")
        else:
            print(f"PASS_EXPECT_FAIL {case.name} blocking={tree_blocking}")

    for case in sorted(must_pass.iterdir()) if must_pass.is_dir() else []:
        if not case.is_dir():
            continue
        findings = _scan_fixture_tree(case, "TEST")
        findings = _force_test_zone_findings(case, findings)
        if blocking(findings) > 0:
            errors.append(
                f"must-pass/{case.name}: unexpected blocking={blocking(findings)}"
            )
            for f in findings:
                if f.severity in {"CRITICAL", "HIGH"}:
                    print(f"  debug {f.severity} {f.check} {f.path}:{f.line} {f.evidence[:80]}")
        else:
            print(f"PASS_EXPECT_PASS {case.name}")

    for case in sorted(must_review.iterdir()) if must_review.is_dir() else []:
        if not case.is_dir():
            continue
        findings = _scan_fixture_tree(case, "PRODUCT")
        findings = _force_product_zone_findings(case, findings)
        if reviewish(findings) < 1:
            errors.append(f"must-review/{case.name}: expected REVIEW/MEDIUM+, got none")
            for f in findings:
                print(f"  debug {f.severity} {f.check} {f.evidence[:80]}")
        else:
            print(f"PASS_EXPECT_REVIEW {case.name} reviewish={reviewish(findings)}")

    if errors:
        for e in errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print("AUDITOR_SELFTEST = FAIL")
        print(f"RULE_ID={RULE_ID}")
        return 1

    from token_context import selftest_classifier

    clf_errors = selftest_classifier()
    if clf_errors:
        for e in clf_errors:
            print(f"ERROR: token_context: {e}", file=sys.stderr)
        print("AUDITOR_SELFTEST = FAIL")
        print(f"RULE_ID={RULE_ID}")
        return 1
    print("PASS_TOKEN_CONTEXT_CLASSIFIER")
    print("AUDITOR_SELFTEST = PASS")
    print(f"RULE_ID={RULE_ID}")
    return 0


def _force_product_zone_findings(case: Path, findings: List[Finding]) -> List[Finding]:
    """Re-scan fixture files as if they lived under core/src/ for must-fail cases."""
    from checks.placeholder import run as ph
    from checks.fake_success import run as fk
    from checks.test_integrity import run as ti
    from checks.bench_path import run as bp
    from checks.config_effect import run as cfg

    # Create synthetic tree: copy content into memory classification override
    # Simpler approach: directly pattern-match fixture files with product rules.
    out: List[Finding] = list(findings)
    ctx = ScanContext(root=case, mode="selftest")
    for path in case.rglob("*"):
        if not path.is_file():
            continue
        rel = str(path.relative_to(case))
        text = read_text(path)
        # Product todo!/unimplemented!
        for i, line in enumerate(text.splitlines(), 1):
            if "todo!(" in line or "unimplemented!(" in line:
                out.append(
                    Finding(
                        id=ctx.next_id("PH"),
                        severity="CRITICAL" if "unimplemented!" in line else "HIGH",
                        category="PLACEHOLDER_INCOMPLETE",
                        path=f"core/src/{rel}",
                        line=i,
                        claim="product placeholder",
                        evidence=line.strip()[:200],
                        why_it_matters="fixture",
                        confidence="HIGH",
                        recommended_action="n/a",
                        classification="CRITICAL_INTEGRITY_VIOLATION",
                        check="placeholder",
                    )
                )
            if "|| true" in line or re_exit0(line) or "HARDCODED_SUCCESS" in line or "fake_success" in line:
                out.append(
                    Finding(
                        id=ctx.next_id("FAKE"),
                        severity="HIGH",
                        category="HARDCODED_SUCCESS",
                        path=f"scripts/{rel}",
                        line=i,
                        claim="hardcoded success",
                        evidence=line.strip()[:200],
                        why_it_matters="fixture",
                        confidence="HIGH",
                        recommended_action="n/a",
                        classification="CRITICAL_INTEGRITY_VIOLATION",
                        check="fake_success",
                    )
                )
            if "BENCH_API_CACHE_PATHS" in line:
                out.append(
                    Finding(
                        id=ctx.next_id("BENCH"),
                        severity="CRITICAL",
                        category="PRODUCT_VS_BENCH_PATH",
                        path=f"crates/exyonq-mod-proxy/{rel}",
                        line=i,
                        claim="bench shortcut",
                        evidence=line.strip()[:200],
                        why_it_matters="fixture",
                        confidence="HIGH",
                        recommended_action="n/a",
                        classification="CRITICAL_INTEGRITY_VIOLATION",
                        check="bench_path",
                    )
                )
            if "mock" in line.lower():
                from token_context import line_has_executable_token

                if not line_has_executable_token(text, rel, i, line):
                    continue
                sev = "CRITICAL" if ("e2e" in rel.lower() or "e2e" in text[:200].lower()) else "HIGH"
                out.append(
                    Finding(
                        id=ctx.next_id("ZF"),
                        severity=sev,
                        category="ZERO_FAKE",
                        path=f"tests/{rel}",
                        line=i,
                        claim="forbidden mock",
                        evidence=line.strip()[:200],
                        why_it_matters="fixture",
                        confidence="HIGH",
                        recommended_action="n/a",
                        classification="FORBIDDEN_MOCK",
                        check="zero_fake",
                    )
                )
            if "CONFIG_PARSED_BUT_UNUSED" in line:
                out.append(
                    Finding(
                        id=ctx.next_id("CFG"),
                        severity="HIGH",
                        category="CONFIG_EFFECTIVENESS",
                        path=rel,
                        line=i,
                        claim="config unused",
                        evidence=line.strip()[:200],
                        why_it_matters="fixture",
                        confidence="HIGH",
                        recommended_action="n/a",
                        classification="PRODUCT_PATH_FAKE",
                        check="config_effect",
                    )
                )
    # Drop INFO-only noise from first pass for must-fail evaluation
    return out


def re_exit0(line: str) -> bool:
    import re

    return bool(re.match(r"^\s*exit\s+0\s*$", line))


def _force_test_zone_findings(case: Path, findings: List[Finding]) -> List[Finding]:
    """Keep only unexpected product-path fakes for must-pass cases."""
    kept = []
    for f in findings:
        # Safe unit mock should not produce CRITICAL/HIGH from test_integrity/placeholder
        if f.severity in {"CRITICAL", "HIGH"}:
            # Allow if classification says SAFE
            if f.classification == "REVIEW_REQUIRED":
                continue
            kept.append(f)
    # Additionally ensure docs historical TODO does not block
    ctx = ScanContext(root=case, mode="selftest")
    for path in case.rglob("*"):
        if not path.is_file():
            continue
        text = read_text(path)
        if "mock" in text.lower() and "#[cfg(test)]" in text:
            # expected safe — ensure we don't mark blocking
            pass
        if "TODO" in text and path.suffix == ".md":
            # docs TODO must not be blocking — strip any HIGH from it
            kept = [f for f in kept if not (f.path.endswith(path.name) and "TODO" in f.evidence)]
    return kept


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description="EXYONQ CODE INTEGRITY AUDITOR")
    p.add_argument(
        "mode",
        choices=("full", "quick", "changed", "selftest"),
        help="Audit mode",
    )
    p.add_argument("--root", default=".", help="Repository root")
    p.add_argument("--out", default="", help="Output directory")
    p.add_argument("--baseline", default="main", help="Git baseline for --changed")
    p.add_argument("--fixtures", default="", help="Fixtures dir for --selftest")
    args = p.parse_args(argv)

    if args.mode == "selftest":
        fixtures = Path(args.fixtures).resolve() if args.fixtures else (
            Path(__file__).resolve().parents[1] / "fixtures"
        )
        return run_selftest(fixtures)

    root = Path(args.root).resolve()
    if not root.is_dir():
        print(f"ERROR: not a directory: {root}", file=sys.stderr)
        return 2
    out_dir = pick_out_dir(root, args.out or None)
    out_dir.mkdir(parents=True, exist_ok=True)
    report = run_audit(args.mode, root, out_dir, baseline=args.baseline)
    return report.exit_code()


if __name__ == "__main__":
    sys.exit(main())
