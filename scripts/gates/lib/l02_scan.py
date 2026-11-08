#!/usr/bin/env python3
"""L-02 benchmark-path decision scanner.

Scans every decision Rust file completely. Only test regions are masked:
`#[cfg(test)]` / `#[cfg(all(test, ...))]` items, files under `tests/`,
`*_tests.rs` / `tests.rs`, and files carrying `#![cfg(test)]`. A production
item placed after a test module is therefore still inspected.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

DECISION_ROOTS = (
    "core/src",
    "module-api/src",
    "crates/exyonq-mod-proxy/src",
    "crates/exyonq-mod-static/src",
    "crates/exyonq-proxy-sdp/src",
    "crates/exyonq-platform-linux/src",
    "crates/exyonq-platform-macos/src",
    "crates/exyonq-runtime-plan/src",
)
SKIP_FILES = {
    "core/src/kernel/test_hooks.rs",  # test-only hook module
}
LITERAL = re.compile(r'b?"(?:/api|/site)[^"]*"')
SITE_DECISION = re.compile(r'==\s*(?:Some\()?"site"|route\.name\s*==\s*"site"')
REMOVED_HELPERS = re.compile(
    r"SSE_STREAM_PATHS|PRESEED_PROXY_PATHS|path_is_sse_stream|"
    r"upstream_timeout_for_path|preseed_path_uris"
)


def _compact_cfg(body: str) -> str:
    return re.sub(r"\s+", "", body)


def _is_test_cfg(body: str) -> bool:
    compact = _compact_cfg(body)
    return compact == "test" or compact.startswith("all(test")


def _line_end(text: str, i: int) -> int:
    end = text.find("\n", i)
    return len(text) if end < 0 else end


def _block_comment_end(text: str, i: int) -> int:
    depth = 0
    while i < len(text):
        if text.startswith("/*", i):
            depth += 1
            i += 2
        elif text.startswith("*/", i):
            depth -= 1
            i += 2
            if depth == 0:
                return i
        else:
            i += 1
    return len(text)


def _string_end(text: str, i: int) -> int:
    i += 1
    while i < len(text):
        if text[i] == "\\":
            i += 2
        elif text[i] == '"':
            return i + 1
        else:
            i += 1
    return len(text)


def _char_end(text: str, i: int) -> int:
    # A lifetime (`'a`) is not a char literal; only consume real chars.
    j = i + 1
    if j >= len(text):
        return j
    if text[j] == "\\":
        j += 1
        while j < len(text):
            if text[j] == "'":
                return j + 1
            j += 1
        return j
    if j + 1 < len(text) and text[j + 1] == "'":
        return j + 2
    return i + 1


def _raw_string_end(text: str, i: int) -> int | None:
    match = re.match(r'(?:b|c)?r(#*)"', text[i:])
    if not match:
        return None
    if i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_"):
        return None
    marker = '"' + match.group(1)
    end = text.find(marker, i + match.end())
    return len(text) if end < 0 else end + len(marker)


def _brace_end(text: str, i: int) -> int:
    depth = 0
    while i < len(text):
        if text.startswith("//", i):
            i = _line_end(text, i)
        elif text.startswith("/*", i):
            i = _block_comment_end(text, i)
        elif text[i] == '"':
            i = _string_end(text, i)
        elif text[i] == "'":
            i = _char_end(text, i)
        else:
            raw_end = _raw_string_end(text, i)
            if raw_end is not None:
                i = raw_end
            elif text[i] == "{":
                depth += 1
                i += 1
            elif text[i] == "}":
                depth -= 1
                i += 1
                if depth == 0:
                    return i
            else:
                i += 1
    return len(text)


def _item_end(text: str, i: int) -> int:
    """End of the item beginning at i, ignoring braces in literals/comments."""
    while i < len(text):
        if text.startswith("//", i):
            i = _line_end(text, i)
        elif text.startswith("/*", i):
            i = _block_comment_end(text, i)
        elif text[i] == '"':
            i = _string_end(text, i)
        elif text[i] == "'":
            i = _char_end(text, i)
        else:
            raw_end = _raw_string_end(text, i)
            if raw_end is not None:
                i = raw_end
            elif text[i] == "{":
                return _brace_end(text, i)
            elif text[i] == ";":
                return i + 1
            elif text[i] == "}":
                return i
            else:
                i += 1
    return len(text)


def _attribute_end(text: str, i: int) -> int:
    """End of the attribute starting at `#[` or `#![`."""
    j = i + (3 if text.startswith("#![", i) else 2)
    depth = 1
    while j < len(text):
        if text.startswith("//", j):
            j = _line_end(text, j)
        elif text.startswith("/*", j):
            j = _block_comment_end(text, j)
        elif text[j] == '"':
            j = _string_end(text, j)
        elif text[j] == "'":
            j = _char_end(text, j)
        else:
            raw_end = _raw_string_end(text, j)
            if raw_end is not None:
                j = raw_end
            elif text[j] == "[":
                depth += 1
                j += 1
            elif text[j] == "]":
                depth -= 1
                j += 1
                if depth == 0:
                    return j
            else:
                j += 1
    return len(text)


def _iter_attributes(text: str):
    """Yield (start, end, cfg_body, is_inner) outside strings/comments."""
    i = 0
    while i < len(text):
        if text.startswith("//", i):
            i = _line_end(text, i)
        elif text.startswith("/*", i):
            i = _block_comment_end(text, i)
        elif text[i] == '"':
            i = _string_end(text, i)
        elif text[i] == "'":
            i = _char_end(text, i)
        else:
            raw_end = _raw_string_end(text, i)
            if raw_end is not None:
                i = raw_end
            elif text.startswith("#![", i) or text.startswith("#[", i):
                end = _attribute_end(text, i)
                inner = text.startswith("#![", i)
                prefix = 3 if inner else 2
                attribute = text[i + prefix : end - 1]
                cfg = re.match(r"\s*cfg\s*\((.*)\)\s*$", attribute, re.S)
                yield i, end, (cfg.group(1) if cfg else None), inner
                i = end
            else:
                i += 1


def _is_entire_test_file(text: str) -> bool:
    return any(
        inner and body is not None and _is_test_cfg(body)
        for _, _, body, inner in _iter_attributes(text)
    )


def _mask_ranges(text: str, ranges: list[tuple[int, int]]) -> str:
    chars = list(text)
    for start, end in ranges:
        for pos in range(max(0, start), min(len(chars), end)):
            if chars[pos] != "\n":
                chars[pos] = " "
    return "".join(chars)


def _test_ranges(text: str) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    for start, end, body, inner in _iter_attributes(text):
        if inner or body is None or not _is_test_cfg(body):
            continue
        ranges.append((start, _item_end(text, end)))
    return ranges


def _comment_ranges(text: str) -> list[tuple[int, int]]:
    ranges: list[tuple[int, int]] = []
    i = 0
    while i < len(text):
        if text.startswith("//", i):
            end = _line_end(text, i)
            ranges.append((i, end))
            i = end
        elif text.startswith("/*", i):
            end = _block_comment_end(text, i)
            ranges.append((i, end))
            i = end
        elif text[i] == '"':
            i = _string_end(text, i)
        elif text[i] == "'":
            i = _char_end(text, i)
        else:
            raw_end = _raw_string_end(text, i)
            i = raw_end if raw_end is not None else i + 1
    return ranges


def _mask_text(text: str) -> str:
    ranges = _test_ranges(text) + _comment_ranges(text)
    ranges.sort()
    merged: list[tuple[int, int]] = []
    for start, end in ranges:
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return _mask_ranges(text, merged)


def scan_root(root: Path) -> tuple[list[str], int]:
    failures: list[str] = []
    scanned = 0
    for decision_root in DECISION_ROOTS:
        base = root / decision_root
        if not base.is_dir():
            continue
        for dirpath, dirnames, filenames in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in {"target", "tests"}]
            for filename in filenames:
                if not filename.endswith(".rs"):
                    continue
                path = Path(dirpath) / filename
                rel = path.relative_to(root).as_posix()
                if (
                    rel in SKIP_FILES
                    or filename.endswith("_tests.rs")
                    or filename == "tests.rs"
                ):
                    continue
                text = path.read_text(encoding="utf-8")
                if _is_entire_test_file(text):
                    continue
                scanned += 1
                for lineno, line in enumerate(_mask_text(text).splitlines(), 1):
                    if LITERAL.search(line):
                        failures.append(
                            f"{rel}:{lineno}: benchmark path literal: {line.strip()}"
                        )
                    if SITE_DECISION.search(line):
                        failures.append(
                            f"{rel}:{lineno}: route-name decision on site: {line.strip()}"
                        )
                    if REMOVED_HELPERS.search(line):
                        failures.append(
                            f"{rel}:{lineno}: removed benchmark helper reference: {line.strip()}"
                        )
    return failures, scanned


def render_scan(root: Path) -> tuple[int, list[str], int]:
    failures, scanned = scan_root(root)
    if failures:
        print("L02_NO_BENCHMARK_PATH_DECISIONS=FAIL")
        for failure in failures:
            print(failure)
        return 1, failures, scanned
    print(f"L02_NO_BENCHMARK_PATH_DECISIONS=PASS files={scanned}")
    return 0, failures, scanned


def cmd_scan(args: argparse.Namespace) -> int:
    root = Path(args.root).resolve()
    if not root.is_dir():
        print(f"ERROR: not a directory: {root}", file=sys.stderr)
        return 2
    return render_scan(root)[0]


def cmd_selftest(args: argparse.Namespace) -> int:
    fixtures = Path(args.fixtures).resolve()
    cases = (("must-fail", 1), ("must-pass", 0))
    errors: list[str] = []
    for name, expected in cases:
        root = fixtures / name
        print(f"L02_SELFTEST_CASE={name}")
        rc, failures, _ = render_scan(root)
        print(f"L02_SELFTEST_CASE={name} expected={expected} actual={rc}")
        if rc != expected:
            errors.append(f"{name}: expected rc={expected}, got rc={rc}")
        if name == "must-fail" and not any(
            "benchmark path literal" in failure and "fixture.rs" in failure
            for failure in failures
        ):
            errors.append("must-fail did not report the production literal after the test module")
        if name == "must-pass" and failures:
            errors.append("must-pass reported failures")
    if errors:
        print("L02_SELFTEST=FAIL")
        for error in errors:
            print(error)
        return 1
    print("L02_SELFTEST=PASS")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    scan = sub.add_parser("scan")
    scan.add_argument("--root", required=True)
    scan.set_defaults(func=cmd_scan)
    selftest = sub.add_parser("selftest")
    selftest.add_argument("--fixtures", required=True)
    selftest.set_defaults(func=cmd_selftest)
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
