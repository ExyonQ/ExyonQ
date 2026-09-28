#!/usr/bin/env python3
"""Classify forbidden-token hits as executable behavior vs non-executable prose.

Used by ZERO_FAKE / test_integrity so policy docstrings and deny-lists are not
treated as mock/smoke implementations, while runtime selectors and identifiers
remain findings.

FAIL_CLOSED: unknown/ambiguous executable-looking strings stay detectable.
"""
from __future__ import annotations

import ast
import io
import json
import re
import tokenize
from dataclasses import dataclass
from functools import lru_cache
from typing import FrozenSet, Iterable, List, Optional, Sequence, Set, Tuple

# Conceptual roles (owner classification model).
EXECUTABLE_BEHAVIOR = "EXECUTABLE_BEHAVIOR"
EXECUTABLE_IDENTIFIER_OR_CALL = "EXECUTABLE_IDENTIFIER_OR_CALL"
EXECUTABLE_CONFIGURATION = "EXECUTABLE_CONFIGURATION"
NON_EXECUTABLE_COMMENT = "NON_EXECUTABLE_COMMENT"
NON_EXECUTABLE_DOCSTRING = "NON_EXECUTABLE_DOCSTRING"
POLICY_PROSE = "POLICY_PROSE"
DENY_LIST_PROSE = "DENY_LIST_PROSE"
NEGATIVE_DETECTOR_FIXTURE = "NEGATIVE_DETECTOR_FIXTURE"
HISTORICAL_EVIDENCE = "HISTORICAL_EVIDENCE"
UNKNOWN_AMBIGUOUS = "UNKNOWN_AMBIGUOUS"

NON_EXECUTABLE_ROLES = frozenset(
    {
        NON_EXECUTABLE_COMMENT,
        NON_EXECUTABLE_DOCSTRING,
        POLICY_PROSE,
        DENY_LIST_PROSE,
        NEGATIVE_DETECTOR_FIXTURE,
        HISTORICAL_EVIDENCE,
    }
)

DENY_LIST_NAMES = frozenset(
    {
        "EXPLICIT_NON_SCOPE",
        "NON_SCOPE",
        "NOT_IN_SCOPE",
        "DENY_LIST",
        "FORBIDDEN_MODES",
        "UNSUPPORTED_AS_ACCEPTANCE",
    }
)

# Whole-string runtime selectors (must remain detectable).
SELECTOR_VALUES = frozenset(
    {
        "mock",
        "mocked",
        "mockito",
        "wiremock",
        "fake",
        "faked",
        "stub",
        "stubbed",
        "dummy",
        "smoke",
        "smoke-test",
        "smoketest",
        "simulation",
        "simulate",
        "simulated",
    }
)

TOKEN_WORD_RE = re.compile(
    r"(?i)\b(?:mock(?:ed|ing|ito)?|wiremock|fake[ds]?|faking|stub(?:bed|bing)?|"
    r"dummy|smoke(?:[-_ ]?test)?|simulation|simulate[ds]?|emulated?|emulator|"
    r"synthetic)\b"
)

# camelCase / snake_case mock/fake segments (create_mock_backend, MockClock).
# Not stub/dummy/smoke — those match too many real identifiers (stub_port).
MOCK_FAKE_IDENT_RE = re.compile(
    r"(?i)(?:^|[^A-Za-z0-9])(?:mock(?:ed|ing|ito)?|wiremock|fake[ds]?)"
    r"(?:[A-Z]|_|$|[^A-Za-z0-9])"
)

IDENT_SEGMENT_RE = MOCK_FAKE_IDENT_RE

POLICY_IDENT_RE = re.compile(
    r"(?i)\b(?:USES_(?:SMOKE|MOCKS?|STUBS?|FAKES?|SYNTHETIC)|ZERO_FAKE|"
    r"FORBIDDEN_(?:SMOKE|MOCK|FAKE|STUB|DUMMY|SIMULATION)|"
    r"NO_SMOKE|ANTI_FAKE|SMOKE_AS_ACCEPTANCE_PROOF|CONTAINED_DETECTOR|"
    r"MOCKS_OR_STUBS_USED|NO_STUB_[A-Z0-9_]+)\b"
)

POLICY_PROSE_RE = re.compile(
    r"(?i)("
    r"\bno\s+(?:mock|fake|stub|dummy|smoke|simulation|synthetic)"
    r"|not\s+(?:an?\s+)?(?:in-memory\s+)?(?:mock|fake|stub|dummy)"
    r"|(?:mock|fake|stub|smoke|simulation|synthetic)s?\s+"
    r"(?:are\s+)?(?:forbidden|banned|prohibited|not\s+allowed)"
    r"|must\s+not\s+(?:use\s+)?(?:mock|fake|stub|smoke)"
    r"|do\s+not\s+use\s+(?:mock|fake|stub|smoke)"
    r"|(?:STUB|SMOKE|MOCK|FAKE|SIMULATION)\s*=\s*FORBIDDEN"
    r"|as\s+acceptance"
    r"|not\s+part\s+of\s+(?:supported|authoritative)"
    r"|EXPLICIT_NON_SCOPE"
    r"|ZERO_FAKE"
    r"|AUDITOR_NEGATIVE_FIXTURE"
    r")"
)

COMMENT_LINE_RE = re.compile(
    r"^\s*(#(?!!)|//|///|//!|/\*|\*|<!--)"
)


@dataclass(frozen=True)
class TokenHit:
    line: int
    role: str
    snippet: str


def is_executable_role(role: str) -> bool:
    return role not in NON_EXECUTABLE_ROLES


def _line_mentions_token(line: str) -> bool:
    """Word-boundary or mock/fake identifier segment after stripping policy flags."""
    stripped = POLICY_IDENT_RE.sub(" ", line)
    return bool(TOKEN_WORD_RE.search(stripped) or MOCK_FAKE_IDENT_RE.search(stripped))


def line_has_executable_token(
    text: str, rel: str, line_no: int, line: str
) -> bool:
    """True if a forbidden token on this line is executable (or ambiguous)."""
    for hit in classify_line(text, rel, line_no, line):
        if is_executable_role(hit.role):
            return True
    return False


def file_has_executable_token(text: str, rel: str) -> bool:
    """True if any forbidden token in the file is executable (or ambiguous)."""
    indexed_lines: Set[int] = set()
    for hit in _index_cached(rel, text):
        indexed_lines.add(hit.line)
        if is_executable_role(hit.role):
            return True
    for i, line in enumerate(text.splitlines(), 1):
        if i in indexed_lines:
            continue
        # Whole-word only. camelCase / snake mock segments are language-indexed
        # (Python AST, C-like). Do not apply them to shell via this fallback
        # (FAKE_DIG, no_fake_oci).
        stripped = POLICY_IDENT_RE.sub(" ", line)
        if TOKEN_WORD_RE.search(stripped) and not COMMENT_LINE_RE.match(line):
            return True
    return False


def classify_line(text: str, rel: str, line_no: int, line: str) -> List[TokenHit]:
    index = _index_cached(rel, text)
    hits = [h for h in index if h.line == line_no]
    if hits:
        return list(hits)
    # Fail closed: token present but not mapped → unknown/ambiguous executable.
    stripped = POLICY_IDENT_RE.sub(" ", line)
    if TOKEN_WORD_RE.search(stripped):
        if COMMENT_LINE_RE.match(line):
            return [TokenHit(line_no, NON_EXECUTABLE_COMMENT, line.strip()[:160])]
        return [TokenHit(line_no, UNKNOWN_AMBIGUOUS, line.strip()[:160])]
    return []


@lru_cache(maxsize=16)
def _index_cached(rel: str, text: str) -> Tuple[TokenHit, ...]:
    return tuple(_index_for(rel, text))


def _index_for(rel: str, text: str) -> List[TokenHit]:
    suffix = rel.rsplit(".", 1)[-1].lower() if "." in rel else ""
    name = rel.rsplit("/", 1)[-1].lower()
    if suffix == "py" or name.endswith(".py"):
        return _index_python(text)
    if suffix in {"rs", "c", "h", "go", "js", "ts"}:
        return _index_c_like(text)
    if suffix in {"sh"} or name.endswith(".sh"):
        return _index_shell(text)
    if suffix in {"yml", "yaml"}:
        return _index_simple_mapping(text)
    if suffix == "json":
        return _index_json(text)
    if suffix in {"toml", "md", "txt", "html"}:
        return _index_c_like(text)
    return _index_c_like(text)


def _index_python(text: str) -> List[TokenHit]:
    hits: List[TokenHit] = []
    comment_lines: Set[int] = set()
    try:
        for tok in tokenize.generate_tokens(io.StringIO(text).readline):
            if tok.type == tokenize.COMMENT:
                start, end = tok.start[0], tok.end[0]
                for ln in range(start, end + 1):
                    comment_lines.add(ln)
                if TOKEN_WORD_RE.search(tok.string):
                    hits.append(
                        TokenHit(start, NON_EXECUTABLE_COMMENT, tok.string.strip()[:160])
                    )
    except (tokenize.TokenError, IndentationError, SyntaxError):
        comment_lines = set()

    docstring_spans: List[Tuple[int, int, int, int]] = []
    deny_spans: List[Tuple[int, int, int, int]] = []
    try:
        tree = ast.parse(text)
    except SyntaxError:
        # Fail closed for unparseable Python: comments already recorded; remaining
        # token lines stay UNKNOWN_AMBIGUOUS via classify_line fallback.
        return hits

    docstring_spans = _python_docstring_spans(tree)
    deny_spans = _python_deny_list_spans(tree)

    def in_spans(node: ast.AST, spans: Sequence[Tuple[int, int, int, int]]) -> bool:
        lineno = getattr(node, "lineno", None)
        col = getattr(node, "col_offset", 0)
        end_lineno = getattr(node, "end_lineno", lineno)
        end_col = getattr(node, "end_col_offset", col)
        if lineno is None:
            return False
        for sl, sc, el, ec in spans:
            if (lineno, col) >= (sl, sc) and (end_lineno or lineno, end_col or 0) <= (el, ec):
                return True
        return False

    # Docstrings (whole span).
    for sl, sc, el, ec in docstring_spans:
        snippet = ""
        for ln in range(sl, el + 1):
            hits.append(TokenHit(ln, NON_EXECUTABLE_DOCSTRING, snippet))

    for node in ast.walk(tree):
        if not _is_str_node(node):
            continue
        value = _str_value(node)
        if not value or not TOKEN_WORD_RE.search(value):
            continue
        lineno = int(getattr(node, "lineno", 1) or 1)
        role = EXECUTABLE_CONFIGURATION
        if in_spans(node, docstring_spans):
            # Fail closed: a docstring whose entire value is a runtime selector
            # ("""mock""") is configuration-shaped, not policy prose.
            role = (
                EXECUTABLE_CONFIGURATION
                if _is_selector_string(value)
                else NON_EXECUTABLE_DOCSTRING
            )
        elif in_spans(node, deny_spans):
            role = DENY_LIST_PROSE
        elif POLICY_PROSE_RE.search(value):
            role = POLICY_PROSE
        elif _is_selector_string(value):
            role = EXECUTABLE_CONFIGURATION
        else:
            # Phrase containing a forbidden word without policy negation:
            # keep detectable (fail closed).
            role = UNKNOWN_AMBIGUOUS
        hits.append(TokenHit(lineno, role, value[:160]))

    for node in ast.walk(tree):
        if isinstance(node, ast.Name) and _ident_is_forbidden(node.id):
            hits.append(
                TokenHit(
                    int(getattr(node, "lineno", 1) or 1),
                    EXECUTABLE_IDENTIFIER_OR_CALL,
                    node.id,
                )
            )
        elif isinstance(node, ast.Attribute) and _ident_is_forbidden(node.attr):
            hits.append(
                TokenHit(
                    int(getattr(node, "lineno", 1) or 1),
                    EXECUTABLE_IDENTIFIER_OR_CALL,
                    node.attr,
                )
            )
        elif isinstance(node, ast.Call):
            fname = _call_name(node.func)
            if fname and _ident_is_forbidden(fname):
                hits.append(
                    TokenHit(
                        int(getattr(node, "lineno", 1) or 1),
                        EXECUTABLE_BEHAVIOR,
                        fname,
                    )
                )

    # Identifiers in tokenize that AST Name might miss (e.g. after syntax issues).
    return hits


def _is_str_node(node: ast.AST) -> bool:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return True
    return False


def _str_value(node: ast.AST) -> str:
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return ""


def _is_selector_string(value: str) -> bool:
    compact = value.strip().strip("\"'").lower().replace("_", "-")
    if compact in SELECTOR_VALUES:
        return True
    # comma/space separated selectors: "mock, fake"
    parts = [p.strip().lower() for p in re.split(r"[,|/]+", compact) if p.strip()]
    if parts and all(p in SELECTOR_VALUES for p in parts):
        return True
    return False


def _python_docstring_spans(tree: ast.AST) -> List[Tuple[int, int, int, int]]:
    spans: List[Tuple[int, int, int, int]] = []

    def consider(body: Optional[List[ast.stmt]]) -> None:
        if not body:
            return
        first = body[0]
        if isinstance(first, ast.Expr) and _is_str_node(first.value):
            n = first.value
            spans.append(
                (
                    int(n.lineno),
                    int(n.col_offset),
                    int(n.end_lineno or n.lineno),
                    int(n.end_col_offset or 0),
                )
            )

    consider(getattr(tree, "body", None))
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            consider(node.body)
    return spans


def _python_deny_list_spans(tree: ast.AST) -> List[Tuple[int, int, int, int]]:
    spans: List[Tuple[int, int, int, int]] = []

    def add_value(node: ast.AST) -> None:
        lineno = getattr(node, "lineno", None)
        if lineno is None:
            return
        spans.append(
            (
                int(node.lineno),
                int(getattr(node, "col_offset", 0) or 0),
                int(getattr(node, "end_lineno", node.lineno) or node.lineno),
                int(getattr(node, "end_col_offset", 0) or 0),
            )
        )

    def target_names(t: ast.AST) -> List[str]:
        names: List[str] = []
        if isinstance(t, ast.Name):
            names.append(t.id)
        elif isinstance(t, ast.Attribute):
            names.append(t.attr)
        elif isinstance(t, ast.Subscript):
            sl = t.slice
            if isinstance(sl, ast.Constant) and isinstance(sl.value, str):
                names.append(sl.value)
        elif isinstance(t, (ast.Tuple, ast.List)):
            for elt in t.elts:
                names.extend(target_names(elt))
        return names

    for node in ast.walk(tree):
        if isinstance(node, ast.Assign):
            names = []
            for t in node.targets:
                names.extend(target_names(t))
            if any(n in DENY_LIST_NAMES for n in names):
                add_value(node.value)
        elif isinstance(node, ast.AnnAssign) and node.value is not None:
            names = target_names(node.target)
            if any(n in DENY_LIST_NAMES for n in names):
                add_value(node.value)
        elif isinstance(node, ast.Dict):
            for key, val in zip(node.keys, node.values):
                if isinstance(key, ast.Constant) and isinstance(key.value, str):
                    if key.value in DENY_LIST_NAMES:
                        add_value(val)
    return spans


def _ident_is_forbidden(name: str) -> bool:
    if POLICY_IDENT_RE.fullmatch(name or ""):
        return False
    upper = (name or "").upper()
    if upper.startswith("NO_STUB_") or upper.startswith("USES_"):
        return False
    return bool(IDENT_SEGMENT_RE.search(name) or TOKEN_WORD_RE.search(name))


def _call_name(func: ast.AST) -> str:
    if isinstance(func, ast.Name):
        return func.id
    if isinstance(func, ast.Attribute):
        return func.attr
    return ""


def _index_c_like(text: str) -> List[TokenHit]:
    """Rust/C/JS: comments vs the rest. Do not globally strip quoted strings."""
    hits: List[TokenHit] = []
    in_block = False
    in_string: Optional[str] = None
    escape = False
    raw = text
    i = 0
    line = 1
    n = len(raw)
    comment_line_has_token: Set[int] = set()
    code_line_has_token: Set[int] = set()
    while i < n:
        ch = raw[i]
        nxt = raw[i + 1] if i + 1 < n else ""
        if ch == "\n":
            line += 1
            i += 1
            escape = False
            continue
        if in_block:
            if ch == "*" and nxt == "/":
                in_block = False
                i += 2
                continue
            i += 1
            continue
        if in_string is not None:
            if escape:
                escape = False
                i += 1
                continue
            if ch == "\\":
                escape = True
                i += 1
                continue
            if ch == in_string:
                in_string = None
            i += 1
            continue
        if ch == "/" and nxt == "*":
            in_block = True
            i += 2
            continue
        if ch == "/" and nxt == "/":
            rest = raw[i:].split("\n", 1)[0]
            if TOKEN_WORD_RE.search(rest):
                hits.append(TokenHit(line, NON_EXECUTABLE_COMMENT, rest[:160]))
                comment_line_has_token.add(line)
            i += len(rest)
            continue
        if ch in {'"', "'"}:
            in_string = ch
            i += 1
            continue
        i += 1
    # Second pass: line-level for remaining tokens (identifiers + string contents).
    for ln, src in enumerate(raw.splitlines(), 1):
        if not _line_mentions_token(src):
            continue
        if ln in comment_line_has_token and COMMENT_LINE_RE.match(src):
            continue
        stripped = src.lstrip()
        if stripped.startswith("//") or stripped.startswith("/*") or stripped.startswith("*"):
            hits.append(TokenHit(ln, NON_EXECUTABLE_COMMENT, stripped[:160]))
            continue
        if _line_has_selector_literal(src) or _line_has_identifier_token(src):
            role = (
                EXECUTABLE_IDENTIFIER_OR_CALL
                if _line_has_identifier_token(src)
                else EXECUTABLE_CONFIGURATION
            )
            hits.append(TokenHit(ln, role, stripped[:160]))
            code_line_has_token.add(ln)
        elif POLICY_PROSE_RE.search(src):
            hits.append(TokenHit(ln, POLICY_PROSE, stripped[:160]))
        else:
            hits.append(TokenHit(ln, UNKNOWN_AMBIGUOUS, stripped[:160]))
    return hits


def _line_has_selector_literal(line: str) -> bool:
    for m in re.finditer(r"""(?i)(['"])([^'"]*)\1""", line):
        if _is_selector_string(m.group(2)):
            return True
    # MODE=smoke / backend: mock (unquoted)
    if re.search(r"(?i)(?:mode|backend|kind|type|harness)\s*[=:]\s*smoke\b", line):
        return True
    if re.search(r"(?i)(?:mode|backend|kind)\s*[=:]\s*['\"]?(?:mock|fake|stub|smoke)", line):
        return True
    return False


def _line_has_identifier_token(line: str) -> bool:
    code = re.sub(r"""(?i)(['"]).*?\1""", "", line)
    code = re.sub(r"//.*", "", code)
    code = re.sub(r"#.*", "", code)
    code = POLICY_IDENT_RE.sub(" ", code)
    return bool(TOKEN_WORD_RE.search(code) or MOCK_FAKE_IDENT_RE.search(code))


def _index_shell(text: str) -> List[TokenHit]:
    hits: List[TokenHit] = []
    for ln, src in enumerate(text.splitlines(), 1):
        stripped = src.lstrip()
        if stripped.startswith("#") and not stripped.startswith("#!"):
            if TOKEN_WORD_RE.search(src):
                hits.append(TokenHit(ln, NON_EXECUTABLE_COMMENT, stripped[:160]))
            continue
        code = src.split(" #", 1)[0]
        # Smoke *mechanism* only: path, env flag, or MODE=smoke selector.
        # Do not treat a generic $MODE / ${MODE} campaign variable as smoke.
        if re.search(
            r"(?i)(?:scripts/smoke/|/smoke/|SMOKE_BACKEND\s*=|(?:^|[^\w])MODE=smoke\b|"
            r"scripts/\$\{?MODE\}?/)",
            code,
        ):
            hits.append(TokenHit(ln, EXECUTABLE_BEHAVIOR, code.strip()[:160]))
            continue
        if not TOKEN_WORD_RE.search(code):
            continue
        if _line_has_selector_literal(code) or _line_has_identifier_token(code):
            hits.append(TokenHit(ln, EXECUTABLE_CONFIGURATION, code.strip()[:160]))
        else:
            hits.append(TokenHit(ln, UNKNOWN_AMBIGUOUS, code.strip()[:160]))
    return hits


def _index_simple_mapping(text: str) -> List[TokenHit]:
    hits: List[TokenHit] = []
    for ln, src in enumerate(text.splitlines(), 1):
        stripped = src.strip()
        if stripped.startswith("#"):
            if TOKEN_WORD_RE.search(stripped):
                hits.append(TokenHit(ln, NON_EXECUTABLE_COMMENT, stripped[:160]))
            continue
        if not TOKEN_WORD_RE.search(stripped):
            continue
        key = stripped.split(":", 1)[0].strip().strip("\"'")
        val = stripped.split(":", 1)[1].strip() if ":" in stripped else ""
        if key in DENY_LIST_NAMES or key == "EXPLICIT_NON_SCOPE":
            hits.append(TokenHit(ln, DENY_LIST_PROSE, stripped[:160]))
            continue
        if POLICY_PROSE_RE.search(stripped) and not _is_selector_string(val.strip("\"'")):
            hits.append(TokenHit(ln, POLICY_PROSE, stripped[:160]))
            continue
        hits.append(TokenHit(ln, EXECUTABLE_CONFIGURATION, stripped[:160]))
    return hits


def _index_json(text: str) -> List[TokenHit]:
    hits: List[TokenHit] = []
    try:
        obj = json.loads(text)
    except json.JSONDecodeError:
        return _index_simple_mapping(text)

    def walk(node: object, parent_key: str = "") -> None:
        if isinstance(node, dict):
            for k, v in node.items():
                walk(v, str(k))
        elif isinstance(node, list):
            for elt in node:
                walk(elt, parent_key)
        elif isinstance(node, str) and TOKEN_WORD_RE.search(node):
            # JSON has no line map after loads; line hits come from text scan below.
            pass

    walk(obj)
    # Line-oriented with key context.
    for ln, src in enumerate(text.splitlines(), 1):
        if not TOKEN_WORD_RE.search(src) and "EXPLICIT_NON_SCOPE" not in src:
            continue
        key_m = re.search(r"""['"]?([A-Za-z_][A-Za-z0-9_]*)['"]?\s*:""", src)
        key = key_m.group(1) if key_m else ""
        if key in DENY_LIST_NAMES or "EXPLICIT_NON_SCOPE" in src:
            hits.append(TokenHit(ln, DENY_LIST_PROSE, src.strip()[:160]))
            continue
        if not TOKEN_WORD_RE.search(src):
            continue
        if POLICY_PROSE_RE.search(src) and not _line_has_selector_literal(src):
            hits.append(TokenHit(ln, POLICY_PROSE, src.strip()[:160]))
            continue
        hits.append(TokenHit(ln, EXECUTABLE_CONFIGURATION, src.strip()[:160]))
    return hits


def selftest_classifier() -> List[str]:
    """Return error strings; empty means PASS. Fixtures remain the primary corpus."""
    errors: List[str] = []

    def expect_nonexec(src: str, rel: str, label: str) -> None:
        if file_has_executable_token(src, rel):
            errors.append(f"false positive: {label}")

    def expect_exec(src: str, rel: str, label: str) -> None:
        if not file_has_executable_token(src, rel):
            errors.append(f"missed executable: {label}")

    expect_nonexec('"""No mock backend is used."""\n', "e2e_policy.py", "py docstring")
    expect_exec('"""mock"""\n', "evil_doc.py", "docstring exact selector")
    expect_nonexec("# simulations are forbidden\nx = 1\n", "mod.py", "py comment")
    expect_nonexec("// no fake response\nfn f() {}\n", "lib.rs", "rust comment")
    expect_nonexec(
        "EXPLICIT_NON_SCOPE = [\"smoke acceptance\"]\n",
        "cap.py",
        "deny-list",
    )
    expect_nonexec('POLICY = "STUB = FORBIDDEN"\n', "policy.py", "policy assignment")

    expect_exec('backend = "fake"\n', "cfg.py", "backend=fake")
    expect_exec('x = "mock"\nrun(x)\n', "run.py", "x=mock")
    expect_exec(
        "def test():\n    \"\"\"No mock allowed.\"\"\"\n    return create_mock_backend()\n",
        "e2e_mix.py",
        "docstring+create_mock",
    )
    expect_exec('let mode = "mock";\n', "main.rs", "rust mode=mock")
    expect_exec("backend: mock\n", "cfg.yaml", "yaml backend mock")
    expect_exec("MODE=smoke\n./scripts/$MODE/run.sh\n", "run.sh", "shell MODE=smoke")
    expect_exec("SMOKE_BACKEND=true\n", "env.sh", "SMOKE_BACKEND")
    expect_exec('mode = f"mock"\nrun(mode)\n', "fs.py", "f-string mock selector")
    expect_exec('{"backend": "mock"}\n', "cfg.json", "json backend mock")
    expect_exec("struct MockBackend;\n", "e2e.rs", "rust MockBackend ident")
    expect_exec('FORBIDDEN = "mock"\n', "bad_deny.py", "FORBIDDEN assigned selector")
    expect_nonexec(
        "MODE=probe\ncat >>\"$EV_DIR/${target}.${MODE}.meta.env\" <<EOF\nMODE=$MODE\nEOF\n",
        "p15-ws2-run.sh",
        "shell $MODE campaign not smoke",
    )
    expect_nonexec(
        "FAKE_DIG=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        "neg.sh",
        "shell FAKE_DIG negative digest",
    )
    expect_nonexec(
        "record P14SIGN_PHASE5_BUNDLE_MODE verify_no_fake_oci\n",
        "p5.sh",
        "shell no_fake_oci ident",
    )
    return errors
