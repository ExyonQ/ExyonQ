#!/usr/bin/env python3
# Copyright 2026 Antonio Cantallops Alba — Apache-2.0
"""Conservative Rust lexical scan: strip comments/strings; keep line numbers."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class CodeSpan:
    """A code fragment with original 1-based line number (start of fragment)."""

    line: int
    text: str


def strip_comments_and_strings(source: str) -> list[CodeSpan]:
    """
    Return code-only spans with approximate original line numbers.

    Handles: //, /* */, ", ", r#...#, b"/br#", char literals.
    Not a full lexer — when ambiguous, prefer leaving tokens (false WARN later)
    over inventing structure inside strings.
    """
    out: list[CodeSpan] = []
    i = 0
    n = len(source)
    line = 1
    buf: list[str] = []
    buf_line = 1

    def flush() -> None:
        nonlocal buf, buf_line
        if buf:
            text = "".join(buf)
            if text.strip():
                out.append(CodeSpan(buf_line, text))
            buf = []

    def start_buf() -> None:
        nonlocal buf_line
        if not buf:
            buf_line = line

    while i < n:
        c = source[i]
        nxt = source[i + 1] if i + 1 < n else ""

        # line comment
        if c == "/" and nxt == "/":
            flush()
            i += 2
            while i < n and source[i] != "\n":
                i += 1
            continue

        # block comment
        if c == "/" and nxt == "*":
            flush()
            i += 2
            while i < n - 1:
                if source[i] == "\n":
                    line += 1
                if source[i] == "*" and source[i + 1] == "/":
                    i += 2
                    break
                i += 1
            else:
                i = n
            continue

        # raw string r#"..."# / r##"..."## / br#"..."#
        if c in "br" or (c == "r" and nxt in '#"'):
            # try byte/raw prefixes
            j = i
            if source[j] == "b":
                j += 1
            if j < n and source[j] == "r":
                j += 1
                hashes = 0
                while j < n and source[j] == "#":
                    hashes += 1
                    j += 1
                if j < n and source[j] == '"':
                    flush()
                    j += 1
                    closing = '"' + ("#" * hashes)
                    while j < n:
                        if source[j] == "\n":
                            line += 1
                        if source.startswith(closing, j):
                            j += len(closing)
                            break
                        j += 1
                    i = j
                    continue

        # normal / byte string
        if c == '"' or (c == "b" and nxt == '"'):
            flush()
            if c == "b":
                i += 1
            i += 1  # opening "
            while i < n:
                if source[i] == "\\":
                    i += 2
                    continue
                if source[i] == "\n":
                    line += 1
                    i += 1
                    continue
                if source[i] == '"':
                    i += 1
                    break
                i += 1
            continue

        # char literal 'x' / '\n' — skip carefully (not lifetimes)
        if c == "'":
            # lifetime or byte-char: 'a or 'a or '\n' or 'ab (invalid)
            # Heuristic: if next is \\ or single char then ', treat as char.
            if i + 2 < n and source[i + 1] == "\\" and i + 3 < n:
                # '\x' form — skip until closing '
                flush()
                i += 1
                while i < n and source[i] != "'":
                    if source[i] == "\n":
                        line += 1
                    i += 1
                if i < n:
                    i += 1
                continue
            if i + 2 < n and source[i + 2] == "'" and source[i + 1] != "'":
                flush()
                i += 3
                continue
            # else lifetime / ambiguous — keep as code
            start_buf()
            buf.append(c)
            i += 1
            continue

        if c == "\n":
            flush()
            line += 1
            i += 1
            continue

        start_buf()
        buf.append(c)
        i += 1

    flush()
    return out


def iter_code_lines(source: str) -> list[tuple[int, str]]:
    """Merge spans back into per-line code text (comment/string free)."""
    by_line: dict[int, list[str]] = {}
    for span in strip_comments_and_strings(source):
        # span.text may contain no newlines (we flush on newline)
        by_line.setdefault(span.line, []).append(span.text)
    return sorted((ln, "".join(parts)) for ln, parts in by_line.items())
