#!/usr/bin/env python3
# Copyright 2026 Antonio Cantallops Alba — Apache-2.0
"""Shared helpers for DB2C2 provider-surface gates."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable

from common import (
    EXIT_CONFIG,
    Finding,
    SEVERITY_ERROR,
    SEVERITY_INFO,
    SEVERITY_WARN,
    load_toml,
    workspace_member_paths,
)
from rust_scan import iter_code_lines

# Cargo package name → Rust path root(s)
PROVIDER_PATH_ROOTS: dict[str, tuple[str, ...]] = {
    "anyhow": ("anyhow",),
    "arc-swap": ("arc_swap",),
    "async-trait": ("async_trait",),
    "base64": ("base64",),
    "bytes": ("bytes",),
    "chrono": ("chrono",),
    "clap": ("clap",),
    "criterion": ("criterion",),
    "dashmap": ("dashmap",),
    "flate2": ("flate2",),
    "h3": ("h3",),
    "h3-quinn": ("h3_quinn",),
    "http": ("http",),
    "http-body-util": ("http_body_util",),
    "hyper": ("hyper",),
    "hyper-util": ("hyper_util",),
    "instant-acme": ("instant_acme",),
    "libc": ("libc",),
    "libfuzzer-sys": ("libfuzzer_sys",),
    "matchit": ("matchit",),
    "memmap2": ("memmap2",),
    "notify": ("notify",),
    "quinn": ("quinn",),
    "quinn-proto": ("quinn_proto",),
    "rustls": ("rustls",),
    "rustls-pemfile": ("rustls_pemfile",),
    "semver": ("semver",),
    "serde": ("serde",),
    "serde_json": ("serde_json",),
    "sha1": ("sha1",),
    "socket2": ("socket2",),
    "tempfile": ("tempfile",),
    "thiserror": ("thiserror",),
    "tokio": ("tokio",),
    "tokio-rustls": ("tokio_rustls",),
    "tokio-util": ("tokio_util",),
    "toml": ("toml",),
    "tracing": ("tracing",),
    "tracing-subscriber": ("tracing_subscriber",),
    "wasmtime": ("wasmtime",),
    "wat": ("wat",),
}

CRITICAL_PROVIDERS = frozenset(
    {
        "tokio",
        "rustls",
        "tokio-rustls",
        "quinn",
        "h3",
        "h3-quinn",
        "wasmtime",
        "socket2",
        "libc",
        "hyper",
        "hyper-util",
        "http-body-util",
    }
)

# Selected http types accepted under DBEX-002 (path suffixes after http::)
HTTP_ACCEPTED_TYPES = frozenset(
    {
        "Method",
        "Uri",
        "StatusCode",
        "Version",
        "HeaderName",
        "HeaderValue",
        "HeaderMap",
        "Request",
        "Response",
        "header",
        "request",
        "response",
        "uri",
        "method",
        "status",
        "version",
    }
)

STABLE_FACADE_CRATES = frozenset(
    {
        "exyonq-module-api",
        "exyonq-runtime-plan",
        "exyonq-addon-api",
    }
)

# Crates treated as CLI/binary/orchestration for anyhow zoning
ANYHOW_ALLOWED_EXTRA = frozenset(
    {
        "exyonq",
        "exyonqctl",
        "xtask",
        "exyonq-bench",
        "exyonq-compat-cli",
        "exyonq-compat-nginx",
        "exyonq-mock-upstream",
    }
)

STATUS_VIOLATION = "VIOLATION"
STATUS_KNOWN_DEBT = "KNOWN_DEBT"
STATUS_ACCEPTED = "ACCEPTED"
STATUS_REVIEW = "REVIEW_REQUIRED"
STATUS_INFO = "INFO"

_IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
_ROOT_ALT = "|".join(
    sorted(
        (re.escape(r) for roots in PROVIDER_PATH_ROOTS.values() for r in roots),
        key=len,
        reverse=True,
    )
)
# NOTE: do not build `::` path tails inside f-strings — `{`/`}` interact with `(?:…)`.
# Provider path requires `root::…` (avoids local bindings named `rustls`, `http`, …).
# Nested `quinn::crypto::rustls` excluded by (?<!::).
_PATH_RE = re.compile(
    r"(?<!::)\b(" + _ROOT_ALT + r")::(" + _IDENT + r"(?:::" + _IDENT + r")*)?"
)
# `use rustls;` / `use rustls::{…}` / `use rustls as X;`
_USE_ROOT_RE = re.compile(
    r"\b(pub(?:\s*\(([^)]*)\))?\s+)?use\s+(" + _ROOT_ALT + r")\s*(?:;|::|\{|as\b)"
)
_PUB_USE_RE = re.compile(r"\bpub(?:\s*\(([^)]*)\))?\s+use\s+(.+?);")
_EXTERN_CRATE_RE = re.compile(r"\bpub(?:\s*\([^)]*\))?\s+extern\s+crate\s+(" + _IDENT + r")")
_CORE_TLS_REEXPORT_RE = re.compile(
    r"\bpub\s+use\s+exyonq_mod_tls\s+as\s+tls\s*;"
)
_PUB_ITEM_RE = re.compile(
    r"\bpub(?:\s*\(([^)]*)\))?\s+(fn|struct|enum|type|trait|const|static)\s+(" + _IDENT + r")"
)


@dataclass(frozen=True)
class ProviderHit:
    provider: str  # cargo package name
    rust_root: str
    path: str  # full path text e.g. tokio::net::TcpStream
    line: int
    kind: str  # use | path | pub_use | extern_crate
    visibility: str  # pub | pub(crate) | pub(super) | private | unknown
    symbol: str


@dataclass
class CrateFile:
    crate: str
    rel_path: str
    abs_path: Path
    kind: str  # prod | test | bench | example | fuzz | build


@dataclass(frozen=True)
class BaselineEntry:
    id: str
    provider: str
    kind: str
    file: str
    symbol_or_pattern: str
    status: str
    policy_reference: str
    expansion_allowed: bool = False


def surface_finding(
    *,
    gate_id: str,
    severity: str,
    status: str,
    provider: str,
    file: str,
    line: int,
    symbol: str,
    message: str,
    policy_source: str,
    suggested_action: str,
    baseline_id: str = "",
    exception_id: str = "",
) -> Finding:
    section = f"L{line}:{status}"
    if baseline_id:
        section = f"L{line}:{status}:{baseline_id}"
    return Finding(
        gate_id=gate_id,
        severity=severity,
        dependency=provider,
        file=file,
        section=section,
        message=message,
        policy_source=policy_source,
        suggested_action=suggested_action,
        status=status,
        line=line,
        symbol=symbol,
        baseline_id=baseline_id,
        exception_id=exception_id,
    )


def build_crate_index(root: Path) -> list[CrateFile]:
    """Map .rs files under workspace members to crate + kind."""
    out: list[CrateFile] = []
    for manifest in workspace_member_paths(root):
        data = load_toml(manifest)
        crate = data.get("package", {}).get("name", manifest.parent.name)
        crate_root = manifest.parent
        rel_crate = str(crate_root.relative_to(root))

        def add_tree(base: Path, kind: str) -> None:
            if not base.is_dir():
                return
            for p in sorted(base.rglob("*.rs")):
                if "target" in p.parts:
                    continue
                rel = str(p.relative_to(root))
                out.append(CrateFile(crate=crate, rel_path=rel, abs_path=p, kind=kind))

        # Standard layout
        add_tree(crate_root / "src", "prod")
        add_tree(crate_root / "tests", "test")
        add_tree(crate_root / "benches", "bench")
        add_tree(crate_root / "examples", "example")
        add_tree(crate_root / "fuzz_targets", "fuzz")
        build_rs = crate_root / "build.rs"
        if build_rs.is_file():
            out.append(
                CrateFile(
                    crate=crate,
                    rel_path=str(build_rs.relative_to(root)),
                    abs_path=build_rs,
                    kind="build",
                )
            )
        # fuzz workspace: sources under fuzz/
        if crate == "exyonq-fuzz" or rel_crate == "fuzz":
            add_tree(crate_root, "fuzz")

    # Dedup by path (prefer first)
    seen: set[str] = set()
    uniq: list[CrateFile] = []
    for cf in out:
        if cf.rel_path in seen:
            continue
        seen.add(cf.rel_path)
        uniq.append(cf)
    return uniq


def rust_root_to_provider() -> dict[str, str]:
    m: dict[str, str] = {}
    for pkg, roots in PROVIDER_PATH_ROOTS.items():
        for r in roots:
            m[r] = pkg
    return m


ROOT_TO_PROVIDER = rust_root_to_provider()


def parse_visibility(pub_group: str | None) -> str:
    if pub_group is None:
        return "private"
    g = pub_group.strip()
    if g == "":
        return "pub"
    if g.startswith("("):
        inner = g.strip("()").strip()
        if inner == "crate":
            return "pub(crate)"
        if inner == "super":
            return "pub(super)"
        return f"pub({inner})"
    return "pub"


def scan_provider_hits(source: str) -> list[ProviderHit]:
    """Detect provider path / use / pub use / extern crate in code (not comments/strings)."""
    hits: list[ProviderHit] = []
    for line_no, text in iter_code_lines(source):
        # Special: core::tls re-export (not a direct rustls path)
        if _CORE_TLS_REEXPORT_RE.search(text):
            hits.append(
                ProviderHit(
                    provider="rustls",
                    rust_root="exyonq_mod_tls",
                    path="exyonq_mod_tls as tls",
                    line=line_no,
                    kind="core_tls_reexport",
                    visibility="pub",
                    symbol="tls",
                )
            )

        # use root; / use root::… / pub use root::…
        for m in _USE_ROOT_RE.finditer(text):
            pub = m.group(1)
            vis_inner = m.group(2)
            root = m.group(3)
            provider = ROOT_TO_PROVIDER[root]
            if pub:
                vis = parse_visibility(f"({vis_inner})" if vis_inner is not None else "")
                kind = "pub_use"
            else:
                vis = "private"
                kind = "use"
            # Prefer full path on the same match line when present
            path = root
            pm = _PATH_RE.search(text, m.start())
            if pm and pm.group(1) == root:
                path = pm.group(0)
            hits.append(
                ProviderHit(
                    provider=provider,
                    rust_root=root,
                    path=path,
                    line=line_no,
                    kind=kind,
                    visibility=vis,
                    symbol=path.split("::")[-1],
                )
            )

        # extern crate
        for m in _EXTERN_CRATE_RE.finditer(text):
            name = m.group(1)
            provider = ROOT_TO_PROVIDER.get(name)
            if provider:
                hits.append(
                    ProviderHit(
                        provider=provider,
                        rust_root=name,
                        path=name,
                        line=line_no,
                        kind="extern_crate",
                        visibility="pub",
                        symbol=name,
                    )
                )

        # path references (root::…)
        for m in _PATH_RE.finditer(text):
            path = m.group(0)
            root = m.group(1)
            provider = ROOT_TO_PROVIDER[root]
            kind = "path"
            vis = "private"
            prefix = text[: m.start()]
            if re.search(r"\buse\s+$", prefix) or re.search(r"\buse\s+\{[^}]*$", prefix):
                kind = "use"
            if re.search(r"\bpub(?:\s*\([^)]*\))?\s+use\s+$", prefix):
                kind = "pub_use"
                pm = re.search(r"\bpub(?:\s*\(([^)]*)\))?\s+use\s+$", prefix)
                if pm:
                    vis = parse_visibility(f"({pm.group(1)})" if pm.group(1) is not None else "")
                else:
                    vis = "pub"
            sym = path.split("::")[-1]
            if any(h.line == line_no and h.path == path for h in hits):
                continue
            hits.append(
                ProviderHit(
                    provider=provider,
                    rust_root=root,
                    path=path,
                    line=line_no,
                    kind=kind,
                    visibility=vis,
                    symbol=sym,
                )
            )
    return hits


def load_baseline(root: Path) -> list[BaselineEntry]:
    path = root / "docs/architecture/dependency-boundaries/dependency-provider-surface-baseline.toml"
    if not path.is_file():
        return []
    data = load_toml(path)
    rows = data.get("baseline", [])
    if not isinstance(rows, list):
        raise SystemExit(f"{EXIT_CONFIG}: {path}: [[baseline]] missing")
    out: list[BaselineEntry] = []
    seen: set[str] = set()
    for row in rows:
        bid = row.get("id")
        if not bid or bid in seen:
            raise SystemExit(f"{EXIT_CONFIG}: duplicate/missing baseline id: {bid!r}")
        seen.add(bid)
        if row.get("expansion_allowed", False):
            raise SystemExit(f"{EXIT_CONFIG}: baseline {bid}: expansion_allowed must be false")
        out.append(
            BaselineEntry(
                id=str(bid),
                provider=str(row["provider"]),
                kind=str(row["kind"]),
                file=str(row["file"]),
                symbol_or_pattern=str(row["symbol_or_pattern"]),
                status=str(row["status"]),
                policy_reference=str(row["policy_reference"]),
                expansion_allowed=False,
            )
        )
    return out


def match_baseline(
    entries: Iterable[BaselineEntry],
    *,
    provider: str,
    file: str,
    kind: str | None = None,
    symbol: str = "",
    path: str = "",
) -> BaselineEntry | None:
    """Exact file match; symbol_or_pattern must be substring of path/symbol/line pattern."""
    for e in entries:
        if e.provider != provider:
            continue
        if e.file != file:
            continue
        if kind and e.kind != kind and e.kind != "any":
            # allow kind mismatch only when baseline kind is broad marker
            if e.kind not in {kind, "import", "pub_use", "pub_type", "error", "owner_rule", "reexport"}:
                continue
        pat = e.symbol_or_pattern
        hay = path or symbol
        if pat == "*" or pat in hay or pat in symbol or (symbol and symbol in pat):
            return e
        # also allow exact pattern equality to kind tags
        if pat == path or pat == symbol:
            return e
    return None


def find_baseline(
    entries: list[BaselineEntry],
    *,
    provider: str,
    file: str,
    symbol_or_pattern: str,
    kinds: tuple[str, ...] | None = None,
) -> BaselineEntry | None:
    for e in entries:
        if e.provider != provider or e.file != file:
            continue
        if kinds and e.kind not in kinds:
            continue
        if e.symbol_or_pattern == symbol_or_pattern or e.symbol_or_pattern in symbol_or_pattern:
            return e
        if symbol_or_pattern in e.symbol_or_pattern:
            return e
    return None


def is_cfg_test_region(lines: list[tuple[int, str]], idx: int) -> bool:
    """Heuristic: inside mod tests { } under #[cfg(test)] above."""
    # walk backwards for #[cfg(test)] mod tests
    for j in range(idx, max(-1, idx - 40), -1):
        if j < 0:
            break
        t = lines[j][1]
        if "#[cfg(test)]" in t.replace(" ", ""):
            return True
        if re.search(r"\bmod\s+tests\b", t) and j > 0 and "cfg(test)" in lines[j - 1][1]:
            return True
    return False


def http_path_accepted(path: str) -> bool:
    parts = path.split("::")
    if not parts or parts[0] != "http":
        return False
    if len(parts) == 1:
        return True
    return parts[1] in HTTP_ACCEPTED_TYPES


def bytes_path_accepted(path: str) -> bool:
    return path == "bytes" or path.startswith("bytes::Bytes") or path.startswith("bytes::BytesMut")


def collect_pub_item_regions(source: str) -> list[tuple[int, str, str, str]]:
    """
    Conservative pub item regions: (line, visibility, item_kind, signature_text).

    Signature text spans from the pub item line until `{` or `;` (not a full AST).
    """
    lines = iter_code_lines(source)
    # map line->text for sequential scan using original order
    ordered = lines
    out: list[tuple[int, str, str, str]] = []
    i = 0
    while i < len(ordered):
        ln, text = ordered[i]
        m = _PUB_ITEM_RE.search(text)
        if not m:
            i += 1
            continue
        vis = parse_visibility(f"({m.group(1)})" if m.group(1) is not None else "")
        kind = m.group(2)
        name = m.group(3)
        chunk = [text]
        j = i
        # accumulate until ; or { at brace depth 0 for generics
        depth_angle = text.count("<") - text.count(">")
        while j + 1 < len(ordered):
            if ";" in chunk[-1] or "{" in chunk[-1]:
                break
            if depth_angle <= 0 and (";" in text or "{" in text):
                break
            j += 1
            _ln2, t2 = ordered[j]
            chunk.append(t2)
            depth_angle += t2.count("<") - t2.count(">")
            if ";" in t2 or "{" in t2:
                break
        sig = " ".join(c.strip() for c in chunk)
        out.append((ln, vis, kind, f"{name} :: {sig}"))
        i = j + 1
    return out


def effective_file_kind(cf: CrateFile, source: str, line: int) -> str:
    """Refine prod → test when inside #[cfg(test)] near the hit."""
    if cf.kind != "prod":
        return cf.kind
    lines = iter_code_lines(source)
    # find index of line
    idx = next((i for i, (ln, _) in enumerate(lines) if ln == line), None)
    if idx is None:
        return cf.kind
    if is_cfg_test_region(lines, idx):
        return "test"
    return cf.kind
