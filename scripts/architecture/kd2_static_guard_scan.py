#!/usr/bin/env python3
"""Classify exyonq_mod_static references in core as production or test-only."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

TARGET = "exyonq_mod_static"
DEDICATED_TEST_FILES = frozenset({"core/src/kernel/test_hooks.rs"})


@dataclass(frozen=True)
class Token:
    text: str
    line: int


def _mask_non_code(source: str) -> str:
    """Replace comments and literals with spaces while preserving newlines."""
    out = list(source)
    i = 0
    block_depth = 0
    while i < len(source):
        if block_depth:
            if source.startswith("/*", i):
                out[i : i + 2] = "  "
                block_depth += 1
                i += 2
            elif source.startswith("*/", i):
                out[i : i + 2] = "  "
                block_depth -= 1
                i += 2
            else:
                if source[i] != "\n":
                    out[i] = " "
                i += 1
            continue
        if source.startswith("//", i):
            end = source.find("\n", i)
            end = len(source) if end < 0 else end
            out[i:end] = " " * (end - i)
            i = end
            continue
        if source.startswith("/*", i):
            out[i : i + 2] = "  "
            block_depth = 1
            i += 2
            continue
        if source[i] in {'"', "'"}:
            quote = source[i]
            # A single quote followed by an identifier is a Rust lifetime.
            if quote == "'" and i + 1 < len(source) and (
                source[i + 1].isalpha() or source[i + 1] == "_"
            ):
                i += 1
                continue
            out[i] = " "
            i += 1
            while i < len(source):
                if source[i] == "\\":
                    out[i] = " "
                    if i + 1 < len(source):
                        if source[i + 1] != "\n":
                            out[i + 1] = " "
                        i += 2
                    continue
                if source[i] == quote:
                    out[i] = " "
                    i += 1
                    break
                if source[i] != "\n":
                    out[i] = " "
                i += 1
            continue
        i += 1
    return "".join(out)


def _tokens(source: str) -> list[Token]:
    masked = _mask_non_code(source)
    tokens: list[Token] = []
    line = 1
    for match in re.finditer(r"[A-Za-z_][A-Za-z0-9_]*|[#\[\]{}();]", masked):
        line += masked.count("\n", 0 if not tokens else _token_end, match.start())
        tokens.append(Token(match.group(0), line))
        _token_end = match.end()
    return tokens


def _test_item_openings(tokens: list[Token]) -> set[int]:
    """Return token indexes of `{` opening items gated by cfg containing test."""
    openings: set[int] = set()
    pending_test_attr = False
    i = 0
    qualifiers = {"pub", "crate", "super", "self"}
    while i < len(tokens):
        if tokens[i].text == "#" and i + 1 < len(tokens) and tokens[i + 1].text == "[":
            depth = 1
            j = i + 2
            attr: list[str] = []
            while j < len(tokens) and depth:
                if tokens[j].text == "[":
                    depth += 1
                elif tokens[j].text == "]":
                    depth -= 1
                    if depth == 0:
                        break
                attr.append(tokens[j].text)
                j += 1
            if "cfg" in attr and "test" in attr:
                pending_test_attr = True
            i = j + 1
            continue
        if tokens[i].text in qualifiers or tokens[i].text in {"(", ")"}:
            i += 1
            continue
        if pending_test_attr:
            j = i
            while j < len(tokens) and tokens[j].text not in {"{", ";"}:
                j += 1
            if j < len(tokens) and tokens[j].text == "{":
                openings.add(j)
            pending_test_attr = False
            i = j
            continue
        if tokens[i].text not in {"#", "[", "]"}:
            pending_test_attr = False
        i += 1
    return openings


def production_reference_lines(source: str) -> list[int]:
    tokens = _tokens(source)
    test_openings = _test_item_openings(tokens)
    test_stack: list[bool] = []
    violations: list[int] = []
    for index, token in enumerate(tokens):
        if token.text == "{":
            test_stack.append((test_stack[-1] if test_stack else False) or index in test_openings)
            continue
        if token.text == "}":
            if test_stack:
                test_stack.pop()
            continue
        if token.text == TARGET and not (test_stack and test_stack[-1]):
            violations.append(token.line)
    return sorted(set(violations))


def scan_file(root: Path, path: Path) -> list[str]:
    rel = path.relative_to(root).as_posix()
    if rel in DEDICATED_TEST_FILES:
        return []
    source = path.read_text(encoding="utf-8", errors="replace")
    original = source.splitlines()
    return [f"{rel}:{line}:{original[line - 1]}" for line in production_reference_lines(source)]


def self_test() -> None:
    fixture = """
#[cfg(all(
    test,
    target_os = "linux"
))]
mod contract_tests {
    fn nested() { exyonq_mod_static::install(); }
}
fn helper_for_tests() { exyonq_mod_static::production_call(); }
"""
    assert production_reference_lines(fixture) == [9], production_reference_lines(fixture)
    commented = '// exyonq_mod_static::ignored();\nconst S: &str = "exyonq_mod_static";\n'
    assert production_reference_lines(commented) == []


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("paths", nargs="*")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    root = args.root.resolve()
    violations: list[str] = []
    for raw in args.paths:
        violations.extend(scan_file(root, (root / raw).resolve()))
    if violations:
        print("\n".join(violations))
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
