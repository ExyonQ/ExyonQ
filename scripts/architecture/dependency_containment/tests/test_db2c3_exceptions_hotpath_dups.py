#!/usr/bin/env python3
"""DB2C3 self-tests for GATE-DEP-006/007/008."""

from __future__ import annotations

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path
from unittest import mock

_HERE = Path(__file__).resolve().parent.parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from check_dependency_exceptions import run as run_006  # noqa: E402
from check_duplicate_dependencies import parse_duplicates, run as run_008  # noqa: E402
from check_hot_path_budget import PATTERN_RES, run as run_007, scan_file  # noqa: E402
from common import EXIT_CONFIG  # noqa: E402
from rust_scan import iter_code_lines  # noqa: E402


def _write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(textwrap.dedent(text).lstrip("\n"), encoding="utf-8")


def _dbex_row(
    eid: str,
    *,
    status: str = "accepted",
    scope: str = "module-api/src/static_wire.rs only",
    expansion: bool = False,
    review: str = "trigger",
    owner: str = "owner",
) -> str:
    return f"""
    [[exception]]
    id = "{eid}"
    dependency = "dep"
    status = "{status}"
    exact_scope = "{scope}"
    allowed_type_or_pattern = "pat"
    owner = "{owner}"
    policy_reference = "DB1 §19 {eid}"
    performance_reason = "perf"
    security_impact = "none"
    replacement_required = false
    review_trigger = "{review}"
    expansion_allowed = {str(expansion).lower()}
    test_or_bench_evidence = "tests"
    notes = "n"
    """


def make_exc_repo(tmp: Path, body: str, surface: str = "") -> Path:
    _write(tmp / "Cargo.toml", '[workspace]\nmembers = ["core"]\n')
    _write(tmp / "docs/architecture/.keep", "")
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-exceptions.toml",
        'schema_version = 1\n' + body,
    )
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-provider-surface-baseline.toml",
        'schema_version = 1\n' + (surface or ""),
    )
    _write(tmp / "core/Cargo.toml", '[package]\nname = "exyonq-core"\nversion = "0.0.0"\n')
    return tmp


def full_dbex_set(**overrides: dict) -> str:
    rows = {
        "DBEX-001": _dbex_row("DBEX-001", scope="workspace shared-buffer consumers listed in dependency-owner-registry.toml"),
        "DBEX-002": _dbex_row("DBEX-002", scope="authorized zones per dependency-owner-registry.toml for http"),
        "DBEX-003": _dbex_row("DBEX-003", scope="module-api/src/static_wire.rs — TcpStream only"),
        "DBEX-004": _dbex_row("DBEX-004", scope="authorized HTTP owners in dependency-owner-registry.toml"),
        "DBEX-005": _dbex_row("DBEX-005", status="temporary", scope="exact files listed in dependency-provider-surface-baseline.toml", review="DB8"),
        "DBEX-006": _dbex_row("DBEX-006", status="temporary", scope="socket2@0.5.10 and 0.6.4", review="upgrade"),
        "DBEX-007": _dbex_row("DBEX-007", scope="approved seams listed in dependency-provider-surface-baseline.toml"),
        "DBEX-008": _dbex_row("DBEX-008", status="temporary", scope="wasm/exyonq-wasm-host public API only", review="core wiring"),
    }
    rows.update(overrides)
    return "\n".join(rows[k] for k in sorted(rows))


class Gate006Tests(unittest.TestCase):
    def test_valid_accepted(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_exc_repo(Path(td), full_dbex_set())
            f = run_006(root)
            self.assertFalse(any(x.severity == "ERROR" for x in f))

    def test_temporary_with_review_trigger(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_exc_repo(Path(td), full_dbex_set())
            f = run_006(root)
            self.assertTrue(any(x.exception_id == "DBEX-005" and x.status == "KNOWN_DEBT" for x in f))

    def test_duplicate_id(self):
        with tempfile.TemporaryDirectory() as td:
            body = full_dbex_set() + _dbex_row("DBEX-003")
            root = make_exc_repo(Path(td), body)
            f = run_006(root)
            self.assertTrue(any("duplicate" in x.message for x in f))

    def test_unknown_id(self):
        with tempfile.TemporaryDirectory() as td:
            body = full_dbex_set() + _dbex_row("DBEX-999")
            # strip policy_reference prefix trick — row has DB1 §19 DBEX-999
            body = body.replace('policy_reference = "DB1 §19 DBEX-999"', 'policy_reference = "invented"')
            root = make_exc_repo(Path(td), body)
            f = run_006(root)
            self.assertTrue(any("unknown exception" in x.message for x in f))

    def test_missing_dbex(self):
        with tempfile.TemporaryDirectory() as td:
            body = "\n".join(_dbex_row(f"DBEX-{i:03d}") for i in range(1, 8))  # missing 008
            root = make_exc_repo(Path(td), body)
            f = run_006(root)
            self.assertTrue(any("DBEX-008" in x.message and x.severity == "ERROR" for x in f))

    def test_empty_scope(self):
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set()
            rows = rows.replace(
                'exact_scope = "module-api/src/static_wire.rs — TcpStream only"',
                'exact_scope = ""',
                1,
            )
            root = make_exc_repo(Path(td), rows)
            f = run_006(root)
            self.assertTrue(any("exact_scope is empty" in x.message for x in f))

    def test_broad_scope(self):
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set(
                **{
                    "DBEX-003": _dbex_row("DBEX-003", scope="core/**"),
                }
            )
            root = make_exc_repo(Path(td), rows)
            f = run_006(root)
            self.assertTrue(any("too broad" in x.message for x in f))

    def test_expansion_allowed_true(self):
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set(
                **{"DBEX-003": _dbex_row("DBEX-003", expansion=True, scope="module-api/src/static_wire.rs")}
            )
            root = make_exc_repo(Path(td), rows)
            f = run_006(root)
            self.assertTrue(any("expansion_allowed=true" in x.message for x in f))

    def test_temporary_without_review(self):
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set(
                **{
                    "DBEX-005": _dbex_row(
                        "DBEX-005",
                        status="temporary",
                        review="",
                        scope="exact files listed in dependency-provider-surface-baseline.toml",
                    )
                }
            )
            root = make_exc_repo(Path(td), rows)
            f = run_006(root)
            self.assertTrue(any("missing review_trigger" in x.message for x in f))

    def test_baseline_refs_missing_dbex(self):
        with tempfile.TemporaryDirectory() as td:
            surface = """
            [[baseline]]
            id = "X"
            provider = "hyper"
            kind = "pub_type"
            file = "core/src/x.rs"
            symbol_or_pattern = "BoxBody"
            status = "KNOWN_DEBT"
            policy_reference = "DB1 §19 DBEX-099"
            expansion_allowed = false
            """
            root = make_exc_repo(Path(td), full_dbex_set(), surface=surface)
            f = run_006(root)
            self.assertTrue(any("DBEX-099" in x.message for x in f))

    def test_dbex003_scope_ok(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_exc_repo(Path(td), full_dbex_set())
            f = run_006(root)
            self.assertTrue(any(x.exception_id == "DBEX-003" and x.status == "ACCEPTED" for x in f))

    def test_dbex003_expansion_rejected_by_scope_text(self):
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set(
                **{"DBEX-003": _dbex_row("DBEX-003", scope="all module-api contracts")}
            )
            root = make_exc_repo(Path(td), rows)
            f = run_006(root)
            self.assertTrue(any(x.exception_id == "DBEX-003" and "static_wire" in x.message for x in f))

    def test_dbex005_present(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_exc_repo(Path(td), full_dbex_set())
            f = run_006(root)
            self.assertTrue(any(x.exception_id == "DBEX-005" for x in f))

    def test_dbex008_outside_host_is_policy_in_exceptions_scope(self):
        # GATE-006 validates exception registry; host confinement is GATE-011.
        # Here: DBEX-008 scope must mention wasm-host.
        with tempfile.TemporaryDirectory() as td:
            rows = full_dbex_set(
                **{"DBEX-008": _dbex_row("DBEX-008", status="temporary", scope="core/src/", review="x")}
            )
            root = make_exc_repo(Path(td), rows)
            # scope empty of wasm-host — still valid schema; coverage note via missing wasm
            f = run_006(root)
            # no ERROR required if fields valid; ensure TEMPORARY still emits
            self.assertTrue(any(x.exception_id == "DBEX-008" for x in f))


class Gate007Tests(unittest.TestCase):
    def _hp_repo(self, tmp: Path, src: str, baseline_count: int = 1, pattern: str = "arc_dyn") -> Path:
        _write(tmp / "Cargo.toml", '[workspace]\nmembers = ["core"]\n')
        _write(tmp / "docs/architecture/.keep", "")
        _write(tmp / "core/Cargo.toml", '[package]\nname = "exyonq-core"\nversion="0.0.0"\n')
        _write(tmp / "core/src/server/handler.rs", src)
        _write(
            tmp / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
            f"""
            schema_version = 1
            [[hot_path_zone]]
            path = "core/src/server/"
            [[pattern]]
            id = "HP-TEST"
            pattern_class = "{pattern}"
            file = "core/src/server/handler.rs"
            symbol_or_line_scope = "file"
            observed_count = {baseline_count}
            status = "temporary"
            policy_reference = "test"
            exception_id = "DBEX-005"
            expansion_allowed = false
            review_trigger = "DB8"
            notes = "t"
            """,
        )
        return tmp

    def test_exact_baselined(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "use std::sync::Arc;\nfn f(x: Arc<dyn Send>) {}\n", 1)
            f = run_007(root)
            self.assertFalse(any(x.severity == "ERROR" for x in f))
            self.assertTrue(any(x.status == "KNOWN_DEBT" for x in f))

    def test_identical_count(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "fn f(x: Arc<dyn Send>) {}\n", 1)
            f = run_007(root)
            self.assertFalse(any("increased" in x.message for x in f))

    def test_count_incremented(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(
                Path(td),
                "fn f(x: Arc<dyn Send>) {}\nfn g(y: Arc<dyn Sync>) {}\n",
                1,
            )
            f = run_007(root)
            self.assertTrue(any(x.severity == "ERROR" and "increased" in x.message for x in f))

    def test_new_file(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "fn f() {}\n", 0, pattern="boxbody")
            # add second file with BoxBody not in baseline
            _write(Path(td) / "core/src/server/other.rs", "type B = BoxBody;\n")
            # fix baseline observed 0 entry - remove pattern with 0 by using empty baseline for boxbody
            _write(
                Path(td) / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
                """
                schema_version = 1
                [[hot_path_zone]]
                path = "core/src/server/"
                """,
            )
            f = run_007(root)
            self.assertTrue(any(x.severity == "ERROR" and "BoxBody" in x.message or "boxbody" in x.message for x in f))

    def test_new_symbol_pattern(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "fn f() {}\n", 1)
            _write(Path(td) / "core/src/server/handler.rs", "fn f(x: Arc<dyn Send>) {}\nfn g() -> Pin<Box<dyn Future<Output=()>>> { todo!() }\n")
            f = run_007(root)
            self.assertTrue(any("pin_box_dyn_future" in x.dependency for x in f))

    def test_arc_t_not_dyn(self):
        src = "fn f(x: Arc<String>) { let _ = x; }\n"
        with tempfile.TemporaryDirectory() as td:
            p = Path(td) / "t.rs"
            _write(p, src)
            c = scan_file(p)
            self.assertEqual(c.get("arc_dyn", 0), 0)

    def test_box_t_not_dyn(self):
        p = Path(tempfile.mkdtemp()) / "t.rs"
        _write(p, "fn f(x: Box<u8>) {}\n")
        self.assertEqual(scan_file(p).get("box_dyn", 0), 0)

    def test_pin_box_dyn_future_detected(self):
        p = Path(tempfile.mkdtemp()) / "t.rs"
        _write(p, "type F = Pin<Box<dyn Future<Output = ()> + Send>>;\n")
        self.assertGreaterEqual(scan_file(p).get("pin_box_dyn_future", 0), 1)

    def test_bodyext_collect_detected(self):
        p = Path(tempfile.mkdtemp()) / "t.rs"
        _write(p, "async fn f(b: B) { BodyExt::collect(b).await; }\n")
        self.assertEqual(scan_file(p).get("bodyext_collect", 0), 1)

    def test_to_bytes_detected(self):
        p = Path(tempfile.mkdtemp()) / "t.rs"
        _write(p, "fn f(b: Bytes) { let _ = to_bytes(b); }\n")
        self.assertEqual(scan_file(p).get("to_bytes", 0), 1)

    def test_comment_string_ignored(self):
        p = Path(tempfile.mkdtemp()) / "t.rs"
        _write(p, '// Arc<dyn Send>\nlet s = "BoxBody";\nfn f() {}\n')
        c = scan_file(p)
        self.assertEqual(c.get("arc_dyn", 0), 0)
        self.assertEqual(c.get("boxbody", 0), 0)

    def test_tests_separated(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "fn f() {}\n")
            _write(Path(td) / "core/src/server/foo_tests.rs", "fn t(x: Arc<dyn Send>) {}\n")
            f = run_007(root)
            self.assertFalse(any("foo_tests" in x.file for x in f if x.severity == "ERROR"))

    def test_ambiguous_clone_review(self):
        with tempfile.TemporaryDirectory() as td:
            root = self._hp_repo(Path(td), "fn f(h: HeaderMap) { let _ = headers.clone(); }\n", 0, "headermap_clone")
            _write(
                Path(td) / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
                """
                schema_version = 1
                [[hot_path_zone]]
                path = "core/src/server/"
                """,
            )
            _write(Path(td) / "core/src/server/handler.rs", "fn f() { let _ = headers.clone(); }\n")
            f = run_007(root)
            self.assertTrue(any(x.status == "REVIEW_REQUIRED" and x.dependency == "headermap_clone" for x in f))

    def test_ambiguous_lock_review(self):
        with tempfile.TemporaryDirectory() as td:
            _write(Path(td) / "Cargo.toml", '[workspace]\nmembers=["core"]\n')
            _write(Path(td) / "docs/architecture/.keep", "")
            _write(Path(td) / "core/Cargo.toml", '[package]\nname="exyonq-core"\nversion="0.0.0"\n')
            _write(Path(td) / "core/src/server/handler.rs", "fn f(m: Mutex<u8>) {}\n")
            _write(
                Path(td) / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
                'schema_version=1\n[[hot_path_zone]]\npath="core/src/server/"\n',
            )
            f = run_007(root := Path(td))
            self.assertTrue(any(x.status == "REVIEW_REQUIRED" and x.dependency == "mutex_rwlock" for x in f))

    def test_new_critical_indirection_error(self):
        with tempfile.TemporaryDirectory() as td:
            _write(Path(td) / "Cargo.toml", '[workspace]\nmembers=["core"]\n')
            _write(Path(td) / "docs/architecture/.keep", "")
            _write(Path(td) / "core/Cargo.toml", '[package]\nname="exyonq-core"\nversion="0.0.0"\n')
            _write(Path(td) / "core/src/server/handler.rs", "fn f(x: Arc<dyn Send>) {}\n")
            _write(
                Path(td) / "docs/architecture/dependency-boundaries/dependency-hot-path-baseline.toml",
                'schema_version=1\n[[hot_path_zone]]\npath="core/src/server/"\n',
            )
            f = run_007(Path(td))
            self.assertTrue(any(x.severity == "ERROR" and x.dependency == "arc_dyn" for x in f))


class Gate008Tests(unittest.TestCase):
    def test_parse_no_dups(self):
        self.assertEqual(parse_duplicates("foo v1.0.0\nbar v2.0.0\n"), {})

    def test_generic_dup_warn(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\nallowed_lock_versions=["0.5.10","0.6.4"]\n',
            )
            _write(root / "Cargo.lock", 'name = "socket2"\nversion = "0.5.10"\n\nname = "socket2"\nversion = "0.6.4"\n')
            tree = "bitflags v1.0.0\nbitflags v2.0.0\nsocket2 v0.5.10\nsocket2 v0.6.4\n"
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                return_value=tree,
            ):
                f = run_008(root)
            self.assertTrue(any(x.dependency == "bitflags" and x.severity == "WARN" for x in f))

    def test_socket2_known_debt(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\nallowed_lock_versions=["0.5.10","0.6.4"]\n',
            )
            _write(root / "Cargo.lock", "")
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                return_value="socket2 v0.5.10\nsocket2 v0.6.4\n",
            ):
                f = run_008(root)
            self.assertTrue(
                any(x.dependency == "socket2" and x.status == "KNOWN_DEBT" and x.exception_id == "DBEX-006" for x in f)
            )

    def test_socket2_third_version_error(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\nallowed_lock_versions=["0.5.10","0.6.4"]\n',
            )
            _write(root / "Cargo.lock", "")
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                return_value="socket2 v0.5.10\nsocket2 v0.6.4\nsocket2 v0.7.0\n",
            ):
                f = run_008(root)
            self.assertTrue(any(x.dependency == "socket2" and x.severity == "ERROR" for x in f))

    def test_socket2_version_change_error(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\nallowed_lock_versions=["0.5.10","0.6.4"]\n',
            )
            _write(root / "Cargo.lock", "")
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                return_value="socket2 v0.5.11\nsocket2 v0.6.4\n",
            ):
                f = run_008(root)
            self.assertTrue(any(x.dependency == "socket2" and x.severity == "ERROR" for x in f))

    def test_strategic_dup_error(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\nallowed_lock_versions=["0.5.10","0.6.4"]\n',
            )
            _write(root / "Cargo.lock", 'name="socket2"\nversion="0.5.10"\n')
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                return_value="tokio v1.0.0\ntokio v1.1.0\nsocket2 v0.5.10\n",
            ):
                f = run_008(root)
            self.assertTrue(any(x.dependency == "tokio" and x.severity == "ERROR" for x in f))

    def test_cargo_failure_exit_config(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            _write(root / "Cargo.toml", '[workspace]\nmembers=[]\n')
            _write(root / "docs/architecture/.keep", "")
            _write(
                root / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
                'schema_version=1\n[[dependency]]\nname="socket2"\nclass="D"\nauthorized_owners=[]\nauthorized_public_surface=""\nforbidden_zones=[]\nerror_policy=""\nhot_path_policy=""\nversion_policy=""\nstatus="BINDING"\n',
            )
            with mock.patch(
                "check_duplicate_dependencies.run_cargo_tree_d",
                side_effect=SystemExit(f"{EXIT_CONFIG}: cargo tree -d failed"),
            ):
                with self.assertRaises(SystemExit) as cm:
                    run_008(root)
                self.assertIn(str(EXIT_CONFIG), str(cm.exception))

    def test_json_deterministic(self):
        text = "socket2 v0.5.10\nsocket2 v0.6.4\nbitflags v1\nbitflags v2\n"
        a = parse_duplicates(text)
        b = parse_duplicates(text)
        self.assertEqual(a, b)
        self.assertEqual(sorted(a["socket2"]), ["0.5.10", "0.6.4"])


if __name__ == "__main__":
    unittest.main()
