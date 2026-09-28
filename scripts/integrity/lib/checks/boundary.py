#!/usr/bin/env python3
"""Check 1 — repository boundary / external path references."""
from __future__ import annotations

import os
from pathlib import Path
from typing import List

from context import (
    EXTERNAL_PATH_RE,
    ScanContext,
    git_meta,
    iter_files,
    read_text,
    rel_of,
    run_git,
)
from model import Finding


def run(ctx: ScanContext) -> List[Finding]:
    findings: List[Finding] = []
    root = ctx.root
    meta = git_meta(root)

    if not meta["head_ok"] or not meta["toplevel"]:
        findings.append(
            Finding(
                id=ctx.next_id("BOUND"),
                severity="CRITICAL",
                category="REPOSITORY_BOUNDARY",
                path=".",
                line=0,
                claim="Workspace is a valid Git repository",
                evidence="git rev-parse failed",
                why_it_matters="Without a Git root, audit provenance is undefined.",
                confidence="HIGH",
                recommended_action="Run only inside the authorized Git worktree.",
                classification="CRITICAL_INTEGRITY_VIOLATION",
                check="boundary",
            )
        )
        return findings

    toplevel = Path(meta["toplevel"]).resolve()
    if toplevel != root.resolve():
        findings.append(
            Finding(
                id=ctx.next_id("BOUND"),
                severity="HIGH",
                category="REPOSITORY_BOUNDARY",
                path=".",
                line=0,
                claim="Audit root equals Git toplevel",
                evidence=f"toplevel={toplevel} root={root.resolve()}",
                why_it_matters="Scanning a subdirectory can miss product-path integrity issues.",
                confidence="HIGH",
                recommended_action="Invoke auditor from repository root.",
                classification="REVIEW_REQUIRED",
                check="boundary",
            )
        )

    # Symlinks escaping workspace
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        try:
            if path.is_symlink():
                target = path.resolve()
                if not str(target).startswith(str(root.resolve())):
                    findings.append(
                        Finding(
                            id=ctx.next_id("BOUND"),
                            severity="HIGH",
                            category="REPOSITORY_BOUNDARY",
                            path=rel_of(root, path),
                            line=0,
                            claim="Symlink stays inside workspace",
                            evidence=f"symlink -> {target}",
                            why_it_matters="Outbound symlinks can pull private/external material into builds.",
                            confidence="HIGH",
                            recommended_action="Remove or retarget symlink inside workspace.",
                            classification="CRITICAL_INTEGRITY_VIOLATION",
                            check="boundary",
                        )
                    )
        except OSError:
            continue

    # Scripts referencing external absolute paths (exclude this auditor's docs/examples)
    for path in iter_files(root, ctx.changed_paths if ctx.changed_only else None):
        rel = rel_of(root, path)
        zone = ctx.classify(rel).zone
        if zone == "DOC":
            continue
        # Scanners/gates that detect external paths will contain the patterns by design.
        if "scan" in rel or "/gates/" in rel or rel.endswith("verify-no-private-paths.sh"):
            continue
        text = read_text(path)
        for i, line in enumerate(text.splitlines(), 1):
            # Skip comments/echo that document forbidden paths.
            if line.lstrip().startswith("#") or "echo " in line or "re.compile" in line:
                continue
            m = EXTERNAL_PATH_RE.search(line)
            if not m:
                continue
            if "/Volumes/Lexar/Cursor/exyonq-laboratorio" in line:
                continue
            severity = "HIGH" if zone in {"PRODUCT", "BENCH"} else "REVIEW"
            findings.append(
                Finding(
                    id=ctx.next_id("BOUND"),
                    severity=severity,
                    category="REPOSITORY_BOUNDARY",
                    path=rel,
                    line=i,
                    claim="No external absolute path dependencies in operational scripts",
                    evidence=line.strip()[:200],
                    why_it_matters="External paths break reproducibility and can leak private host layout.",
                    confidence="MEDIUM",
                    recommended_action="Use repo-relative paths or documented host aliases only.",
                    classification="REVIEW_REQUIRED",
                    check="boundary",
                )
            )

    # Submodules
    sub = run_git(root, ["submodule", "status"])
    if sub.returncode == 0 and sub.stdout.strip():
        findings.append(
            Finding(
                id=ctx.next_id("BOUND"),
                severity="INFO",
                category="REPOSITORY_BOUNDARY",
                path=".gitmodules",
                line=1,
                claim="Submodules present — verify they are intentional",
                evidence=sub.stdout.strip()[:300],
                why_it_matters="Submodules expand the trust boundary of the audit.",
                confidence="HIGH",
                recommended_action="Confirm submodule pins and scan each separately.",
                classification="REVIEW_REQUIRED",
                check="boundary",
            )
        )

    # Record boundary meta as INFO (not a finding of failure)
    findings.append(
        Finding(
            id=ctx.next_id("BOUND"),
            severity="INFO",
            category="REPOSITORY_BOUNDARY",
            path=".",
            line=0,
            claim="Repository boundary metadata",
            evidence=(
                f"branch={meta['branch']} head={meta['head'][:12]} "
                f"dirty={'YES' if meta['dirty'] else 'NO'} cwd={os.getcwd()}"
            ),
            why_it_matters="Provenance for interpreting later findings.",
            confidence="HIGH",
            recommended_action="Retain in audit evidence package.",
            classification="REVIEW_REQUIRED",
            check="boundary",
        )
    )
    return findings
