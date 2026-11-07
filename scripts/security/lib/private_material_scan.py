#!/usr/bin/env python3
# EXYONQ-SEC-PRIVATE-MATERIAL-ZERO — content scanner engine (FAIL_CLOSED).
# Never prints secret bodies — only relative/absolute offending paths + rule ids.
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Iterable, Iterator, List, Optional, Sequence, Set, Tuple

RULE_ID = "EXYONQ-SEC-PRIVATE-MATERIAL-ZERO"
FAIL_MODE = "FAIL_CLOSED"


def _b(*parts: str) -> bytes:
    """Build needle without storing contiguous forbidden literals in source."""
    return "".join(parts).encode("ascii")


def _compile_rules() -> List[Tuple[str, re.Pattern[bytes]]]:
    # Fragments intentionally split so this source file does not self-match.
    begin = "-----" + "BEGIN "
    enddash = "-----"
    priv = "PRIVATE " + "KEY"
    return [
        (
            "PEM_PRIVATE_KEY",
            re.compile(
                _b(begin, "(?:ENCRYPTED )?", priv, enddash)
            ),
        ),
        (
            "PEM_RSA_PRIVATE_KEY",
            re.compile(_b(begin, "RSA ", priv, enddash)),
        ),
        (
            "PEM_EC_PRIVATE_KEY",
            re.compile(_b(begin, "EC ", priv, enddash)),
        ),
        (
            "PEM_DSA_PRIVATE_KEY",
            re.compile(_b(begin, "DSA ", priv, enddash)),
        ),
        (
            "OPENSSH_PRIVATE_KEY",
            re.compile(_b(begin, "OPENSSH ", priv, enddash)),
        ),
        (
            "PGP_PRIVATE_KEY",
            re.compile(_b(begin, "PGP ", priv, " BLOCK", enddash)),
        ),
        (
            "PUTTY_PRIVATE_KEY",
            re.compile(_b("PuTTY", "-User-", "Key-File-")),
        ),
        (
            "GITHUB_PAT",
            re.compile(
                br"(?:ghp_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})"
            ),
        ),
        ("AWS_ACCESS_KEY_ID", re.compile(br"AKIA[0-9A-Z]{16}")),
        (
            "BEARER_TOKEN",
            re.compile(
                br"(?i)authorization\s*[:=]\s*bearer\s+[A-Za-z0-9\-._~+/]+=*"
            ),
        ),
        (
            "DOCKER_AUTH_JSON",
            re.compile(br'"auth"\s*:\s*"[A-Za-z0-9+/=]{20,}"'),
        ),
        (
            "SIGNING_VOLUME_PATH",
            re.compile(br"(?i)/Volumes/[^/\s]+/EXYONQ-SIGNING-A"),
        ),
        (
            "ENV_SECRET_ASSIGN",
            re.compile(
                br"(?i)^(?:export\s+)?(?:AWS_SECRET_ACCESS_KEY|COSIGN_PASSWORD|GH_TOKEN|GITHUB_TOKEN)\s*="
            ),
        ),
    ]


CONTENT_RULES: List[Tuple[str, re.Pattern[bytes]]] = _compile_rules()

SUSPICIOUS_NAME_RE = re.compile(
    r"(?i)(^id_rsa$|^id_dsa$|^id_ecdsa$|^id_ed25519$|private_key|_private\.(?:pem|key)$|"
    r"server\.key$|client\.key$|cosign\.key$|release-private\.key$|\.p12$|\.pfx$|\.jks$|"
    r"\.keystore$|\.pem$|\.key$)"
)

ARCHIVE_SUFFIXES = (
    ".tar",
    ".tar.gz",
    ".tgz",
    ".tar.xz",
    ".tar.bz2",
    ".zip",
    ".gz",
    ".xz",
    ".deb",
    ".rpm",
)

SKIP_DIR_NAMES = {
    ".git",
    "target",
    "node_modules",
    ".exyonq-local",
    "__pycache__",
    ".venv",
    "venv",
}

MAX_FILE_READ = 8 * 1024 * 1024  # 8 MiB content sample + full for small files
BINARY_STRING_MIN = 4


def eprint(*args: object) -> None:
    print(*args, file=sys.stderr)


def should_skip_dir(name: str) -> bool:
    return name in SKIP_DIR_NAMES


def is_archive(path: Path) -> bool:
    n = path.name.lower()
    return any(n.endswith(s) for s in ARCHIVE_SUFFIXES)


def read_sample(path: Path) -> bytes:
    try:
        size = path.stat().st_size
    except OSError:
        return b""
    try:
        with path.open("rb") as fh:
            if size <= MAX_FILE_READ:
                return fh.read()
            return fh.read(MAX_FILE_READ)
    except OSError:
        return b""


def match_content(data: bytes) -> List[str]:
    hits: List[str] = []
    for rule, pat in CONTENT_RULES:
        if pat.search(data):
            hits.append(rule)
    return hits


def extract_c_strings(data: bytes) -> bytes:
    # Join printable ASCII runs so PEM headers inside binaries are visible.
    out = bytearray()
    run = bytearray()
    for b in data:
        if 32 <= b < 127:
            run.append(b)
        else:
            if len(run) >= BINARY_STRING_MIN:
                out.extend(run)
                out.append(10)  # newline separator
            run.clear()
    if len(run) >= BINARY_STRING_MIN:
        out.extend(run)
    return bytes(out)


def scan_file(
    path: Path,
    findings: List[Tuple[str, str, str]],
    origin: str,
    display: Optional[str] = None,
) -> None:
    if not path.is_file() or path.is_symlink():
        return
    name = path.name
    data = read_sample(path)
    hits = match_content(data)
    if not hits and path.suffix.lower() in {".so", ".dylib", ".a", ".o", ".bin", ""}:
        # Also strings-scan likely binaries / extensionless blobs when large enough
        if data and (b"\0" in data[:1024] or path.stat().st_size > 4096):
            hits = match_content(extract_c_strings(data))
    elif not hits and b"\0" in data[:512]:
        hits = match_content(extract_c_strings(data))

    # Suspicious names without content match still get a content re-check note —
    # only fail on content (or known private basenames that are never public).
    base_l = name.lower()
    if base_l in {"cosign.key", "release-private.key", "gpg_private_key"} or base_l.endswith(
        ".private"
    ):
        hits = hits or ["SUSPICIOUS_PRIVATE_BASENAME"]

    shown = display if display is not None else str(path)
    for h in hits:
        findings.append((origin, shown, h))


def extract_archive(archive: Path, dest: Path) -> bool:
    dest.mkdir(parents=True, exist_ok=True)
    name = archive.name.lower()
    try:
        if name.endswith(".zip"):
            subprocess.run(
                ["unzip", "-qq", "-o", str(archive), "-d", str(dest)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return True
        if name.endswith(".deb"):
            subprocess.run(
                ["dpkg-deb", "-x", str(archive), str(dest)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return True
        if name.endswith(".rpm"):
            # Extract via rpm2cpio | cpio when available
            with tempfile.TemporaryDirectory() as td:
                cpio = Path(td) / "payload.cpio"
                with cpio.open("wb") as out:
                    subprocess.run(
                        ["rpm2cpio", str(archive)],
                        check=True,
                        stdout=out,
                        stderr=subprocess.DEVNULL,
                    )
                subprocess.run(
                    ["cpio", "-idm"],
                    check=True,
                    cwd=str(dest),
                    stdin=cpio.open("rb"),
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            return True
        # tar family (including .gz/.xz single-file and tarballs)
        if any(
            name.endswith(s)
            for s in (".tar", ".tar.gz", ".tgz", ".tar.xz", ".tar.bz2", ".gz", ".xz")
        ):
            # Prefer tar auto-detect
            subprocess.run(
                ["tar", "-xf", str(archive), "-C", str(dest)],
                check=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            return True
    except (subprocess.CalledProcessError, FileNotFoundError, OSError):
        # Fall back: if gzip/xz single file, decompress raw
        try:
            if name.endswith(".gz") and not name.endswith(".tar.gz"):
                out = dest / archive.stem
                subprocess.run(
                    ["gzip", "-dc", str(archive)],
                    check=True,
                    stdout=out.open("wb"),
                    stderr=subprocess.DEVNULL,
                )
                return True
            if name.endswith(".xz") and not name.endswith(".tar.xz"):
                out = dest / archive.stem
                subprocess.run(
                    ["xz", "-dc", str(archive)],
                    check=True,
                    stdout=out.open("wb"),
                    stderr=subprocess.DEVNULL,
                )
                return True
        except (subprocess.CalledProcessError, FileNotFoundError, OSError):
            eprint(f"WARN: could not extract archive for scan: {archive}")
            return False
    return False


def iter_files(root: Path) -> Iterator[Path]:
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if not should_skip_dir(d)]
        for fn in filenames:
            yield Path(dirpath) / fn


def scan_tree(root: Path, findings: List[Tuple[str, str, str]], origin: str) -> None:
    for path in iter_files(root):
        if is_archive(path):
            with tempfile.TemporaryDirectory(prefix="exyonq-pmz-arch-") as td:
                td_path = Path(td)
                if extract_archive(path, td_path):
                    scan_tree(td_path, findings, f"{origin}|archive:{path}")
                # Always also scan the archive bytes themselves (embedded PEM)
                scan_file(path, findings, origin)
        else:
            scan_file(path, findings, origin)


def scan_oci(root: Path, findings: List[Tuple[str, str, str]]) -> None:
    """Scan an OCI layout directory or a docker-save / image tar."""
    if root.is_file() and is_archive(root):
        with tempfile.TemporaryDirectory(prefix="exyonq-pmz-oci-") as td:
            td_path = Path(td)
            if extract_archive(root, td_path):
                scan_oci(td_path, findings)
            scan_file(root, findings, "oci-archive")
        return
    # OCI layout: blobs/sha256/*
    blobs = root / "blobs" / "sha256"
    if blobs.is_dir():
        for blob in blobs.iterdir():
            if blob.is_file():
                # Treat each blob as potential layer tar
                with tempfile.TemporaryDirectory(prefix="exyonq-pmz-layer-") as td:
                    td_path = Path(td)
                    if extract_archive(blob, td_path) or True:
                        # Always content-scan blob; try extract as tar
                        try:
                            subprocess.run(
                                ["tar", "-xf", str(blob), "-C", str(td_path)],
                                check=True,
                                stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL,
                            )
                            scan_tree(td_path, findings, f"oci-layer:{blob.name}")
                        except (subprocess.CalledProcessError, FileNotFoundError, OSError):
                            scan_file(blob, findings, f"oci-blob:{blob.name}")
                scan_file(blob, findings, f"oci-blob-raw:{blob.name}")
    # config / manifests / history as text
    for name in ("index.json", "manifest.json", "oci-layout"):
        p = root / name
        if p.is_file():
            scan_file(p, findings, f"oci-meta:{name}")
    # docker-save unpacked: */layer.tar
    for path in iter_files(root):
        if path.name == "layer.tar" or path.name.endswith(".tar"):
            with tempfile.TemporaryDirectory(prefix="exyonq-pmz-dlayer-") as td:
                td_path = Path(td)
                if extract_archive(path, td_path):
                    scan_tree(td_path, findings, f"docker-layer:{path}")
            scan_file(path, findings, f"docker-layer-raw:{path}")
        elif path.suffix.lower() in {".json", ".txt", ".yml", ".yaml", ".env"}:
            scan_file(path, findings, f"oci-fs:{path}")
        else:
            scan_file(path, findings, f"oci-fs:{path}")


def git_ls_files(repo: Path, staged_only: bool = False) -> List[Path]:
    cmd = ["git", "-C", str(repo), "ls-files", "-z"]
    if staged_only:
        cmd = ["git", "-C", str(repo), "diff", "--cached", "--name-only", "-z", "--diff-filter=ACMR"]
    out = subprocess.check_output(cmd)
    paths: List[Path] = []
    for raw in out.split(b"\0"):
        if not raw:
            continue
        rel = raw.decode("utf-8", errors="surrogateescape")
        paths.append(repo / rel)
    return paths


def git_publication_files(repo: Path) -> List[Path]:
    """Return the files a normal `git add -A` + push would publish.

    This deliberately includes tracked paths (working tree) and untracked,
    non-ignored paths. Ignored paths are local-only by construction and must
    never be part of a publication review.
    """
    out = subprocess.check_output(
        [
            "git",
            "-C",
            str(repo),
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--deduplicate",
        ]
    )
    paths: List[Path] = []
    for raw in out.split(b"\0"):
        if not raw:
            continue
        rel = raw.decode("utf-8", errors="surrogateescape")
        paths.append(repo / rel)
    return paths


def git_index_entries(repo: Path) -> List[Tuple[str, str, int, str]]:
    """Return (mode, blob_sha, stage, relative_path) from the index."""
    out = subprocess.check_output(["git", "-C", str(repo), "ls-files", "-s", "-z"])
    entries: List[Tuple[str, str, int, str]] = []
    for raw in out.split(b"\0"):
        if not raw or b"\t" not in raw:
            continue
        meta, rel_raw = raw.split(b"\t", 1)
        parts = meta.split(b" ")
        if len(parts) != 3:
            continue
        entries.append(
            (
                parts[0].decode("ascii", errors="replace"),
                parts[1].decode("ascii", errors="replace"),
                int(parts[2].decode("ascii", errors="replace")),
                rel_raw.decode("utf-8", errors="surrogateescape"),
            )
        )
    return entries


def scan_index_entry(
    repo: Path,
    mode: str,
    blob_sha: str,
    stage: int,
    rel: str,
    findings: List[Tuple[str, str, str]],
) -> None:
    logical = str(repo / rel)
    if stage != 0:
        findings.append(("publication-index", logical, "UNMERGED_INDEX_ENTRY"))
        return
    if mode == "120000":
        findings.append(("publication-index", logical, "SYMLINK_IN_PUBLICATION_SET"))
        return
    if mode == "160000":
        # Gitlink: the commit object is published, not a file body.
        return
    try:
        data = subprocess.check_output(["git", "-C", str(repo), "cat-file", "blob", blob_sha])
    except subprocess.CalledProcessError:
        findings.append(("publication-index", logical, "INDEX_BLOB_UNREADABLE"))
        return
    with tempfile.TemporaryDirectory(prefix="exyonq-pmz-index-") as td:
        path = Path(td) / Path(rel).name
        path.write_bytes(data)
        if is_archive(path):
            with tempfile.TemporaryDirectory(prefix="exyonq-pmz-index-arch-") as atd:
                atd_path = Path(atd)
                if extract_archive(path, atd_path):
                    scan_tree(atd_path, findings, f"publication-index-archive:{logical}")
        scan_file(path, findings, f"publication-index:{rel}")


def git_diff_files(repo: Path, base: str) -> List[Path]:
    out = subprocess.check_output(
        ["git", "-C", str(repo), "diff", "--name-only", "-z", f"{base}...HEAD"]
    )
    paths: List[Path] = []
    for raw in out.split(b"\0"):
        if not raw:
            continue
        paths.append(repo / raw.decode())
    return paths


def report_and_exit(findings: Sequence[Tuple[str, str, str]]) -> int:
    print(f"RULE_ID={RULE_ID}")
    print(f"FAIL_MODE={FAIL_MODE}")
    print(f"PRIVATE_MATERIAL_FINDINGS={len(findings)}")
    for origin, path, rule in findings:
        eprint(f"ERROR: {RULE_ID}: {rule}: {path} (via {origin})")
    if findings:
        eprint("PRIVATE_MATERIAL_GATE=FAIL")
        eprint("COMMIT=BLOCKED")
        eprint("PUSH=BLOCKED")
        eprint("RELEASE_BUILD=BLOCKED")
        eprint("TAG_PUBLICATION=BLOCKED")
        eprint("GITHUB_RELEASE=BLOCKED")
        eprint("GHCR_PUBLIC_VISIBILITY=BLOCKED")
        return 1
    print("PRIVATE_MATERIAL_GATE=PASS")
    return 0


def cmd_tree(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    findings: List[Tuple[str, str, str]] = []
    scan_tree(root, findings, "tree")
    return report_and_exit(findings)


def cmd_git_index(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    findings: List[Tuple[str, str, str]] = []
    staged = {str(path) for path in git_ls_files(repo, staged_only=True)}
    entries = [
        entry
        for entry in git_index_entries(repo)
        if str(repo / entry[3]) in staged
    ]
    print(f"INDEX_FILES={len(entries)}")
    for mode, blob_sha, stage, rel in entries:
        scan_index_entry(repo, mode, blob_sha, stage, rel, findings)
    return report_and_exit(findings)


def cmd_git_tree(args: argparse.Namespace) -> int:
    if not getattr(args, "allow_blind_scan", False):
        eprint(
            "ERROR: git-tree is blind to untracked files and staged blobs; "
            "use --publication. --allow-blind-scan is diagnostic-only."
        )
        return 2
    repo = Path(args.repo).resolve()
    findings: List[Tuple[str, str, str]] = []
    for path in git_ls_files(repo, staged_only=False):
        if not path.is_file():
            continue
        if is_archive(path):
            with tempfile.TemporaryDirectory(prefix="exyonq-pmz-gt-") as td:
                td_path = Path(td)
                if extract_archive(path, td_path):
                    scan_tree(td_path, findings, f"git-archive:{path}")
            scan_file(path, findings, "git-tree")
        else:
            scan_file(path, findings, "git-tree")
    return report_and_exit(findings)


def cmd_git_diff(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    base = args.base
    findings: List[Tuple[str, str, str]] = []
    for path in git_diff_files(repo, base):
        if path.is_file():
            scan_file(path, findings, f"diff:{base}")
    return report_and_exit(findings)


def cmd_git_publication(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    findings: List[Tuple[str, str, str]] = []
    paths = git_publication_files(repo)
    print(f"PUBLICATION_FILES={len(paths)}")
    index_entries = git_index_entries(repo)
    print(f"PUBLICATION_INDEX_FILES={len(index_entries)}")
    for mode, blob_sha, stage, rel in index_entries:
        scan_index_entry(repo, mode, blob_sha, stage, rel, findings)
    for path in paths:
        if path.is_symlink():
            findings.append(("publication", str(path), "SYMLINK_IN_PUBLICATION_SET"))
            continue
        if not path.is_file():
            # A tracked deletion is not content in the publication set.
            continue
        if is_archive(path):
            with tempfile.TemporaryDirectory(prefix="exyonq-pmz-pub-") as td:
                td_path = Path(td)
                if extract_archive(path, td_path):
                    scan_tree(td_path, findings, f"publication-archive:{path}")
            scan_file(path, findings, "publication")
        else:
            scan_file(path, findings, "publication")
    return report_and_exit(findings)


def git_history_blobs(repo: Path) -> Tuple[List[str], dict[str, Tuple[str, str]]]:
    """Return all reachable commits and one (commit, path) per unique blob."""
    commits = subprocess.check_output(
        ["git", "-C", str(repo), "rev-list", "--all"], text=True
    ).splitlines()
    blobs: dict[str, Tuple[str, str]] = {}
    for commit in commits:
        out = subprocess.check_output(
            ["git", "-C", str(repo), "ls-tree", "-r", "-z", "--full-tree", commit]
        )
        for raw in out.split(b"\0"):
            if not raw or b"\t" not in raw:
                continue
            meta, rel_raw = raw.split(b"\t", 1)
            parts = meta.split(b" ")
            if len(parts) != 3 or parts[1] != b"blob":
                continue
            sha = parts[2].decode("ascii", errors="replace")
            rel = rel_raw.decode("utf-8", errors="surrogateescape")
            blobs.setdefault(sha, (commit, rel))
    return commits, blobs


def history_pem_is_pattern_source(data: bytes) -> bool:
    """True when PEM headers are only a grep/regex definition, not key material."""
    try:
        text = data.decode("utf-8", errors="replace")
    except Exception:
        return False
    key_lines = [line for line in text.splitlines() if "PRIVATE KEY" in line]
    if not key_lines:
        return False
    return all("^-----BEGIN" in line or "grep -qE" in line for line in key_lines)


def history_bearer_is_noncredential(data: bytes) -> bool:
    """True for placeholder or explicit redaction/test fixtures, never a real token."""
    tokens = re.findall(
        br"(?i)authorization\s*[:=]\s*bearer\s+(\S+)", data
    )
    if not tokens:
        return False
    markers = (b"redact", b"test", b"example", b"dummy", b"fake", b"placeholder")
    for token in tokens:
        lowered = token.lower()
        if any(marker in lowered for marker in markers):
            continue
        if b"metrics" in lowered and b"token" in lowered:
            continue
        if b"secret" in lowered and b"token" in lowered:
            continue
        return False
    return True


def cmd_history(args: argparse.Namespace) -> int:
    repo = Path(args.repo).resolve()
    raw_findings: List[Tuple[str, str, str]] = []
    blob_data: dict[str, bytes] = {}
    commits, blobs = git_history_blobs(repo)
    print(f"HISTORY_COMMITS={len(commits)}")
    print(f"HISTORY_BLOBS={len(blobs)}")
    for blob_sha, (commit, rel) in blobs.items():
        try:
            data = subprocess.check_output(
                ["git", "-C", str(repo), "cat-file", "blob", blob_sha]
            )
        except subprocess.CalledProcessError:
            raw_findings.append(("history", f"{commit}:{rel}", "HISTORY_BLOB_UNREADABLE"))
            continue
        with tempfile.TemporaryDirectory(prefix="exyonq-pmz-history-") as td:
            path = Path(td) / Path(rel).name
            path.write_bytes(data)
            display = f"{commit}:{rel}"
            blob_data[display] = data
            if is_archive(path):
                before = len(raw_findings)
                with tempfile.TemporaryDirectory(prefix="exyonq-pmz-history-arch-") as atd:
                    atd_path = Path(atd)
                    if extract_archive(path, atd_path):
                        scan_tree(atd_path, raw_findings, f"history-archive:{display}")
                for i in range(before, len(raw_findings)):
                    origin, _temp_path, rule = raw_findings[i]
                    raw_findings[i] = (origin, display, rule)
            scan_file(path, raw_findings, f"history:{display}", display=display)

    false_counts: dict[str, int] = {
        "PATTERN_SOURCE": 0,
        "TEST_OR_PLACEHOLDER": 0,
    }
    findings: List[Tuple[str, str, str]] = []
    for origin, path, rule in raw_findings:
        data = blob_data.get(path, b"")
        if rule in {
            "PEM_PRIVATE_KEY",
            "PEM_RSA_PRIVATE_KEY",
            "PEM_EC_PRIVATE_KEY",
            "PEM_DSA_PRIVATE_KEY",
            "OPENSSH_PRIVATE_KEY",
            "PGP_PRIVATE_KEY",
        } and history_pem_is_pattern_source(data):
            false_counts["PATTERN_SOURCE"] += 1
            print(f"HISTORY_NON_CREDENTIAL_HIT=PATTERN_SOURCE:{rule}:{path}")
            continue
        if rule == "BEARER_TOKEN" and history_bearer_is_noncredential(data):
            false_counts["TEST_OR_PLACEHOLDER"] += 1
            print(f"HISTORY_NON_CREDENTIAL_HIT=TEST_OR_PLACEHOLDER:{rule}:{path}")
            continue
        findings.append((origin, path, rule))

    print(f"RULE_ID={RULE_ID}")
    print(f"FAIL_MODE={FAIL_MODE}")
    print(f"HISTORY_RAW_FINDINGS={len(raw_findings)}")
    print(f"HISTORY_FALSE_POSITIVES={sum(false_counts.values())}")
    print(f"HISTORY_PATTERN_SOURCE_HITS={false_counts['PATTERN_SOURCE']}")
    print(f"HISTORY_TEST_OR_PLACEHOLDER_HITS={false_counts['TEST_OR_PLACEHOLDER']}")
    print(f"PRIVATE_MATERIAL_FINDINGS={len(findings)}")
    for origin, path, rule in findings:
        eprint(f"ERROR: {RULE_ID}: {rule}: {path} (via {origin})")
    if findings:
        eprint("HISTORY_EXPOSURE=YES")
        eprint("HISTORY_ROTATION_REQUIRED=YES")
        eprint("PRIVATE_MATERIAL_GATE=FAIL")
        eprint("HISTORY_REWRITE_AUTHORIZED=NO")
        return 1
    print("HISTORY_EXPOSURE=NO")
    print("HISTORY_ROTATION_REQUIRED=NO")
    print("PRIVATE_MATERIAL_GATE=PASS")
    return 0


def cmd_archive(args: argparse.Namespace) -> int:
    archive = Path(args.archive).resolve()
    findings: List[Tuple[str, str, str]] = []
    with tempfile.TemporaryDirectory(prefix="exyonq-pmz-arc-") as td:
        td_path = Path(td)
        if extract_archive(archive, td_path):
            scan_tree(td_path, findings, f"archive:{archive}")
        scan_file(archive, findings, "archive-raw")
    return report_and_exit(findings)


def cmd_oci(args: argparse.Namespace) -> int:
    root = Path(args.oci).resolve()
    findings: List[Tuple[str, str, str]] = []
    scan_oci(root, findings)
    return report_and_exit(findings)


def cmd_assets(args: argparse.Namespace) -> int:
    root = Path(args.assets).resolve()
    findings: List[Tuple[str, str, str]] = []
    scan_tree(root, findings, "assets")
    return report_and_exit(findings)


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description=f"{RULE_ID} scanner")
    sub = p.add_subparsers(dest="cmd", required=True)

    t = sub.add_parser("tree")
    t.add_argument("--root", required=True)
    t.set_defaults(func=cmd_tree)

    gi = sub.add_parser("git-index")
    gi.add_argument("--repo", default=".")
    gi.set_defaults(func=cmd_git_index)

    gt = sub.add_parser("git-tree")
    gt.add_argument("--repo", default=".")
    gt.add_argument("--allow-blind-scan", action="store_true")
    gt.set_defaults(func=cmd_git_tree)

    gd = sub.add_parser("git-diff")
    gd.add_argument("--repo", default=".")
    gd.add_argument("--base", default="origin/main")
    gd.set_defaults(func=cmd_git_diff)

    gp = sub.add_parser("git-publication")
    gp.add_argument("--repo", default=".")
    gp.set_defaults(func=cmd_git_publication)

    gh = sub.add_parser("history")
    gh.add_argument("--repo", default=".")
    gh.set_defaults(func=cmd_history)

    a = sub.add_parser("archive")
    a.add_argument("--archive", required=True)
    a.set_defaults(func=cmd_archive)

    o = sub.add_parser("oci")
    o.add_argument("--oci", required=True)
    o.set_defaults(func=cmd_oci)

    as_ = sub.add_parser("assets")
    as_.add_argument("--assets", required=True)
    as_.set_defaults(func=cmd_assets)

    args = p.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    sys.exit(main())
