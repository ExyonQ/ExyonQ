#!/usr/bin/env python3
"""Path zoning and scan helpers for EXYONQ CODE INTEGRITY AUDITOR."""
from __future__ import annotations

import os
import re
import subprocess
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Iterator, List, Optional, Sequence, Set


SKIP_DIR_NAMES = {
    ".git",
    "target",
    "node_modules",
    ".exyonq-local",
    "__pycache__",
    ".venv",
    "venv",
    ".sccache",
    "rivals-cache",
    "graphify-out",
}

SKIP_REL_PREFIXES = (
    "benchmarks/results/",
    "benchmarks/bv04/runs/",
    "benchmarks/bv04/reports/",
    "benchmarks/bv04/seals/",
    "benchmarks/rivals-cache/",
    # Auditor self-source and fixtures must not scan themselves as product findings.
    "scripts/integrity/",
    "scripts/gates/fixtures/",
    ".exyonq-local/",
)

TEXT_SUFFIXES = {
    ".rs",
    ".toml",
    ".sh",
    ".py",
    ".md",
    ".yml",
    ".yaml",
    ".json",
    ".txt",
    ".conf",
    ".cfg",
    ".html",
    ".c",
    ".h",
    ".go",
    ".js",
    ".ts",
}

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

TEST_PATH_RE = re.compile(
    r"(^|/)(tests?/|benches/|fuzz/|examples/)|_test\.rs$|/test_[^/]+\.rs$|"
    r"tests\.rs$|integration[_-]tests?/",
    re.I,
)
BENCH_PATH_RE = re.compile(r"(^|/)(benchmarks?/|benches?/|perf/)", re.I)
DOC_PATH_RE = re.compile(r"(^|/)(docs?/|\.cursor/|study/|README|CHANGELOG|CONTRIBUTING)", re.I)
TOOL_PATH_RE = re.compile(r"(^|/)(scripts?/|tools?/|xtask/)", re.I)
SMOKE_PATH_RE = re.compile(r"(^|/)smoke(/|$)|smoke[-_]", re.I)

EXTERNAL_PATH_RE = re.compile(
    r"(?:^|[\"'\s=])("
    r"/Volumes/(?!Lexar/Cursor/exyonq-laboratorio(?:/|\"|'|\s|$))"
    r"|/Users/"
    r"|/home/"
    r"|/root/(?!exyonq)"
    r"|~/Library"
    r")"
)


@dataclass
class PathZone:
    rel: str
    zone: str  # PRODUCT | TEST | BENCH | DOC | TOOL | SMOKE | OTHER | FIXTURE


@dataclass
class ScanContext:
    root: Path
    mode: str
    changed_only: bool = False
    changed_paths: Set[str] = field(default_factory=set)
    finding_seq: int = 0

    def next_id(self, prefix: str) -> str:
        self.finding_seq += 1
        return f"{prefix}-{self.finding_seq:04d}"

    def classify(self, rel: str) -> PathZone:
        norm = rel.replace("\\", "/")
        if "scripts/integrity/fixtures/" in norm:
            return PathZone(norm, "FIXTURE")
        if SMOKE_PATH_RE.search(norm):
            return PathZone(norm, "SMOKE")
        if TEST_PATH_RE.search(norm):
            return PathZone(norm, "TEST")
        if BENCH_PATH_RE.search(norm):
            return PathZone(norm, "BENCH")
        if DOC_PATH_RE.search(norm):
            return PathZone(norm, "DOC")
        if TOOL_PATH_RE.search(norm):
            return PathZone(norm, "TOOL")
        if any(norm.startswith(p) for p in PRODUCT_PREFIXES):
            return PathZone(norm, "PRODUCT")
        return PathZone(norm, "OTHER")

    def is_product_like(self, zone: str) -> bool:
        return zone in {"PRODUCT", "OTHER"}


def rel_of(root: Path, path: Path) -> str:
    try:
        return str(path.resolve().relative_to(root.resolve())).replace("\\", "/")
    except Exception:
        return str(path).replace("\\", "/")


def should_skip_dir(name: str) -> bool:
    return name in SKIP_DIR_NAMES


def should_skip_rel(rel: str) -> bool:
    norm = rel.replace("\\", "/")
    if any(norm.startswith(p) or f"/{p}" in f"/{norm}" for p in SKIP_REL_PREFIXES):
        return True
    if "/target/" in f"/{norm}/":
        return True
    return False


def iter_files(root: Path, changed: Optional[Set[str]] = None) -> Iterator[Path]:
    root = root.resolve()
    if changed is not None:
        for rel in sorted(changed):
            p = root / rel
            if p.is_file() and not should_skip_rel(rel):
                yield p
        return
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not should_skip_dir(d)]
        base = Path(dirpath)
        for name in filenames:
            path = base / name
            rel = rel_of(root, path)
            if should_skip_rel(rel):
                continue
            if path.suffix.lower() not in TEXT_SUFFIXES and name not in {
                "Dockerfile",
                "Makefile",
                "Cargo.toml",
            }:
                # still allow extensionless shell scripts with shebang later via name
                if not name.endswith(".sh") and "." in name:
                    continue
            yield path


def read_text(path: Path, max_bytes: int = 2_000_000) -> str:
    try:
        data = path.read_bytes()
    except OSError:
        return ""
    if len(data) > max_bytes:
        data = data[:max_bytes]
    if b"\x00" in data[:4096]:
        return ""
    return data.decode("utf-8", errors="replace")


def run_git(root: Path, args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["git", *args],
        cwd=str(root),
        text=True,
        capture_output=True,
        check=False,
    )


def git_meta(root: Path) -> dict:
    head = run_git(root, ["rev-parse", "HEAD"])
    branch = run_git(root, ["branch", "--show-current"])
    dirty = run_git(root, ["status", "--porcelain"])
    toplevel = run_git(root, ["rev-parse", "--show-toplevel"])
    return {
        "head": head.stdout.strip() if head.returncode == 0 else "UNKNOWN",
        "branch": branch.stdout.strip() if branch.returncode == 0 else "UNKNOWN",
        "dirty": bool(dirty.stdout.strip()) if dirty.returncode == 0 else True,
        "toplevel": toplevel.stdout.strip() if toplevel.returncode == 0 else "",
        "head_ok": head.returncode == 0,
    }


def changed_files(root: Path, baseline: str = "main") -> Set[str]:
    """Files changed vs merge-base with baseline (or HEAD~1 fallback)."""
    mb = run_git(root, ["merge-base", "HEAD", baseline])
    base = mb.stdout.strip() if mb.returncode == 0 else ""
    if not base:
        prev = run_git(root, ["rev-parse", "HEAD~1"])
        base = prev.stdout.strip() if prev.returncode == 0 else "HEAD"
    diff = run_git(root, ["diff", "--name-only", f"{base}...HEAD"])
    unstaged = run_git(root, ["diff", "--name-only"])
    staged = run_git(root, ["diff", "--name-only", "--cached"])
    untracked = run_git(root, ["ls-files", "--others", "--exclude-standard"])
    paths: Set[str] = set()
    for blob in (diff.stdout, unstaged.stdout, staged.stdout, untracked.stdout):
        for line in blob.splitlines():
            line = line.strip()
            if line:
                paths.add(line.replace("\\", "/"))
    return paths


def dim_status(findings: Iterable, categories: Sequence[str]) -> str:
    sev_rank = {"CRITICAL": 0, "HIGH": 1, "REVIEW": 2, "MEDIUM": 3, "LOW": 4, "INFO": 5}
    worst = None
    for f in findings:
        if f.category not in categories and f.check not in categories:
            # also match by check name prefix
            if not any(str(f.check).startswith(c) or str(f.category).startswith(c) for c in categories):
                continue
        rank = sev_rank.get(f.severity, 9)
        if worst is None or rank < worst:
            worst = rank
    if worst is None:
        return "PASS"
    if worst <= 1:
        return "FAIL"
    if worst == 2:
        return "REVIEW"
    return "PASS"
