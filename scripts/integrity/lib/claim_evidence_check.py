#!/usr/bin/env python3
"""CLAIM-EVIDENCE-INTEGRITY deterministic checker.

Verifies structured claim/evidence blocks for mechanically provable
mismatches. It does not read natural language, it does not judge whether a
claim is true, and it never upgrades an unparseable field to PASS.

A PASS from this checker means exactly: "no mechanically provable mismatch in
the declared block". It is not proof of the claim.

RULE_ID = CLAIM-EVIDENCE-INTEGRITY
FAIL_MODE = FAIL_CLOSED
Stdlib only. Read-only: never writes, never mutates product state.
"""
from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Set, Tuple

RULE_ID = "CLAIM-EVIDENCE-INTEGRITY"

BEGIN_MARKER = "BEGIN_CLAIM_EVIDENCE_BLOCK"
END_MARKER = "END_CLAIM_EVIDENCE_BLOCK"

SCAN_SUFFIXES = {".cei", ".txt", ".md", ".mdc"}
SKIP_DIR_NAMES = {".git", "target", "node_modules", ".exyonq-local", "__pycache__", ".venv", "venv"}

# --- vocabulary -------------------------------------------------------------

KNOWN_FIELDS = {
    "CLAIM_ID",
    "CLAIM",
    "CLAIM_KIND",
    "CLAIM_STATUS",
    "CLAIM_SCOPE",
    "CLAIM_PLATFORM",
    "CLAIM_CLIENT",
    "CLAIM_EXECUTION_CLASS",
    "CLAIM_SOURCE_STATE",
    "CLAIM_REQUIRED_EFFECT",
    "CLAIM_CONCURRENCY_CLASS",
    "EPISTEMIC_STATUS",
    "CONTRACT_SOURCE",
    "EVIDENCE",
    "EVIDENCE_COMMAND",
    "EVIDENCE_SCOPE",
    "EVIDENCE_PLATFORM",
    "EVIDENCE_CLIENT",
    "EVIDENCE_EXECUTION_CLASS",
    "EVIDENCE_SOURCE_STATE",
    "EVIDENCE_EXECUTION",
    "EVIDENCE_RESULT",
    "EVIDENCE_ORACLE_ORIGIN",
    "EVIDENCE_INDEPENDENCE",
    "EVIDENCE_OBSERVED_EFFECT",
    "MIRRORED_IMPLEMENTATION_ORACLE",
    "SELF_CONFIRMING_TEST_LOOP_RISK",
    "TARGET_CAUSAL_PATH",
    "PATH_EXECUTED",
    "PATH_EXECUTION_EVIDENCE",
    "CONCURRENCY_EVIDENCE_FORM",
    "CLAIM_EVIDENCE_COMPATIBILITY",
    "SOURCE_BINDING",
    "NOTE",
}

MATERIAL_STATUSES = {
    "PASS",
    "VERIFIED",
    "CLOSED",
    "FIXED",
    "ROOT_CAUSE",
    "PRODUCTION",
    "GLOBAL",
    # Terminal words this project actually uses elsewhere. An inflated claim
    # must not escape the material gates by picking a local synonym.
    "VERIFIED_REAL_PRODUCTION",
    "ACCEPT",
    "APPROVED",
    "COMPLETE",
    "DONE",
    "MERGE_READY",
    "RELEASE_APPROVED",
    "PROVEN",
}
NON_MATERIAL_STATUSES = {
    "FAIL",
    "PARTIAL",
    "BLOCKED",
    "NOT_PROVEN",
    "REVIEW",
    "IN_PROGRESS",
    "OPEN",
    "HYPOTHESIS",
    "DISPROVEN",
    "REJECT",
}

# Every field a material status must declare. Omission is not compliance: a
# check that silently skips its own trigger field would reward deletion.
REQUIRED_WHEN_MATERIAL = (
    "CLAIM_ID",
    "CLAIM",
    "CLAIM_KIND",
    "CLAIM_SCOPE",
    "CLAIM_PLATFORM",
    "CLAIM_CLIENT",
    "CLAIM_EXECUTION_CLASS",
    "CLAIM_SOURCE_STATE",
    "CLAIM_REQUIRED_EFFECT",
    "CLAIM_CONCURRENCY_CLASS",
    "EPISTEMIC_STATUS",
    "EVIDENCE",
    "EVIDENCE_COMMAND",
    "EVIDENCE_SCOPE",
    "EVIDENCE_PLATFORM",
    "EVIDENCE_CLIENT",
    "EVIDENCE_EXECUTION_CLASS",
    "EVIDENCE_SOURCE_STATE",
    "EVIDENCE_EXECUTION",
    "EVIDENCE_RESULT",
    "EVIDENCE_ORACLE_ORIGIN",
    "EVIDENCE_INDEPENDENCE",
    "EVIDENCE_OBSERVED_EFFECT",
    "CONTRACT_SOURCE",
    "MIRRORED_IMPLEMENTATION_ORACLE",
    "SELF_CONFIRMING_TEST_LOOP_RISK",
    "TARGET_CAUSAL_PATH",
    "PATH_EXECUTED",
    "PATH_EXECUTION_EVIDENCE",
    "CONCURRENCY_EVIDENCE_FORM",
    "CLAIM_EVIDENCE_COMPATIBILITY",
)

# Explicit supports-table. Not a numeric ladder: a class supports only the
# claim classes listed on its own row.
EXECUTION_SUPPORTS: Dict[str, Set[str]] = {
    "STATIC_ANALYSIS": {"STATIC_ANALYSIS"},
    "BUILD": {"BUILD"},
    "UNIT": {"UNIT"},
    "INTEGRATION": {"INTEGRATION", "UNIT"},
    "E2E": {"E2E", "INTEGRATION", "UNIT"},
    "REAL_RUNTIME": {"REAL_RUNTIME", "E2E", "INTEGRATION", "UNIT"},
    "REAL_CLIENT": {"REAL_CLIENT", "REAL_RUNTIME", "E2E", "INTEGRATION", "UNIT"},
    "FAILURE_PATH": {"FAILURE_PATH"},
    "SECURITY": {"SECURITY"},
    "BENCHMARK": {"BENCHMARK"},
    "ADVERSARIAL": {"ADVERSARIAL", "FAILURE_PATH"},
    "FORMAL_OR_PROPERTY": {"FORMAL_OR_PROPERTY"},
}
EXECUTION_CLASSES = set(EXECUTION_SUPPORTS)

# Scope is a partial order, not a ladder. MODULE and FILE are deliberately
# incomparable (a module may span files; a file may hold several modules), and
# FIXTURE / SINGLE_CASE support nothing but themselves.
SCOPE_SUPPORTS: Dict[str, Set[str]] = {
    "FUNCTION": {"FUNCTION"},
    "SINGLE_CASE": {"SINGLE_CASE"},
    "FIXTURE": {"FIXTURE"},
    "MODULE": {"MODULE", "FUNCTION"},
    "FILE": {"FILE", "FUNCTION"},
    "PACKAGE": {"PACKAGE", "CRATE", "MODULE", "FILE", "FUNCTION"},
    "CRATE": {"CRATE", "PACKAGE", "MODULE", "FILE", "FUNCTION"},
    "WORKSPACE": {"WORKSPACE", "PACKAGE", "CRATE", "MODULE", "FILE", "FUNCTION"},
    "REPOSITORY": {
        "REPOSITORY",
        "WORKSPACE",
        "PACKAGE",
        "CRATE",
        "MODULE",
        "FILE",
        "FUNCTION",
    },
    "GLOBAL": {
        "GLOBAL",
        "REPOSITORY",
        "WORKSPACE",
        "PACKAGE",
        "CRATE",
        "MODULE",
        "FILE",
        "FUNCTION",
    },
}
SCOPES = set(SCOPE_SUPPORTS)
WIDE_SCOPES = {"WORKSPACE", "REPOSITORY", "GLOBAL"}

PLATFORMS = {
    "LINUX_AMD64",
    "LINUX_ARM64",
    "DARWIN_ARM64",
    "DARWIN_AMD64",
    "WINDOWS_AMD64",
    "ALL_PLATFORMS",
    "NOT_APPLICABLE",
}

CLIENTS = {
    "MCP_STDIO",
    "CURSOR",
    "CLAUDE_DESKTOP",
    "VSCODE",
    "HTTP_CLIENT",
    "CURL",
    "BROWSER",
    "ALL_CLIENTS",
    "NOT_APPLICABLE",
}

EPISTEMIC_STATUSES = {
    "PROVEN",
    "STRONGLY_SUPPORTED",
    "BOUNDED",
    "PARTIAL",
    "HYPOTHESIS",
    "NOT_PROVEN",
    "DISPROVEN",
}

# EPISTEMIC_STATUS -> (severity for a material status, finding code)
EPISTEMIC_MATERIAL_BLOCK: Dict[str, tuple] = {
    "PROVEN": (None, ""),
    "STRONGLY_SUPPORTED": ("REVIEW", "EPISTEMIC_UNDERSUPPORT"),
    "BOUNDED": ("REVIEW", "EPISTEMIC_UNDERSUPPORT"),
    "PARTIAL": ("FAIL", "PARTIAL_AS_GLOBAL"),
    "HYPOTHESIS": ("FAIL", "HYPOTHESIS_AS_CAUSE"),
    "NOT_PROVEN": ("FAIL", "NOT_PROVEN_AS_PASS"),
    "DISPROVEN": ("FAIL", "DISPROVEN_AS_PASS"),
}

CLAIM_KINDS = {
    "CONTRACT_PROOF",
    "BEHAVIOR_PROOF",
    "EXECUTION_REPORT",
    "SCOPE_STATUS",
    "ROOT_CAUSE",
    "CAPABILITY_CLOSURE",
    "OTHER",
}

CONTRACT_SOURCES = {
    "PROJECT_DOC",
    "EXTERNAL_SPEC",
    "RFC",
    "PUBLIC_API",
    "SCHEMA",
    "PROTOCOL_SPEC",
    "OWNER_INVARIANT",
    "ARCHITECTURE_DOC",
    "IMPLEMENTATION",
    "NONE",
    "UNKNOWN",
}
WEAK_CONTRACT_SOURCES = {"IMPLEMENTATION", "NONE", "UNKNOWN"}

ORACLE_ORIGINS = {
    "CONTRACT_DOC",
    "PROJECT_DOC",
    "ARCHITECTURE_DOC",
    "EXTERNAL_SPEC",
    "RFC",
    "SCHEMA",
    "PROTOCOL_SPEC",
    "PUBLIC_API",
    "OWNER_INVARIANT",
    "INDEPENDENT_IMPLEMENTATION",
    "EXTERNAL_TOOL",
    "DIFFERENTIAL",
    "PROPERTY",
    "IMPLEMENTATION_DERIVED",
    "UNKNOWN",
}

ORACLE_INDEPENDENCE = {"INDEPENDENT", "PARTIALLY_INDEPENDENT", "IMPLEMENTATION_DERIVED", "UNKNOWN"}

EXECUTION_STATES = {"EXECUTED", "PARTIALLY_EXECUTED", "NOT_EXECUTED", "UNKNOWN"}
EVIDENCE_RESULTS = {"PASS", "FAIL", "PARTIAL", "BLOCKED", "NOT_RUN", "UNKNOWN"}

YES_NO = {"YES", "NO"}
RISK_LEVELS = {"NONE", "LOW", "MEDIUM", "HIGH"}
# ILLUSTRATIVE marks a block that demonstrates the format rather than claiming
# anything, so its commit ids are not expected to resolve. Declaring it is
# visible in the block and to the auditor; omitting it means the ids are real.
SOURCE_BINDING_KINDS = {"REAL", "ILLUSTRATIVE"}
# Statuses that assert causation and therefore owe an executed causal path.
REQUIRES_CAUSAL_PATH = {"ROOT_CAUSE", "FIXED"}
PATH_EXECUTED_VALUES = {"YES", "NO", "UNKNOWN", "NOT_APPLICABLE"}

CONCURRENCY_CLASSES = {
    "NONE",
    "CONCURRENCY",
    "TOCTOU",
    "CROSS_PROCESS",
    "OWNERSHIP_LIFECYCLE",
    "RACE",
    "LOCKING",
    "ATOMICITY",
    "PROCESS_LIFETIME",
    "SOURCE_STATE_RACE",
}

SUFFICIENT_CONCURRENCY_FORMS = {
    "DETERMINISTIC_INTERLEAVING",
    "BARRIER_CONTROLLED_RACE",
    "PROPERTY_TEST",
    "MODEL_CHECK",
    "STRESS_WITH_INDEPENDENT_ORACLE",
    "SYSTEMATIC_STATE_SEQUENCE",
    "REAL_CONCURRENT_PROCESS_REPRO",
}
INSUFFICIENT_CONCURRENCY_FORMS = {"SEQUENTIAL_TEST", "NONE", "HAPPY_PATH_TEST"}

# Effect and causal-path evidence are classified by a leading token from a
# closed set, followed by free text. Free text alone would let a synonym walk
# past the gate: "NO_LEAK_REPORTED" and "NO_FAILURE_OBSERVED" mean the same
# thing to a reader and nothing alike to a matcher. Unrecognized leading token
# is REVIEW, never PASS.
ABSENCE_EFFECTS = {"ABSENCE_OF_DEFECT", "NO_DEFECT", "DEFECT_ABSENT"}
NON_OBSERVATION_EFFECTS = {"NO_FAILURE_OBSERVED", "NOT_OBSERVED", "NONE", "NOT_EXECUTED"}
POSITIVE_EFFECTS = {"POSITIVE_OBSERVATION", "STATE_CHANGE_OBSERVED", "OUTPUT_MATCHED"}
EFFECT_KINDS = ABSENCE_EFFECTS | NON_OBSERVATION_EFFECTS | POSITIVE_EFFECTS | {"NOT_APPLICABLE"}

CAUSAL_EVIDENCE_KINDS = {
    "DIRECT_OBSERVATION",
    "TRACE",
    "LOG_CORRELATION",
    "INSTRUMENTATION",
    "DIFFERENTIAL",
    "STATUS_CODE_ONLY",
    "SAME_OUTPUT",
    "NOT_OBSERVED",
    "NONE",
    "NOT_APPLICABLE",
}
NON_CAUSAL_PATH_EVIDENCE = {"STATUS_CODE_ONLY", "SAME_OUTPUT", "NOT_OBSERVED", "NONE"}

# Flags that narrow a command below the whole workspace.
#   * exclusion is never cancelled: "--workspace --exclude x" still leaves x
#     unexamined;
#   * plural target selection is cancelled by --workspace, because "--workspace
#     --tests" widens the package set rather than narrowing it;
#   * a *named* target is never cancelled: "--workspace --test routing" runs
#     one test target across the workspace, not the workspace;
#   * package selection is cancelled by --workspace, and also by the runner
#     form "cargo run -p <runner> -- <work>", where -p names the tool being
#     run and not the scope of the work it does.
PACKAGE_NARROWING_CMD = re.compile(r"(?:^|\s)(?:-p|--package|--manifest-path)(?:\s|=|$)")
TARGET_NARROWING_CMD = re.compile(
    r"(?:^|\s)(?:--lib|--bins|--tests|--examples|--benches|--all-targets)(?:\s|$)"
)
# The argument-taking forms name one target and survive --workspace.
NAMED_TARGET_CMD = re.compile(
    r"(?:^|\s)(?:--bin|--test|--example|--bench)(?:\s+|=)[A-Za-z0-9_.:/-]+"
)
EXCLUDE_CMD = re.compile(r"(?:^|\s)--exclude(?:\s|=|$)")
WORKSPACE_WIDE_CMD = re.compile(r"(?:^|\s)(?:--workspace|--all)(?:\s|$)")
RUNNER_FORM_CMD = re.compile(r"\bcargo\s+run\b.*(?:^|\s)--(?:\s|$)")
# Any token that tells the checker something about the breadth of the run. A
# wide claim whose command contains none of these is not cross-checkable.
SCOPE_BEARING_CMD = re.compile(
    r"(?:^|\s)(?:--workspace|--all|--all-targets|--exclude|-p|--package|--manifest-path"
    r"|--lib|--bin|--bins|--test|--tests|--example|--examples|--bench|--benches)(?:\s|=|$)"
)
SOURCE_TOKEN = re.compile(r"\b(HEAD|TREE|STATE)\s*=\s*([A-Za-z0-9_.:/-]+)")
SHA_SHAPE = re.compile(r"^[0-9a-fA-F]+$")
MIN_SHA_LEN = 7

# Prose that asserts a universal the structured fields do not carry. Narrow on
# purpose: only phrases whose structured counterpart is unambiguous.
PROSE_UNIVERSALS: List[Tuple[str, re.Pattern, str]] = [
    (
        "PLATFORM",
        re.compile(r"(?i)\b(?:all|every|any)\s+(?:platform|platforms|architecture|architectures|arch|arches|os|oses)\b"),
        "CLAIM_PLATFORM is not ALL_PLATFORMS",
    ),
    (
        "CLIENT",
        re.compile(r"(?i)\b(?:all|every|any)\s+(?:client|clients)\b"),
        "CLAIM_CLIENT is not ALL_CLIENTS",
    ),
    (
        "SCOPE",
        re.compile(
            r"(?i)\b(?:global(?:ly)?|workspace[- ]wide|repo(?:sitory)?[- ]wide|everywhere)\b"
        ),
        "CLAIM_SCOPE is narrower than WORKSPACE",
    ),
    (
        "PRODUCTION",
        re.compile(r"(?i)\bproduction[- ]ready\b|\b(?:zero|no)\s+known\s+defects\b"),
        "CLAIM_EXECUTION_CLASS does not include REAL_RUNTIME or REAL_CLIENT",
    ),
]

CONCURRENCY_WORDS = re.compile(
    r"(?i)(?:\b(?:race|races|racy|concurrent(?:ly)?|concurrency|toctou|tocttou|interleav\w*|"
    r"atomic(?:ity)?|deadlock|livelock|simultaneous(?:ly)?|thread[- ]safe|lock[- ]free|"
    r"parallel|contention|reentran\w*|cancellation|cancelled|serializab\w*|"
    r"mutex|semaphore|await(?:ing|ed)?|torn|happens[- ]before|memory ordering|"
    r"critical section|lock ordering)\b"
    r"|check[- ]then[- ]act)"
)

# Wording that asserts causation. A block may not assert a cause in prose and
# disclaim the causal path in its fields.
CAUSAL_WORDS = re.compile(
    r"(?i)\b(?:caused by|causes|causing|root cause|because of|due to|"
    r"responsible for|stems from|arises from|attributable to|results from|"
    r"the fix (?:removes|eliminates|resolves|addresses))\b"
)

# Wording that names a platform or a client the structured field disclaims.
PLATFORM_WORDS = re.compile(
    r"(?i)\b(?:linux|darwin|macos|mac ?os|windows|arm64|aarch64|amd64|x86[_-]?64)\b"
)
CLIENT_WORDS = re.compile(r"(?i)\b(?:cursor|claude desktop|vs ?code|browser|curl)\b")


# --- model ------------------------------------------------------------------


@dataclass
class Finding:
    code: str
    severity: str  # FAIL | REVIEW
    detail: str


@dataclass
class Block:
    source: str
    line: int
    fields: Dict[str, str] = field(default_factory=dict)
    unknown_fields: List[str] = field(default_factory=list)
    duplicate_fields: List[str] = field(default_factory=list)
    malformed: List[str] = field(default_factory=list)

    @property
    def claim_id(self) -> str:
        return self.fields.get("CLAIM_ID", f"{Path(self.source).name}:{self.line}")

    def get(self, key: str) -> Optional[str]:
        val = self.fields.get(key)
        if val is None:
            return None
        val = val.strip()
        return val or None


# --- parsing ----------------------------------------------------------------


def parse_blocks(text: str, source: str) -> List[Block]:
    """Extract blocks. Markers must be alone on their line, so a document that
    merely mentions them in prose contains no block. A structurally broken
    block becomes a finding on that block; it never aborts the run, because an
    aborted run would hide the FAIL verdicts of every other block."""
    blocks: List[Block] = []
    current: Optional[Block] = None
    for lineno, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if line.startswith("#"):
            continue
        if line == BEGIN_MARKER:
            if current is not None:
                current.malformed.append(f"line {lineno}: nested {BEGIN_MARKER}, block not closed")
                blocks.append(current)
            current = Block(source=source, line=lineno)
            continue
        if line == END_MARKER:
            if current is None:
                orphan = Block(source=source, line=lineno)
                orphan.malformed.append(f"line {lineno}: {END_MARKER} without {BEGIN_MARKER}")
                blocks.append(orphan)
                continue
            blocks.append(current)
            current = None
            continue
        if current is None or not line:
            continue
        key, sep, value = line.partition("=")
        key = key.strip()
        if not sep or not key:
            current.malformed.append(f"line {lineno}: not a KEY = VALUE line: {line[:80]}")
            continue
        value = value.strip()
        if key in current.fields:
            current.duplicate_fields.append(key)
        if key not in KNOWN_FIELDS:
            current.unknown_fields.append(key)
        current.fields[key] = value
    if current is not None:
        current.malformed.append(f"unterminated block opened at line {current.line}")
        blocks.append(current)
    return blocks


def split_tokens(value: str) -> List[str]:
    return [t for t in re.split(r"[,\s]+", value.strip()) if t]


# Fields whose whole value is drawn from a closed vocabulary. Their tokens are
# checked by the enum machinery and must stay out of the prose scan, or
# EVIDENCE_CLIENT = MCP_STDIO would read as a sentence naming a client.
ENUM_ONLY_FIELDS = frozenset(
    {
        "CLAIM_KIND",
        "CLAIM_STATUS",
        "CLAIM_SCOPE",
        "CLAIM_PLATFORM",
        "CLAIM_CLIENT",
        "CLAIM_EXECUTION_CLASS",
        "CLAIM_CONCURRENCY_CLASS",
        "EPISTEMIC_STATUS",
        "CONTRACT_SOURCE",
        "EVIDENCE_SCOPE",
        "EVIDENCE_PLATFORM",
        "EVIDENCE_CLIENT",
        "EVIDENCE_EXECUTION_CLASS",
        "EVIDENCE_EXECUTION",
        "EVIDENCE_RESULT",
        "EVIDENCE_ORACLE_ORIGIN",
        "EVIDENCE_INDEPENDENCE",
        "MIRRORED_IMPLEMENTATION_ORACLE",
        "SELF_CONFIRMING_TEST_LOOP_RISK",
        "PATH_EXECUTED",
        "CONCURRENCY_EVIDENCE_FORM",
        "CLAIM_EVIDENCE_COMPATIBILITY",
        "SOURCE_BINDING",
        "CLAIM_SOURCE_STATE",
        "EVIDENCE_SOURCE_STATE",
        "CLAIM_ID",
    }
)

# Fields that are a classifier token followed by free text. Only the tail is
# prose; the token itself is already checked.
CLASSIFIER_FIELDS = frozenset(
    {"CLAIM_REQUIRED_EFFECT", "EVIDENCE_OBSERVED_EFFECT", "PATH_EXECUTION_EVIDENCE"}
)


def free_text_of(key: str, value: str) -> str:
    if key in ENUM_ONLY_FIELDS:
        return ""
    if key in CLASSIFIER_FIELDS:
        parts = value.strip().split(None, 1)
        return parts[1] if len(parts) > 1 else ""
    return value


def parse_source_state(value: Optional[str]) -> Dict[str, str]:
    out: Dict[str, str] = {}
    if not value:
        return out
    for key, val in SOURCE_TOKEN.findall(value):
        out[key.upper()] = val
    upper = value.upper()
    for bare in ("CURRENT_SOURCE", "UNKNOWN", "NOT_APPLICABLE"):
        if re.search(rf"\b{bare}\b", upper):
            out.setdefault("BARE", bare)
    return out


def sha_compatible(a: str, b: str) -> bool:
    a, b = a.lower(), b.lower()
    return a.startswith(b) or b.startswith(a)


# --- checks -----------------------------------------------------------------


def check_enum(
    block: Block, key: str, allowed: Set[str], findings: List[Finding], *, multi: bool = False
) -> Optional[List[str]]:
    raw = block.get(key)
    if raw is None:
        return None
    tokens = split_tokens(raw) if multi else [raw.strip()]
    bad = [t for t in tokens if t.upper() not in allowed]
    if bad:
        findings.append(
            Finding("UNKNOWN_TOKEN", "REVIEW", f"{key} has unrecognized value(s): {','.join(bad)}")
        )
        return None
    return [t.upper() for t in tokens]


def is_empty_template(block: Block) -> bool:
    """A block with no populated field asserts nothing. Documentation shows the
    empty template; treating it as a claim would make every document that
    explains the format unscannable."""
    return not block.malformed and not any(v.strip() for v in block.fields.values())


def evaluate(block: Block) -> List[Finding]:
    f: List[Finding] = []

    for reason in block.malformed:
        f.append(Finding("MALFORMED_BLOCK", "FAIL", reason))

    for key in sorted(set(block.unknown_fields)):
        f.append(Finding("UNKNOWN_FIELD", "REVIEW", f"unrecognized field: {key}"))
    for key in sorted(set(block.duplicate_fields)):
        f.append(Finding("DUPLICATE_FIELD", "REVIEW", f"field declared more than once: {key}"))

    status_raw = block.get("CLAIM_STATUS")
    if status_raw is None:
        f.append(Finding("MISSING_REQUIRED_FIELD", "REVIEW", "CLAIM_STATUS is required"))
        material = False
        status = ""
    else:
        status = status_raw.upper()
        if status in NON_MATERIAL_STATUSES:
            material = False
        elif status in MATERIAL_STATUSES:
            material = True
        else:
            # Fail closed: an unrecognized status is treated as material, so a
            # local synonym cannot switch off the evidence-quality gates.
            f.append(
                Finding(
                    "UNKNOWN_TOKEN",
                    "REVIEW",
                    f"CLAIM_STATUS unrecognized, treated as material: {status_raw}",
                )
            )
            material = True

    if material:
        for key in REQUIRED_WHEN_MATERIAL:
            if block.get(key) is None:
                f.append(
                    Finding(
                        "MISSING_REQUIRED_FIELD",
                        "REVIEW",
                        f"{key} is required for material status {status}",
                    )
                )

    claim_exec = check_enum(block, "CLAIM_EXECUTION_CLASS", EXECUTION_CLASSES, f, multi=True)
    ev_exec = check_enum(block, "EVIDENCE_EXECUTION_CLASS", EXECUTION_CLASSES, f, multi=True)
    if claim_exec and ev_exec:
        supported: Set[str] = set()
        for cls in ev_exec:
            supported |= EXECUTION_SUPPORTS[cls]
        missing = [c for c in claim_exec if c not in supported]
        if missing:
            f.append(
                Finding(
                    "EXECUTION_CLASS_INFLATION",
                    "FAIL",
                    f"evidence class {'+'.join(ev_exec)} does not support claim class "
                    f"{','.join(missing)}",
                )
            )

    claim_scope = check_enum(block, "CLAIM_SCOPE", SCOPES, f)
    ev_scope = check_enum(block, "EVIDENCE_SCOPE", SCOPES, f)
    if claim_scope and ev_scope:
        if claim_scope[0] not in SCOPE_SUPPORTS[ev_scope[0]]:
            # Inflation is a proven mismatch. Two scopes that simply do not
            # order against each other (MODULE vs FILE) are a modelling
            # ambiguity, and answering ambiguity with FAIL would be a guess.
            inverted = ev_scope[0] in SCOPE_SUPPORTS[claim_scope[0]]
            f.append(
                Finding(
                    "SCOPE_INFLATION" if inverted else "SCOPE_INCOMPARABLE",
                    "FAIL" if inverted else "REVIEW",
                    f"evidence scope {ev_scope[0]} does not support claim scope {claim_scope[0]}",
                )
            )

    command = block.get("EVIDENCE_COMMAND")
    wide_claim = bool(claim_scope) and claim_scope[0] in WIDE_SCOPES
    if wide_claim and command:
        wide_cmd = bool(WORKSPACE_WIDE_CMD.search(command))
        runner = bool(RUNNER_FORM_CMD.search(command))
        narrowed = (
            EXCLUDE_CMD.search(command)
            or NAMED_TARGET_CMD.search(command)
            or (TARGET_NARROWING_CMD.search(command) and not wide_cmd)
            or (PACKAGE_NARROWING_CMD.search(command) and not wide_cmd and not runner)
        )
        if narrowed:
            f.append(
                Finding(
                    "SCOPE_COMMAND_MISMATCH",
                    "FAIL",
                    f"{claim_scope[0]} claim but the command is narrowed: {command}",
                )
            )
        elif not wide_cmd and (runner or not SCOPE_BEARING_CMD.search(command)):
            f.append(
                Finding(
                    "SCOPE_COMMAND_UNVERIFIABLE",
                    "REVIEW",
                    f"{claim_scope[0]} claim but the command carries no recognized breadth "
                    f"flag, so what it covered is unchecked: {command}",
                )
            )
    elif wide_claim and material:
        f.append(
            Finding(
                "SCOPE_COMMAND_UNVERIFIABLE",
                "REVIEW",
                f"{claim_scope[0]} claim without EVIDENCE_COMMAND; the breadth of the run "
                "cannot be cross-checked",
            )
        )

    _prose_divergence_check(block, claim_scope, f)

    _set_subset_check(
        block, "PLATFORM", PLATFORMS, "ALL_PLATFORMS", "PLATFORM_SCOPE_INFLATION", material, f
    )
    _set_subset_check(
        block, "CLIENT", CLIENTS, "ALL_CLIENTS", "CLIENT_SCOPE_INFLATION", material, f
    )
    _runtime_dimension_check(block, material, claim_exec, f)

    _source_state_check(block, material, f)

    execution = check_enum(block, "EVIDENCE_EXECUTION", EXECUTION_STATES, f)
    if material and execution:
        if execution[0] == "NOT_EXECUTED":
            f.append(
                Finding(
                    "UNEXECUTED_CLAIM",
                    "FAIL",
                    f"material status {status} with EVIDENCE_EXECUTION=NOT_EXECUTED",
                )
            )
        elif execution[0] == "PARTIALLY_EXECUTED":
            f.append(
                Finding(
                    "PARTIAL_AS_GLOBAL",
                    "FAIL",
                    f"material status {status} with partially executed evidence",
                )
            )
        elif execution[0] == "UNKNOWN":
            f.append(Finding("EXECUTION_UNKNOWN", "REVIEW", "EVIDENCE_EXECUTION=UNKNOWN"))

    result = check_enum(block, "EVIDENCE_RESULT", EVIDENCE_RESULTS, f)
    if material and result:
        if result[0] in {"FAIL", "BLOCKED", "NOT_RUN"}:
            f.append(
                Finding(
                    "RESULT_NOT_PASS",
                    "FAIL",
                    f"material status {status} with EVIDENCE_RESULT={result[0]}",
                )
            )
        elif result[0] == "PARTIAL":
            f.append(
                Finding(
                    "PARTIAL_AS_GLOBAL",
                    "FAIL",
                    f"material status {status} with EVIDENCE_RESULT=PARTIAL",
                )
            )
        elif result[0] == "UNKNOWN":
            f.append(Finding("RESULT_UNKNOWN", "REVIEW", "EVIDENCE_RESULT=UNKNOWN"))

    epistemic = check_enum(block, "EPISTEMIC_STATUS", EPISTEMIC_STATUSES, f)
    if material and epistemic:
        severity, code = EPISTEMIC_MATERIAL_BLOCK[epistemic[0]]
        if severity:
            f.append(
                Finding(
                    code,
                    severity,
                    f"EPISTEMIC_STATUS={epistemic[0]} cannot carry material status {status}",
                )
            )

    kind_raw = (block.get("CLAIM_KIND") or "").strip().upper()
    # FIXED is a causal statement: it says this change removed that defect.
    # Keying only on the ROOT_CAUSE label would let the author pick the word
    # that switches the gate off.
    is_root_cause = status in REQUIRES_CAUSAL_PATH or kind_raw == "ROOT_CAUSE"

    _oracle_checks(block, material, f)
    _effect_checks(block, material, f)
    _causal_path_checks(block, is_root_cause, material, f)
    _concurrency_checks(block, f)
    _declared_verdict_check(block, f)

    return f


def _declared_verdict_check(block: Block, f: List[Finding]) -> None:
    """The author's own CLAIM_EVIDENCE_COMPATIBILITY is itself a claim, and it
    may not be stronger than what the block supports."""
    declared = check_enum(block, "CLAIM_EVIDENCE_COMPATIBILITY", {"PASS", "FAIL", "REVIEW"}, f)
    if not declared:
        return
    computed = verdict_for(f)
    rank = {"FAIL": 0, "REVIEW": 1, "PASS": 2}
    if rank[declared[0]] > rank[computed]:
        f.append(
            Finding(
                "DECLARED_VERDICT_INFLATED",
                "REVIEW",
                f"block declares CLAIM_EVIDENCE_COMPATIBILITY={declared[0]} but the "
                f"mechanical verdict is {computed}",
            )
        )
    elif rank[declared[0]] < rank[computed]:
        # The author declared a stricter verdict than the checker can derive.
        # They may know something no field encodes; discarding that and
        # printing PASS would let the exit code contradict the block itself.
        f.append(
            Finding(
                "DECLARED_VERDICT_HONORED",
                declared[0],
                f"block declares CLAIM_EVIDENCE_COMPATIBILITY={declared[0]}; the checker "
                f"found no mismatch of its own, and the stricter declaration stands",
            )
        )


def _prose_divergence_check(
    block: Block, claim_scope: Optional[List[str]], f: List[Finding]
) -> None:
    """Prose is what a human quotes. Flag the sentences that assert a universal
    the structured fields do not carry.

    Every field's free text is read, not a chosen few: reading only `CLAIM` and
    `EVIDENCE` would mean the same sentence passes by being written one field
    lower down. Enum fields are excluded because their own token vocabulary
    would trip the word lists. A report that declares no block at all is the
    auditor's problem, not the checker's."""
    prose = " ".join(
        free_text_of(k, block.get(k) or "") for k in block.fields
    ).strip()
    if not prose:
        return
    platform = {t.upper() for t in split_tokens(block.get("CLAIM_PLATFORM") or "")}
    client = {t.upper() for t in split_tokens(block.get("CLAIM_CLIENT") or "")}
    exec_cls = {t.upper() for t in split_tokens(block.get("CLAIM_EXECUTION_CLASS") or "")}
    scope = claim_scope[0] if claim_scope else ""

    for kind, words, present in (
        ("PLATFORM", PLATFORM_WORDS, platform),
        ("CLIENT", CLIENT_WORDS, client),
    ):
        if present == {"NOT_APPLICABLE"} and words.search(prose):
            f.append(
                Finding(
                    "DIMENSION_DISCLAIMED",
                    "REVIEW",
                    f"the wording names a {kind.lower()} but CLAIM_{kind}=NOT_APPLICABLE",
                )
            )

    concurrency = (block.get("CLAIM_CONCURRENCY_CLASS") or "").strip().upper()
    if concurrency in {"", "NONE"} and CONCURRENCY_WORDS.search(prose):
        f.append(
            Finding(
                "CONCURRENCY_CLASS_DIVERGENCE",
                "REVIEW",
                "the wording describes a race, ordering or locking property but "
                f"CLAIM_CONCURRENCY_CLASS={concurrency or 'MISSING'}",
            )
        )

    for kind, pattern, why in PROSE_UNIVERSALS:
        if not pattern.search(prose):
            continue
        if kind == "PLATFORM":
            divergent = "ALL_PLATFORMS" not in platform
        elif kind == "CLIENT":
            divergent = "ALL_CLIENTS" not in client
        elif kind == "SCOPE":
            divergent = scope not in WIDE_SCOPES
        else:
            divergent = not (exec_cls & {"REAL_RUNTIME", "REAL_CLIENT"})
        if divergent:
            f.append(
                Finding(
                    "PROSE_SCOPE_DIVERGENCE",
                    "REVIEW",
                    f"CLAIM wording asserts a {kind.lower()} universal but {why}",
                )
            )


def _set_subset_check(
    block: Block,
    kind: str,
    allowed: Set[str],
    universal: str,
    code: str,
    material: bool,
    f: List[Finding],
) -> None:
    claim = check_enum(block, f"CLAIM_{kind}", allowed, f, multi=True)
    evidence = check_enum(block, f"EVIDENCE_{kind}", allowed, f, multi=True)
    if not claim or not evidence:
        return
    claim_set = set(claim)
    ev_set = set(evidence)
    if claim_set == {"NOT_APPLICABLE"}:
        # Disclaiming a dimension is not the same as satisfying it. When the
        # evidence names a concrete value, the run did have that dimension and
        # the claim silently left itself unbounded on it.
        if material and ev_set != {"NOT_APPLICABLE"}:
            f.append(
                Finding(
                    "DIMENSION_DISCLAIMED",
                    "REVIEW",
                    f"CLAIM_{kind}=NOT_APPLICABLE while the evidence ran on "
                    f"{','.join(sorted(ev_set))}; the claim states no {kind.lower()} bound",
                )
            )
        return
    if universal in ev_set:
        return
    missing = sorted(claim_set - ev_set - {"NOT_APPLICABLE"})
    if missing:
        f.append(
            Finding(
                code,
                "FAIL",
                f"claim covers {','.join(sorted(claim_set))} but evidence covers "
                f"{','.join(sorted(ev_set))}",
            )
        )


def _runtime_dimension_check(
    block: Block, material: bool, claim_exec: Optional[List[str]], f: List[Finding]
) -> None:
    """Code that runs, runs somewhere.

    Disclaiming the platform on the claim side alone is caught by
    `_set_subset_check`. Disclaiming it on *both* sides is not, and that is the
    cheaper move: it removes the dimension from the comparison entirely. So
    whenever the evidence says the work was actually executed, a material claim
    owes a platform. Static analysis is the one honest exception — reading
    source is the same on every machine."""
    if not material:
        return
    classes = set(claim_exec or [])
    execution = (block.get("EVIDENCE_EXECUTION") or "").strip().upper()
    ran = execution in {"EXECUTED", "PARTIALLY_EXECUTED"}
    platform = {t.upper() for t in split_tokens(block.get("CLAIM_PLATFORM") or "")}
    client = {t.upper() for t in split_tokens(block.get("CLAIM_CLIENT") or "")}
    if platform == {"NOT_APPLICABLE"} and ran and classes and classes != {"STATIC_ANALYSIS"}:
        f.append(
            Finding(
                "DIMENSION_DISCLAIMED",
                "REVIEW",
                f"EVIDENCE_EXECUTION={execution} for a "
                f"{','.join(sorted(classes))} claim, so the work ran somewhere, "
                "but CLAIM_PLATFORM=NOT_APPLICABLE states no platform bound",
            )
        )
    # Only REAL_CLIENT. An E2E or REAL_RUNTIME claim about a server's own
    # behavior — reload, drain, a race between two of its threads — has no
    # client, and demanding one would make the honest block the one that gets
    # flagged. Wording that does name a client is caught by CLIENT_WORDS.
    if "REAL_CLIENT" in classes and client == {"NOT_APPLICABLE"}:
        f.append(
            Finding(
                "DIMENSION_DISCLAIMED",
                "REVIEW",
                "REAL_CLIENT claim with CLAIM_CLIENT=NOT_APPLICABLE",
            )
        )


_GIT_CACHE: Dict[str, Optional[Tuple[str, bool]]] = {}


def git_state(start: Path) -> Optional[Tuple[str, bool]]:
    """(HEAD sha, dirty) for the work tree containing `start`, or None."""
    key = str(start)
    if key in _GIT_CACHE:
        return _GIT_CACHE[key]
    result: Optional[Tuple[str, bool]] = None
    try:
        head = subprocess.run(
            ["git", "-C", key, "rev-parse", "HEAD"],
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        if head.returncode == 0 and head.stdout.strip():
            status = subprocess.run(
                ["git", "-C", key, "status", "--porcelain"],
                capture_output=True,
                text=True,
                timeout=30,
                check=False,
            )
            if status.returncode == 0:
                result = (head.stdout.strip(), bool(status.stdout.strip()))
    except (OSError, subprocess.SubprocessError):
        result = None
    _GIT_CACHE[key] = result
    return result


_OBJECT_CACHE: Dict[Tuple[str, str], bool] = {}


def git_object_exists(start: Path, sha: str) -> bool:
    key = (str(start), sha)
    if key in _OBJECT_CACHE:
        return _OBJECT_CACHE[key]
    exists = False
    try:
        proc = subprocess.run(
            ["git", "-C", str(start), "cat-file", "-e", f"{sha}^{{object}}"],
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        exists = proc.returncode == 0
    except (OSError, subprocess.SubprocessError):
        # Cannot answer, so cannot accuse. The caller only reaches here when
        # git already resolved a HEAD, so this is a narrow window.
        exists = True
    _OBJECT_CACHE[key] = exists
    return exists


def _source_state_check(block: Block, material: bool, f: List[Finding]) -> None:
    claim_raw = block.get("CLAIM_SOURCE_STATE")
    ev_raw = block.get("EVIDENCE_SOURCE_STATE")
    claim = parse_source_state(claim_raw)
    evidence = parse_source_state(ev_raw)

    for name, parsed, raw in (("CLAIM", claim, claim_raw), ("EVIDENCE", evidence, ev_raw)):
        if raw and not parsed:
            f.append(
                Finding(
                    "UNKNOWN_TOKEN",
                    "REVIEW",
                    f"{name}_SOURCE_STATE not parseable (expect HEAD=/TREE=/STATE= or "
                    f"CURRENT_SOURCE/UNKNOWN/NOT_APPLICABLE): {raw}",
                )
            )

    for name, parsed in (("CLAIM", claim), ("EVIDENCE", evidence)):
        for key in ("HEAD", "TREE"):
            val = parsed.get(key)
            if val is not None and not SHA_SHAPE.match(val):
                f.append(
                    Finding(
                        "UNKNOWN_TOKEN",
                        "REVIEW",
                        f"{name}_SOURCE_STATE {key}={val} is not a commit/tree id; "
                        "a branch or tag name does not bind a source state",
                    )
                )
                parsed.pop(key)

    for key in ("HEAD", "TREE"):
        if key not in claim or key not in evidence:
            continue
        if min(len(claim[key]), len(evidence[key])) < MIN_SHA_LEN:
            f.append(
                Finding(
                    "SHORT_SOURCE_BINDING",
                    "REVIEW",
                    f"{key} binding shorter than {MIN_SHA_LEN} characters; cannot compare",
                )
            )
        elif not sha_compatible(claim[key], evidence[key]):
            f.append(
                Finding(
                    "STALE_EVIDENCE",
                    "FAIL",
                    f"claim {key}={claim[key]} but evidence {key}={evidence[key]}",
                )
            )

    if claim.get("STATE", "").upper() == "CLEAN" and evidence.get("STATE", "").upper() == "DIRTY":
        f.append(
            Finding("SOURCE_STATE_MISMATCH", "FAIL", "clean-source claim with dirty-tree evidence")
        )

    says_current = claim.get("BARE") == "CURRENT_SOURCE"

    # A material claim is a claim about some state of the source. Declaring the
    # dimension inapplicable removes the binding rather than satisfying it, so
    # a three-week-old run could be offered for today's tree.
    if material and not says_current and "HEAD" not in claim and "TREE" not in claim:
        f.append(
            Finding(
                "SOURCE_BINDING_MISSING",
                "REVIEW",
                "material claim without a source binding: CLAIM_SOURCE_STATE declares "
                "neither HEAD=, TREE= nor CURRENT_SOURCE",
            )
        )

    if says_current and "HEAD" not in evidence:
        f.append(
            Finding(
                "SOURCE_BINDING_MISSING",
                "REVIEW",
                "current-source claim without HEAD binding in evidence",
            )
        )

    # Compare against the real repository whenever the block says it is about
    # the checkout in front of us — either by the CURRENT_SOURCE token or by
    # naming the commit that is actually checked out. A block that names some
    # other commit is history, and history is not checked against today's tree.
    claim_head = claim.get("HEAD")
    repo = git_state(Path(block.source).resolve().parent)
    binds_here = says_current
    if repo is not None and claim_head and len(claim_head) >= MIN_SHA_LEN:
        binds_here = binds_here or sha_compatible(claim_head, repo[0])

    if says_current and repo is None:
        f.append(
            Finding(
                "UNBOUND_SOURCE_STATE",
                "REVIEW",
                "current-source claim but the repository state could not be read",
            )
        )
    elif binds_here and repo is not None:
        head, dirty = repo
        ev_head = evidence.get("HEAD")
        if ev_head and len(ev_head) >= MIN_SHA_LEN and not sha_compatible(ev_head, head):
            f.append(
                Finding(
                    "STALE_EVIDENCE",
                    "FAIL",
                    f"claim binds to the current source but evidence HEAD={ev_head} is not "
                    f"the checked-out HEAD={head[:12]}",
                )
            )
        if dirty and claim.get("STATE", "").upper() == "CLEAN":
            f.append(
                Finding(
                    "SOURCE_STATE_MISMATCH",
                    "FAIL",
                    "clean-source claim but the work tree has uncommitted changes",
                )
            )

    if material and evidence.get("BARE") == "UNKNOWN":
        f.append(
            Finding("SOURCE_BINDING_MISSING", "REVIEW", "EVIDENCE_SOURCE_STATE is UNKNOWN")
        )

    # A well-formed sha is not a real one. Without this, `HEAD=cafebabecafebabe`
    # on both sides satisfies the binding requirement and makes every
    # comparison against it vacuous. Blocks that exist to illustrate the format
    # rather than to claim anything say so, and say it in the block.
    binding_kind = check_enum(block, "SOURCE_BINDING", SOURCE_BINDING_KINDS, f)
    illustrative = bool(binding_kind) and binding_kind[0] == "ILLUSTRATIVE"
    if repo is not None and not illustrative:
        for name, parsed in (("CLAIM", claim), ("EVIDENCE", evidence)):
            sha = parsed.get("HEAD") or parsed.get("TREE")
            if not sha or len(sha) < MIN_SHA_LEN:
                continue
            if not git_object_exists(Path(block.source).resolve().parent, sha):
                f.append(
                    Finding(
                        "UNRESOLVABLE_SOURCE_BINDING",
                        "REVIEW",
                        f"{name}_SOURCE_STATE names {sha}, which this repository "
                        "cannot resolve; the binding cannot be verified",
                    )
                )


def _oracle_checks(block: Block, material: bool, f: List[Finding]) -> None:
    kind = check_enum(block, "CLAIM_KIND", CLAIM_KINDS, f)
    contract_source = check_enum(block, "CONTRACT_SOURCE", CONTRACT_SOURCES, f)
    origin = check_enum(block, "EVIDENCE_ORACLE_ORIGIN", ORACLE_ORIGINS, f)
    independence = check_enum(block, "EVIDENCE_INDEPENDENCE", ORACLE_INDEPENDENCE, f)
    mirrored = check_enum(block, "MIRRORED_IMPLEMENTATION_ORACLE", YES_NO, f)
    risk = check_enum(block, "SELF_CONFIRMING_TEST_LOOP_RISK", RISK_LEVELS, f)

    if material and mirrored and mirrored[0] == "YES":
        f.append(
            Finding(
                "SELF_CONFIRMING_ORACLE",
                "FAIL",
                "test mirrors the implementation it validates; cannot close a material claim",
            )
        )
    if material and risk:
        if risk[0] == "HIGH":
            f.append(
                Finding(
                    "SELF_CONFIRMING_ORACLE",
                    "FAIL",
                    "SELF_CONFIRMING_TEST_LOOP_RISK=HIGH cannot close a material claim",
                )
            )
        elif risk[0] == "MEDIUM":
            f.append(
                Finding(
                    "SELF_CONFIRMING_RISK",
                    "REVIEW",
                    "SELF_CONFIRMING_TEST_LOOP_RISK=MEDIUM on a material claim",
                )
            )

    derived = (origin and origin[0] == "IMPLEMENTATION_DERIVED") or (
        independence and independence[0] == "IMPLEMENTATION_DERIVED"
    )
    # An implementation-derived oracle is the guard's own definition of a
    # self-confirming loop. Whether it is reported cannot depend on the
    # author's choice of CLAIM_KIND, or the label becomes the off-switch.
    if material and derived:
        f.append(
            Finding(
                "SELF_CONFIRMING_RISK",
                "REVIEW",
                "oracle is implementation-derived; the test and the code can be wrong "
                "in the same way",
            )
        )

    proof_kind = bool(kind) and kind[0] in {"CONTRACT_PROOF", "BEHAVIOR_PROOF"}
    if proof_kind:
        weak_contract = contract_source is None or contract_source[0] in WEAK_CONTRACT_SOURCES
        if weak_contract or derived:
            src = contract_source[0] if contract_source else "MISSING"
            f.append(
                Finding(
                    "TEST_PASS_AS_CONTRACT_PROOF",
                    "REVIEW",
                    f"{kind[0]} claim with CONTRACT_SOURCE={src} and implementation-derived oracle",
                )
            )

    if material and independence and independence[0] == "UNKNOWN":
        f.append(Finding("ORACLE_INDEPENDENCE_UNKNOWN", "REVIEW", "EVIDENCE_INDEPENDENCE=UNKNOWN"))


def classifier(block: Block, key: str, allowed: Set[str], f: List[Finding]) -> str:
    """First token of a `TOKEN free text` field, validated against a closed
    set. Returns "" when absent or unrecognized; an unrecognized token also
    raises REVIEW so it cannot be read as an innocent value."""
    raw = block.get(key)
    if raw is None:
        return ""
    token = split_tokens(raw)[0].upper() if split_tokens(raw) else ""
    if token in allowed:
        return token
    f.append(
        Finding(
            "UNKNOWN_TOKEN",
            "REVIEW",
            f"{key} must start with one of {'/'.join(sorted(allowed))}, got: {raw[:60]}",
        )
    )
    return ""


def _effect_checks(block: Block, material: bool, f: List[Finding]) -> None:
    req_tok = classifier(block, "CLAIM_REQUIRED_EFFECT", EFFECT_KINDS, f)
    obs_tok = classifier(block, "EVIDENCE_OBSERVED_EFFECT", EFFECT_KINDS, f)

    if not material:
        # Under-claiming is always allowed: an honest PARTIAL / NOT_PROVEN
        # report of an unobserved failure is the behavior this guard wants.
        return

    if "NOT_APPLICABLE" in (req_tok, obs_tok):
        f.append(
            Finding(
                "DIMENSION_DISCLAIMED",
                "REVIEW",
                "a material claim must state the effect it requires and the effect that "
                "was observed; NOT_APPLICABLE removes the comparison",
            )
        )
    elif req_tok in ABSENCE_EFFECTS and obs_tok in NON_OBSERVATION_EFFECTS:
        f.append(
            Finding(
                "NO_FAILURE_AS_NO_DEFECT",
                "FAIL",
                f"CLAIM_REQUIRED_EFFECT={req_tok} supported only by {obs_tok}",
            )
        )
    elif req_tok in ABSENCE_EFFECTS:
        f.append(
            Finding(
                "ABSENCE_CLAIM_REVIEW",
                "REVIEW",
                "absence-of-defect claims require adversarial or exhaustive evidence",
            )
        )
    elif obs_tok in NON_OBSERVATION_EFFECTS and req_tok not in NON_OBSERVATION_EFFECTS:
        f.append(
            Finding(
                "UNOBSERVED_EFFECT",
                "FAIL",
                f"material status with EVIDENCE_OBSERVED_EFFECT={obs_tok}",
            )
        )


def _causal_path_checks(
    block: Block, is_root_cause: bool, material: bool, f: List[Finding]
) -> None:
    target = block.get("TARGET_CAUSAL_PATH")
    if not target or target.strip().upper() in {"NOT_APPLICABLE", "NONE"}:
        if is_root_cause:
            f.append(
                Finding(
                    "CAUSAL_PATH_REQUIRED",
                    "FAIL",
                    "root-cause claim without a declared TARGET_CAUSAL_PATH",
                )
            )
        # Disclaiming the target must not switch off the classification of the
        # proof token: SAME_OUTPUT is offered as causal proof precisely when
        # the block would rather not name the path it did not execute.
        proof = classifier(block, "PATH_EXECUTION_EVIDENCE", CAUSAL_EVIDENCE_KINDS, f)
        if material and proof in {"STATUS_CODE_ONLY", "SAME_OUTPUT"}:
            f.append(
                Finding(
                    "WRONG_CAUSAL_PATH",
                    "FAIL",
                    f"PATH_EXECUTION_EVIDENCE={proof} offered while TARGET_CAUSAL_PATH is "
                    "disclaimed; the same output is not the same causal path",
                )
            )
        if material and not is_root_cause and CAUSAL_WORDS.search(block.get("CLAIM") or ""):
            f.append(
                Finding(
                    "CAUSAL_PATH_REQUIRED",
                    "REVIEW",
                    "the wording asserts causation but TARGET_CAUSAL_PATH is disclaimed",
                )
            )
        return
    executed = check_enum(block, "PATH_EXECUTED", PATH_EXECUTED_VALUES, f)
    if executed is None:
        f.append(
            Finding(
                "CAUSAL_PATH_UNVERIFIED",
                "REVIEW",
                "TARGET_CAUSAL_PATH declared without PATH_EXECUTED",
            )
        )
        return
    if executed[0] == "NO":
        f.append(Finding("WRONG_CAUSAL_PATH", "FAIL", f"target causal path not executed: {target}"))
        return
    if executed[0] in {"UNKNOWN", "NOT_APPLICABLE"}:
        f.append(
            Finding("CAUSAL_PATH_UNVERIFIED", "REVIEW", f"PATH_EXECUTED={executed[0]} for {target}")
        )
        return
    proof = classifier(block, "PATH_EXECUTION_EVIDENCE", CAUSAL_EVIDENCE_KINDS, f)
    if not proof:
        return
    if proof in NON_CAUSAL_PATH_EVIDENCE:
        f.append(
            Finding(
                "WRONG_CAUSAL_PATH",
                "FAIL",
                f"PATH_EXECUTED=YES but the evidence is {proof}; "
                "the same output is not the same causal path",
            )
        )


def _concurrency_checks(block: Block, f: List[Finding]) -> None:
    klass = check_enum(block, "CLAIM_CONCURRENCY_CLASS", CONCURRENCY_CLASSES, f)
    if not klass or klass[0] == "NONE":
        return
    raw = block.get("CONCURRENCY_EVIDENCE_FORM")
    if raw is None:
        f.append(
            Finding(
                "CONCURRENCY_EVIDENCE_GAP",
                "FAIL",
                f"{klass[0]} claim without CONCURRENCY_EVIDENCE_FORM",
            )
        )
        return
    forms = {t.upper() for t in split_tokens(raw)}
    unknown = forms - SUFFICIENT_CONCURRENCY_FORMS - INSUFFICIENT_CONCURRENCY_FORMS
    if unknown:
        f.append(
            Finding(
                "UNKNOWN_TOKEN",
                "REVIEW",
                f"CONCURRENCY_EVIDENCE_FORM unrecognized: {','.join(sorted(unknown))}",
            )
        )
        return
    if not forms & SUFFICIENT_CONCURRENCY_FORMS:
        f.append(
            Finding(
                "CONCURRENCY_EVIDENCE_GAP",
                "FAIL",
                f"{klass[0]} claim supported only by {','.join(sorted(forms))}",
            )
        )


def verdict_for(findings: Sequence[Finding]) -> str:
    if any(x.severity == "FAIL" for x in findings):
        return "FAIL"
    if any(x.severity == "REVIEW" for x in findings):
        return "REVIEW"
    return "PASS"


# --- driver -----------------------------------------------------------------


def iter_candidate_files(paths: Sequence[str]) -> List[Path]:
    out: List[Path] = []
    for raw in paths:
        p = Path(raw)
        if p.is_file():
            out.append(p)
            continue
        if not p.is_dir():
            raise FileNotFoundError(raw)
        for dirpath, dirnames, filenames in os.walk(p):
            dirnames[:] = [d for d in dirnames if d not in SKIP_DIR_NAMES]
            for fn in sorted(filenames):
                fp = Path(dirpath) / fn
                if fp.suffix.lower() in SCAN_SUFFIXES:
                    out.append(fp)
    return out


def cmd_check(args: argparse.Namespace) -> int:
    try:
        files = iter_candidate_files(args.paths)
    except FileNotFoundError as exc:
        print(f"ERROR: path not found: {exc}", file=sys.stderr)
        return 2

    explicit_files = {str(Path(p)) for p in args.paths if Path(p).is_file()}
    total = failed = reviewed = passed = 0
    parse_errors: List[str] = []

    print(f"RULE_ID={RULE_ID}")
    for fp in files:
        try:
            text = fp.read_text(encoding="utf-8", errors="replace")
        except OSError as exc:
            parse_errors.append(f"{fp}: {exc}")
            continue
        blocks = parse_blocks(text, str(fp)) if BEGIN_MARKER in text else []
        blocks = [b for b in blocks if not is_empty_template(b)]
        if not blocks:
            # A file handed to the checker on purpose and holding nothing
            # checkable is REVIEW, not PASS. Files reached by walking a
            # directory are simply skipped.
            if str(fp) in explicit_files:
                reviewed += 1
                total += 1
                print(f"CLAIM_ID={fp.name}")
                print("CLAIM_EVIDENCE_COMPATIBILITY=REVIEW")
                print("FINDINGS=NO_BLOCK_FOUND")
                print(f"DETAIL=REVIEW: NO_BLOCK_FOUND: no claim/evidence block in {fp}")
            continue
        for block in blocks:
            findings = evaluate(block)
            verdict = verdict_for(findings)
            total += 1
            if verdict == "FAIL":
                failed += 1
            elif verdict == "REVIEW":
                reviewed += 1
            else:
                passed += 1
            codes = sorted({x.code for x in findings})
            print(f"CLAIM_ID={block.claim_id}")
            print(f"SOURCE={block.source}:{block.line}")
            print(f"CLAIM_EVIDENCE_COMPATIBILITY={verdict}")
            print(f"FINDINGS={','.join(codes) if codes else 'NONE'}")
            for x in findings:
                print(f"DETAIL={x.severity}: {x.code}: {x.detail}")

    if parse_errors:
        for e in parse_errors:
            print(f"ERROR: {e}", file=sys.stderr)
        print("CEI_CHECK=CHECKER_ERROR")
        return 2

    print(f"BLOCKS_TOTAL={total}")
    print(f"BLOCKS_PASS={passed}")
    print(f"BLOCKS_FAIL={failed}")
    print(f"BLOCKS_REVIEW={reviewed}")
    if total == 0:
        # "I ran the checker over that tree and it passed" must not be
        # obtainable from a tree that contained nothing to check.
        print("FINDINGS=NO_BLOCK_FOUND")
        print("CEI_CHECK=REVIEW")
        return 3
    if failed:
        print("CEI_CHECK=FAIL")
        return 1
    if reviewed:
        print("CEI_CHECK=REVIEW")
        return 3
    print("CEI_CHECK=PASS")
    print("CEI_CHECK_MEANING=NO_MECHANICAL_MISMATCH_NOT_CLAIM_PROVEN")
    return 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    p = argparse.ArgumentParser(description="CLAIM-EVIDENCE-INTEGRITY deterministic checker")
    sub = p.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("check")
    c.add_argument("paths", nargs="+")
    c.set_defaults(func=cmd_check)
    args = p.parse_args(argv)
    return int(args.func(args))


if __name__ == "__main__":
    sys.exit(main())
