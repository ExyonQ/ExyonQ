#!/usr/bin/env python3
# Copyright 2026 Antonio Cantallops Alba — Apache-2.0
"""DB2D.1 — N12–N25 negative + P01–P10 positive scenarios (tempfile fixtures)."""

from __future__ import annotations

import subprocess
import textwrap
from pathlib import Path
from typing import Callable
from unittest import mock

from check_critical_provider_owners import run as run_011
from check_dependency_exceptions import run as run_006
from check_duplicate_dependencies import run as run_008
from check_hot_path_budget import run as run_007
from check_lock_and_manifest_policy import run as run_009
from check_manifest_zones import run as run_002
from check_public_provider_surface import run as run_003_004
from check_source_import_zones import run as run_002s
from common import (
    EXIT_CONFIG,
    EXIT_PASS,
    EXIT_VIOLATION,
    Finding,
    find_repo_root,
    worst_exit,
)
from run_negative_validation import (  # noqa: E402 — shared fixture helpers
    ScenarioResult,
    _dep_row,
    _ensure_dep,
    _has_gate_error,
    _write,
    base_fixture,
)


def _fill(result: ScenarioResult, hit: Finding | None) -> ScenarioResult:
    if hit is None:
        return result
    result.matched_gate = hit.gate_id
    result.matched_status = hit.status or (
        "VIOLATION" if hit.severity == "ERROR" else hit.severity
    )
    result.matched_severity = hit.severity
    result.matched_dependency = hit.dependency
    result.matched_file = hit.file
    result.matched_finding = hit.message
    return result


def _config_code(exc: BaseException) -> int:
    s = str(exc)
    if s[:1].isdigit():
        try:
            return int(s.split(":", 1)[0])
        except ValueError:
            return EXIT_CONFIG
    return EXIT_CONFIG


def scenario_n12_box_dyn(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(root / "core/src/server/handler.rs", "pub fn f(x: Box<dyn Send>) {}\n")
    findings = run_007(root)
    hit = _has_gate_error(findings, ["GATE-DEP-007"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and hit.status == "VIOLATION"
        and "box_dyn" in (hit.dependency + hit.message)
    )
    r = ScenarioResult(
        id="N12",
        title="New Box<dyn> on hot path",
        passed=ok,
        expected_gates=["GATE-DEP-007"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n13_pin_box_future(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "core/src/server/conn.rs",
        "use std::pin::Pin;\n"
        "type F = Pin<Box<dyn Future<Output = ()> + Send>>;\n"
        "pub fn f(_x: F) {}\n",
    )
    findings = run_007(root)
    hit = next(
        (
            f
            for f in findings
            if f.severity == "ERROR"
            and f.gate_id == "GATE-DEP-007"
            and f.status == "VIOLATION"
            and "pin_box_dyn_future" in (f.dependency + f.message)
        ),
        None,
    )
    ok = worst_exit(findings) == EXIT_VIOLATION and hit is not None
    r = ScenarioResult(
        id="N13",
        title="New Pin<Box<dyn Future>> on hot path",
        passed=ok,
        expected_gates=["GATE-DEP-007"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n14_body_collect(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "core/src/server/body_path.rs",
        "async fn handle(b: B) { let _ = http_body_util::BodyExt::collect(b).await; }\n",
    )
    findings = run_007(root)
    hit = _has_gate_error(findings, ["GATE-DEP-007"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and hit.status == "VIOLATION"
        and "bodyext_collect" in (hit.dependency + hit.message)
    )
    r = ScenarioResult(
        id="N14",
        title="New BodyExt::collect on request/body path",
        passed=ok,
        expected_gates=["GATE-DEP-007"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n15_baseline_count_exceeded(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
        """
        schema_version = 1
        [[hot_path_zone]]
        path = "core/src/server/"
        [[pattern]]
        id = "HP-TEST-ARC"
        pattern_class = "arc_dyn"
        file = "core/src/server/handler.rs"
        symbol_or_line_scope = "f"
        observed_count = 1
        status = "temporary"
        policy_reference = "test"
        exception_id = ""
        expansion_allowed = false
        review_trigger = "t"
        """,
    )
    _write(
        root / "core/src/server/handler.rs",
        "fn f(a: Arc<dyn Send>) {}\n"
        "fn g(b: Arc<dyn Sync>) {}\n",
    )
    findings = run_007(root)
    hit = _has_gate_error(findings, ["GATE-DEP-007"])
    msg = (hit.message if hit else "").lower()
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and ("baseline" in msg or "observed=" in msg or "count" in msg)
    )
    r = ScenarioResult(
        id="N15",
        title="Hot-path baseline count exceeded",
        passed=ok,
        expected_gates=["GATE-DEP-007"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n16_temporary_no_review(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    path = root / "docs/architecture/dependency-boundaries/dependency-exceptions.toml"
    text = path.read_text(encoding="utf-8")
    # Empty review_trigger on DBEX-005 (temporary)
    text = text.replace(
        'id = "DBEX-005"\n        dependency = "hyper"\n        status = "temporary"',
        'id = "DBEX-005"\n        dependency = "hyper"\n        status = "temporary"',
    )
    # Force empty review_trigger for DBEX-005 block
    parts = text.split('id = "DBEX-005"')
    if len(parts) != 2:
        return ScenarioResult(
            id="N16",
            title="DBEX TEMPORARY without review_trigger",
            passed=False,
            expected_gates=["GATE-DEP-006"],
            expected_exit=EXIT_VIOLATION,
            actual_exit=2,
            matched_finding="DBEX-005 block not found",
        )
    tail = parts[1]
    # replace first review_trigger in that block
    import re

    tail2, n = re.subn(
        r'review_trigger = "[^"]*"',
        'review_trigger = ""',
        tail,
        count=1,
    )
    if n != 1:
        return ScenarioResult(
            id="N16",
            title="DBEX TEMPORARY without review_trigger",
            passed=False,
            expected_gates=["GATE-DEP-006"],
            expected_exit=EXIT_VIOLATION,
            actual_exit=2,
            matched_finding="review_trigger rewrite failed",
        )
    _write(path, parts[0] + 'id = "DBEX-005"' + tail2)
    findings = run_006(root)
    hit = _has_gate_error(findings, ["GATE-DEP-006"])
    ok = worst_exit(findings) == EXIT_VIOLATION and hit is not None and (
        "review_trigger" in hit.message.lower()
    )
    r = ScenarioResult(
        id="N16",
        title="DBEX TEMPORARY without review_trigger",
        passed=ok,
        expected_gates=["GATE-DEP-006"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n17_unknown_dbex(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    path = root / "docs/architecture/dependency-boundaries/dependency-exceptions.toml"
    _write(
        path,
        path.read_text(encoding="utf-8")
        + textwrap.dedent(
            """
            [[exception]]
            id = "DBEX-999"
            dependency = "fake"
            status = "temporary"
            exact_scope = "nowhere"
            allowed_type_or_pattern = "x"
            owner = "test"
            policy_reference = "invented-not-db1"
            performance_reason = "n"
            security_impact = "n"
            replacement_required = true
            review_trigger = "soon"
            expansion_allowed = false
            test_or_bench_evidence = "n"
            notes = "n"
            """
        ),
    )
    findings = run_006(root)
    hit = _has_gate_error(findings, ["GATE-DEP-006"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and ("DBEX-999" in hit.message or hit.exception_id == "DBEX-999" or hit.symbol == "DBEX-999")
    )
    r = ScenarioResult(
        id="N17",
        title="Unknown DBEX-999",
        passed=ok,
        expected_gates=["GATE-DEP-006"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n18_baseline_missing_exception(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
        """
        schema_version = 1
        [[hot_path_zone]]
        path = "core/src/server/"
        [[pattern]]
        id = "HP-BAD-EXC"
        pattern_class = "arc_dyn"
        file = "core/src/server/handler.rs"
        symbol_or_line_scope = "f"
        observed_count = 0
        status = "temporary"
        policy_reference = "test"
        exception_id = "DBEX-999"
        expansion_allowed = false
        review_trigger = "t"
        """,
    )
    findings = run_006(root)
    hit = _has_gate_error(findings, ["GATE-DEP-006"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and "DBEX-999" in (hit.message + hit.exception_id)
    )
    r = ScenarioResult(
        id="N18",
        title="Baseline references nonexistent exception",
        passed=ok,
        expected_gates=["GATE-DEP-006"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def _socket2_registry(root: Path) -> None:
    reg = root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
    _write(
        reg,
        "schema_version = 1\n"
        + _dep_row(
            "bytes",
            owners=[
                "exyonq-core",
                "exyonq-module-api",
                "exyonq-runtime-plan",
                "exyonq-platform-linux",
                "exyonq-wasm-host",
            ],
        )
        + """
        [[dependency]]
        name = "socket2"
        class = "D"
        authorized_owners = ["exyonq-platform-linux"]
        authorized_public_surface = "test"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "n/a"
        version_policy = "test"
        status = "BINDING"
        allowed_lock_versions = ["0.5.10", "0.6.4"]
        """,
    )


def scenario_n19_third_socket2(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _socket2_registry(root)
    _write(root / "Cargo.lock", "")
    with mock.patch(
        "check_duplicate_dependencies.run_cargo_tree_d",
        return_value="socket2 v0.5.10\nsocket2 v0.6.4\nsocket2 v0.7.0\n",
    ):
        findings = run_008(root)
    hit = _has_gate_error(findings, ["GATE-DEP-008"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and hit.status == "VIOLATION"
        and hit.dependency == "socket2"
    )
    r = ScenarioResult(
        id="N19",
        title="Third socket2 version",
        passed=ok,
        expected_gates=["GATE-DEP-008"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n20_generic_dup_warn(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _socket2_registry(root)
    _write(root / "Cargo.lock", "")
    with mock.patch(
        "check_duplicate_dependencies.run_cargo_tree_d",
        return_value="bitflags v1.0.0\nbitflags v2.0.0\nsocket2 v0.5.10\nsocket2 v0.6.4\n",
    ):
        findings = run_008(root)
    exit_code = worst_exit(findings)
    hit = next(
        (
            f
            for f in findings
            if f.dependency == "bitflags"
            and f.severity == "WARN"
            and f.status in {"REVIEW_REQUIRED", "REVIEW"}
        ),
        None,
    )
    # fail-open: exit 0 despite WARN
    ok = exit_code == EXIT_PASS and hit is not None
    r = ScenarioResult(
        id="N20",
        title="Generic duplicate as WARN (fail-open)",
        passed=ok,
        expected_gates=["GATE-DEP-008"],
        expected_exit=EXIT_PASS,
        actual_exit=exit_code,
        matched_finding=(hit.message if hit else "NONE"),
        notes="FAIL_OPEN: WARN/REVIEW without exit 1",
    )
    return _fill(r, hit)


def scenario_n21_known_debt_exact(tmp: Path) -> ScenarioResult:
    """Baselined socket2 dual-version → KNOWN_DEBT, no VIOLATION."""
    root = base_fixture(tmp)
    _socket2_registry(root)
    _write(root / "Cargo.lock", "")
    with mock.patch(
        "check_duplicate_dependencies.run_cargo_tree_d",
        return_value="socket2 v0.5.10\nsocket2 v0.6.4\n",
    ):
        findings = run_008(root)
    exit_code = worst_exit(findings)
    known = next(
        (f for f in findings if f.dependency == "socket2" and f.status == "KNOWN_DEBT"),
        None,
    )
    viol = any(f.severity == "ERROR" for f in findings)
    ok = exit_code == EXIT_PASS and known is not None and not viol
    r = ScenarioResult(
        id="N21",
        title="Known debt exact (socket2 dual-version)",
        passed=ok,
        expected_gates=["GATE-DEP-008"],
        expected_exit=EXIT_PASS,
        actual_exit=exit_code,
        matched_finding=(known.message if known else "NONE"),
    )
    return _fill(r, known)


def scenario_n22_comments_strings_ignored(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(
        root,
        name="rustls",
        owners=["exyonq-mod-tls", "exyonq-core"],
        forbidden=["exyonq-module-api"],
    )
    _write(
        root / "module-api/src/lib.rs",
        "// pub use rustls::ServerConfig;\n"
        'let x = "tokio::net::TcpStream";\n'
        'let y = r#"BodyExt::collect"#;\n'
        "pub fn ok() {}\n",
    )
    _write(
        root / "core/src/server/handler.rs",
        "// Arc<dyn Send>\n"
        'let s = "BoxBody";\n'
        "pub fn ok() {}\n",
    )
    findings = run_003_004(root) + run_011(root) + run_007(root) + run_002s(root)
    critical = [
        f
        for f in findings
        if f.severity == "ERROR"
        and any(
            x in (f.message + f.dependency + f.symbol)
            for x in ("rustls", "TcpStream", "BodyExt", "boxbody", "arc_dyn", "bodyext")
        )
    ]
    ok = worst_exit(findings) == EXIT_PASS or (
        # may have unrelated noise; require no ERROR from comment/string fixtures
        not critical
    )
    # Prefer strict: no ERRORs at all from these checkers on clean comments
    ok = len(critical) == 0 and not any(
        f.severity == "ERROR" and "forbidden public re-export" in f.message for f in findings
    )
    # Also no hot-path ERROR for commented patterns
    ok = not any(f.severity == "ERROR" for f in findings)
    r = ScenarioResult(
        id="N22",
        title="Comments and strings ignored",
        passed=ok,
        expected_gates=["GATE-DEP-003", "GATE-DEP-007", "GATE-DEP-011"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="no critical findings from comments/strings",
    )
    return r


def scenario_n23_cargo_alias(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(
        root,
        name="tokio",
        owners=["exyonq-core", "exyonq-module-api"],
        forbidden=["exyonq-runtime-plan"],
    )
    cargo = (root / "runtime-plan/Cargo.toml").read_text(encoding="utf-8")
    _write(
        root / "runtime-plan/Cargo.toml",
        cargo + '\nmy_tokio = { package = "tokio", version = "1" }\n',
    )
    findings = run_002(root)
    hit = _has_gate_error(findings, ["GATE-DEP-002"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and hit.dependency == "tokio"
    )
    r = ScenarioResult(
        id="N23",
        title="Cargo alias package=tokio in forbidden owner",
        passed=ok,
        expected_gates=["GATE-DEP-002"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n24_external_path(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    cargo = (root / "core/Cargo.toml").read_text(encoding="utf-8")
    _write(
        root / "core/Cargo.toml",
        cargo + '\noutside_vendor = { path = "/tmp/exyonq-db2d1-outside-vendor" }\n',
    )
    # register so 001 doesn't dominate — 009 owns path_external
    _ensure_dep(root, name="outside_vendor", owners=["exyonq-core"])
    findings = run_009(root)
    hit = _has_gate_error(findings, ["GATE-DEP-009"])
    ok = (
        worst_exit(findings) == EXIT_VIOLATION
        and hit is not None
        and "path" in hit.message.lower()
    )
    r = ScenarioResult(
        id="N24",
        title="External path dependency",
        passed=ok,
        expected_gates=["GATE-DEP-009"],
        expected_exit=EXIT_VIOLATION,
        actual_exit=worst_exit(findings),
        matched_finding=(hit.message if hit else "NONE"),
    )
    return _fill(r, hit)


def scenario_n25_invalid_registry(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
        "schema_version = 1\n[[dependency\nthis is not valid toml {{{{\n",
    )
    exit_code = EXIT_PASS
    msg = ""
    try:
        from check_manifest_registry import run as run_001

        run_001(root)
    except SystemExit as exc:
        exit_code = _config_code(exc)
        msg = str(exc)
    ok = exit_code == EXIT_CONFIG
    return ScenarioResult(
        id="N25",
        title="Invalid registry TOML",
        passed=ok,
        expected_gates=["GATE-DEP-001"],
        expected_exit=EXIT_CONFIG,
        actual_exit=exit_code,
        matched_finding=msg or "NONE",
        matched_severity="CONFIG",
        matched_status="CONFIGURATION_OR_TOOL_FAILURE",
        notes="EXIT=2 required; exit 1 not accepted",
    )


# --- Positive scenarios ---


def scenario_p01_bytes(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(root / "core/src/lib.rs", "pub fn f(b: bytes::Bytes) { let _ = b; }\n")
    findings = run_003_004(root) + run_011(root)
    viol = [f for f in findings if f.severity == "ERROR"]
    ok = worst_exit(findings) == EXIT_PASS and not viol
    return ScenarioResult(
        id="P01",
        title="bytes::Bytes under DBEX-001",
        passed=ok,
        expected_gates=["GATE-DEP-003", "GATE-DEP-011"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="no violation for bytes::Bytes",
    )


def scenario_p02_http(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(root, name="http", owners=["exyonq-core", "exyonq-module-api"])
    cargo = (root / "core/Cargo.toml").read_text(encoding="utf-8")
    _write(root / "core/Cargo.toml", cargo + "\nhttp = { workspace = true }\n")
    _write(root / "core/src/lib.rs", "pub fn m() -> http::Method { http::Method::GET }\n")
    findings = run_002(root) + run_003_004(root) + run_011(root)
    ok = worst_exit(findings) == EXIT_PASS
    return ScenarioResult(
        id="P02",
        title="http types under DBEX-002",
        passed=ok,
        expected_gates=["GATE-DEP-002", "GATE-DEP-004"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="no violation for authorized http",
    )


def scenario_p03_dbex003_exact(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(root, name="tokio", owners=["exyonq-core", "exyonq-module-api"])
    cargo = (root / "module-api/Cargo.toml").read_text(encoding="utf-8")
    _write(root / "module-api/Cargo.toml", cargo + "\ntokio = { workspace = true }\n")
    _write(
        root / "module-api/src/static_wire.rs",
        "pub type Stream = tokio::net::TcpStream;\n",
    )
    _write(root / "module-api/src/lib.rs", "pub mod static_wire;\n")
    findings = run_011(root) + run_006(root)
    viol = [f for f in findings if f.severity == "ERROR"]
    accepted = any(
        f.status == "ACCEPTED" and "DBEX-003" in (f.exception_id + f.message + f.baseline_id)
        for f in findings
    )
    ok = worst_exit(findings) == EXIT_PASS and not viol and accepted
    return ScenarioResult(
        id="P03",
        title="DBEX-003 exact static_wire TcpStream",
        passed=ok,
        expected_gates=["GATE-DEP-011", "GATE-DEP-006"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="DBEX-003 accepted" if accepted else "missing ACCEPTED",
    )


def scenario_p04_hyper_owner(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(root, name="hyper", owners=["exyonq-core"])
    cargo = (root / "core/Cargo.toml").read_text(encoding="utf-8")
    _write(root / "core/Cargo.toml", cargo + "\nhyper = { workspace = true }\n")
    _write(root / "core/src/lib.rs", "use hyper::Uri;\npub fn f(_u: Uri) {}\n")
    findings = run_002(root) + run_011(root)
    ok = worst_exit(findings) == EXIT_PASS
    return ScenarioResult(
        id="P04",
        title="Hyper inside authorized owner",
        passed=ok,
        expected_gates=["GATE-DEP-002", "GATE-DEP-011"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="no violation for hyper in core",
    )


def scenario_p05_anyhow_cli(tmp: Path) -> ScenarioResult:
    """Positive against real tree: anyhow in CLI/xtask owners."""
    real = find_repo_root()
    from check_manifest_zones import run as rz

    findings = rz(real)
    viol = [
        f
        for f in findings
        if f.severity == "ERROR" and f.dependency == "anyhow"
    ]
    ok = not viol
    return ScenarioResult(
        id="P05",
        title="anyhow in CLI/xtask (real tree)",
        passed=ok,
        expected_gates=["GATE-DEP-002"],
        expected_exit=EXIT_PASS,
        actual_exit=EXIT_PASS if ok else EXIT_VIOLATION,
        matched_finding="no anyhow zone violations",
        notes="real-tree read-only",
    )


def scenario_p06_socket2_exact(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _socket2_registry(root)
    _write(root / "Cargo.lock", "")
    with mock.patch(
        "check_duplicate_dependencies.run_cargo_tree_d",
        return_value="socket2 v0.5.10\nsocket2 v0.6.4\n",
    ):
        findings = run_008(root)
    ok = (
        worst_exit(findings) == EXIT_PASS
        and any(f.status == "KNOWN_DEBT" and f.dependency == "socket2" for f in findings)
        and not any(f.severity == "ERROR" for f in findings)
    )
    return ScenarioResult(
        id="P06",
        title="socket2 0.5.10 + 0.6.4 exact",
        passed=ok,
        expected_gates=["GATE-DEP-008"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="KNOWN_DEBT exact dual-version",
    )


def scenario_p07_hotpath_baselined(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _write(
        root / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
        """
        schema_version = 1
        [[hot_path_zone]]
        path = "core/src/server/"
        [[pattern]]
        id = "HP-EXACT"
        pattern_class = "arc_dyn"
        file = "core/src/server/handler.rs"
        symbol_or_line_scope = "f"
        observed_count = 1
        status = "temporary"
        policy_reference = "test"
        exception_id = ""
        expansion_allowed = false
        review_trigger = "t"
        """,
    )
    _write(root / "core/src/server/handler.rs", "fn f(x: Arc<dyn Send>) {}\n")
    findings = run_007(root)
    ok = worst_exit(findings) == EXIT_PASS and not any(f.severity == "ERROR" for f in findings)
    return ScenarioResult(
        id="P07",
        title="Hot-path exact baselined pattern",
        passed=ok,
        expected_gates=["GATE-DEP-007"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="exact baseline match",
    )


def scenario_p08_wasmtime_host(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(root, name="wasmtime", owners=["exyonq-wasm-host"])
    cargo = (root / "wasm-host/Cargo.toml").read_text(encoding="utf-8")
    _write(root / "wasm-host/Cargo.toml", cargo + "\nwasmtime = { workspace = true }\n")
    _write(root / "wasm-host/src/lib.rs", "use wasmtime::Engine;\npub fn e() -> Engine { todo!() }\n")
    findings = run_002(root) + run_011(root)
    ok = worst_exit(findings) == EXIT_PASS
    return ScenarioResult(
        id="P08",
        title="Wasmtime inside wasm-host leaf",
        passed=ok,
        expected_gates=["GATE-DEP-002", "GATE-DEP-011"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="wasmtime allowed in wasm-host",
    )


def scenario_p09_pub_crate(tmp: Path) -> ScenarioResult:
    root = base_fixture(tmp)
    _ensure_dep(
        root,
        name="rustls",
        owners=["exyonq-mod-tls", "exyonq-core", "exyonq-module-api"],
    )
    cargo = (root / "module-api/Cargo.toml").read_text(encoding="utf-8")
    _write(root / "module-api/Cargo.toml", cargo + "\nrustls = { workspace = true }\n")
    _write(root / "module-api/src/lib.rs", "pub(crate) use rustls::ServerConfig;\n")
    findings = run_003_004(root)
    viol = [f for f in findings if f.severity == "ERROR" and "re-export" in f.message]
    ok = worst_exit(findings) == EXIT_PASS and not viol
    return ScenarioResult(
        id="P09",
        title="pub(crate) rustls allowed (no public re-export)",
        passed=ok,
        expected_gates=["GATE-DEP-003"],
        expected_exit=EXIT_PASS,
        actual_exit=worst_exit(findings),
        matched_finding="pub(crate) not treated as public re-export",
    )


def scenario_p10_real_tree(tmp: Path) -> ScenarioResult:
    del tmp  # unused — real tree
    real = find_repo_root()
    proc = subprocess.run(
        ["bash", str(real / "scripts/architecture/verify-dependency-containment.sh"), "--check"],
        cwd=real,
        capture_output=True,
        text=True,
        check=False,
    )
    ok = proc.returncode == 0
    return ScenarioResult(
        id="P10",
        title="Full real tree --check",
        passed=ok,
        expected_gates=["ALL"],
        expected_exit=EXIT_PASS,
        actual_exit=proc.returncode,
        matched_finding=(proc.stdout + proc.stderr).strip().splitlines()[-1]
        if (proc.stdout or proc.stderr)
        else "NONE",
        notes="real-tree read-only",
    )


EXTRA_NEGATIVE: list[tuple[str, Callable[[Path], ScenarioResult]]] = [
    ("N12", scenario_n12_box_dyn),
    ("N13", scenario_n13_pin_box_future),
    ("N14", scenario_n14_body_collect),
    ("N15", scenario_n15_baseline_count_exceeded),
    ("N16", scenario_n16_temporary_no_review),
    ("N17", scenario_n17_unknown_dbex),
    ("N18", scenario_n18_baseline_missing_exception),
    ("N19", scenario_n19_third_socket2),
    ("N20", scenario_n20_generic_dup_warn),
    ("N21", scenario_n21_known_debt_exact),
    ("N22", scenario_n22_comments_strings_ignored),
    ("N23", scenario_n23_cargo_alias),
    ("N24", scenario_n24_external_path),
    ("N25", scenario_n25_invalid_registry),
]

POSITIVE: list[tuple[str, Callable[[Path], ScenarioResult]]] = [
    ("P01", scenario_p01_bytes),
    ("P02", scenario_p02_http),
    ("P03", scenario_p03_dbex003_exact),
    ("P04", scenario_p04_hyper_owner),
    ("P05", scenario_p05_anyhow_cli),
    ("P06", scenario_p06_socket2_exact),
    ("P07", scenario_p07_hotpath_baselined),
    ("P08", scenario_p08_wasmtime_host),
    ("P09", scenario_p09_pub_crate),
    ("P10", scenario_p10_real_tree),
]
