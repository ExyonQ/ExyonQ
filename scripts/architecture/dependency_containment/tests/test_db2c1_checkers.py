#!/usr/bin/env python3
"""DB2C1 self-tests for dependency_containment checkers (stdlib unittest)."""

from __future__ import annotations

import shutil
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

# Allow importing sibling modules when run as a script.
_HERE = Path(__file__).resolve().parent.parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from common import (  # noqa: E402
    inventory_direct_externals,
    load_admissions,
    load_owner_registry,
)
from check_dependency_admissions import run as run_adm  # noqa: E402
from check_lock_and_manifest_policy import run as run_lock  # noqa: E402
from check_manifest_registry import run as run_reg  # noqa: E402
from check_manifest_zones import run as run_zones  # noqa: E402


def _write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(textwrap.dedent(text).lstrip("\n"), encoding="utf-8")


def make_min_repo(tmp: Path) -> Path:
    """Minimal fake workspace for isolated tests."""
    _write(
        tmp / "Cargo.toml",
        """
        [workspace]
        members = ["core", "tools"]
        [workspace.dependencies]
        bytes = "1"
        tokio = { version = "1", features = ["rt"] }
        exyonq-core = { path = "core" }
        """,
    )
    _write(tmp / "docs/architecture/.keep", "")
    _write(
        tmp / "core/Cargo.toml",
        """
        [package]
        name = "exyonq-core"
        version = "0.0.0"
        edition = "2021"
        [dependencies]
        bytes = { workspace = true }
        tokio = { workspace = true }
        [dev-dependencies]
        tempfile = "3"
        [target.'cfg(unix)'.dependencies]
        libc = "0.2"
        """,
    )
    _write(
        tmp / "tools/Cargo.toml",
        """
        [package]
        name = "tools"
        version = "0.0.0"
        edition = "2021"
        [dependencies]
        # alias: key != package
        renamed-http = { package = "http", version = "1" }
        exyonq-core = { workspace = true }
        """,
    )
    _write(
        tmp / "Cargo.lock",
        """
        # This file is automatically @generated — test fixture
        version = 3

        [[package]]
        name = "bytes"
        version = "1.0.0"

        [[package]]
        name = "http"
        version = "1.0.0"

        [[package]]
        name = "libc"
        version = "0.2.0"

        [[package]]
        name = "tempfile"
        version = "3.0.0"

        [[package]]
        name = "tokio"
        version = "1.0.0"
        """,
    )
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
        """
        schema_version = 1
        [[dependency]]
        name = "bytes"
        class = "A"
        authorized_owners = ["exyonq-core"]
        authorized_public_surface = "Bytes"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "no_clone"
        version_policy = "workspace"
        status = "BINDING"

        [[dependency]]
        name = "tokio"
        class = "B"
        authorized_owners = ["exyonq-core"]
        authorized_public_surface = "restricted"
        forbidden_zones = []
        error_policy = "map"
        hot_path_policy = "concrete"
        version_policy = "workspace"
        status = "BINDING"

        [[dependency]]
        name = "tempfile"
        class = "I"
        authorized_owners = ["exyonq-core"]
        authorized_public_surface = "dev"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "none"
        version_policy = "direct"
        status = "BINDING"

        [[dependency]]
        name = "libc"
        class = "D"
        authorized_owners = ["exyonq-core"]
        authorized_public_surface = "no_reexport"
        forbidden_zones = []
        error_policy = "io"
        hot_path_policy = "mechanism"
        version_policy = "target"
        status = "BINDING"

        [[dependency]]
        name = "http"
        class = "A"
        authorized_owners = ["tools"]
        authorized_public_surface = "value types"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "shared"
        version_policy = "workspace"
        status = "BINDING"
        """,
    )
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-admissions.toml",
        """
        schema_version = 1
        [[admission]]
        dependency = "bytes"
        problem = "buffers"
        category = "A"
        owner = "exyonq-core"
        authorized_zones = ["exyonq-core"]
        public_type_impact = "Bytes"
        error_model = "n/a"
        hot_path_impact = "no_clone"
        version_policy = "workspace"
        status = "accepted"

        [[admission]]
        dependency = "tokio"
        problem = "runtime"
        category = "B"
        owner = "exyonq-core"
        authorized_zones = ["exyonq-core"]
        public_type_impact = "restricted"
        error_model = "map"
        hot_path_impact = "concrete"
        version_policy = "workspace"
        status = "accepted"

        [[admission]]
        dependency = "tempfile"
        problem = "tests"
        category = "I"
        owner = "exyonq-core"
        authorized_zones = ["exyonq-core"]
        public_type_impact = "dev"
        error_model = "n/a"
        hot_path_impact = "none"
        version_policy = "direct"
        status = "build-test-only"

        [[admission]]
        dependency = "libc"
        problem = "os"
        category = "D"
        owner = "exyonq-core"
        authorized_zones = ["exyonq-core"]
        public_type_impact = "no_reexport"
        error_model = "io"
        hot_path_impact = "mechanism"
        version_policy = "target"
        status = "accepted"

        [[admission]]
        dependency = "http"
        problem = "http values"
        category = "A"
        owner = "tools"
        authorized_zones = ["tools"]
        public_type_impact = "value types"
        error_model = "n/a"
        hot_path_impact = "shared"
        version_policy = "direct"
        status = "accepted"
        """,
    )
    return tmp


class InventoryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        make_min_repo(self.tmp)

    def tearDown(self) -> None:
        shutil.rmtree(self.tmp)

    def test_normal_workspace_path_target_dev_alias(self) -> None:
        occ = inventory_direct_externals(self.tmp)
        pkgs = {o.package for o in occ}
        self.assertIn("bytes", pkgs)
        self.assertIn("tokio", pkgs)
        self.assertIn("tempfile", pkgs)
        self.assertIn("libc", pkgs)
        self.assertIn("http", pkgs)  # via package alias
        # internal path skipped
        self.assertNotIn("exyonq-core", pkgs)
        libc = [o for o in occ if o.package == "libc"][0]
        self.assertTrue(libc.target)
        self.assertEqual(libc.section, "dependencies")
        http = [o for o in occ if o.package == "http"][0]
        self.assertEqual(http.dep_key, "renamed-http")


class GateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        make_min_repo(self.tmp)

    def tearDown(self) -> None:
        shutil.rmtree(self.tmp)

    def test_pass_baseline(self) -> None:
        self.assertEqual(run_reg(self.tmp), [])
        self.assertEqual(run_zones(self.tmp), [])
        self.assertEqual(run_adm(self.tmp), [])
        # lock policy may be clean
        errs = [f for f in run_lock(self.tmp) if f.severity == "ERROR"]
        self.assertEqual(errs, [])

    def test_unregistered_dependency(self) -> None:
        # add serde without registry
        p = self.tmp / "core/Cargo.toml"
        text = p.read_text(encoding="utf-8")
        p.write_text(text + "\nserde = \"1\"\n", encoding="utf-8")
        _write(
            self.tmp / "Cargo.lock",
            self.tmp.joinpath("Cargo.lock").read_text(encoding="utf-8")
            + '\n[[package]]\nname = "serde"\nversion = "1.0.0"\n',
        )
        findings = run_reg(self.tmp)
        self.assertTrue(any(f.dependency == "serde" and f.severity == "ERROR" for f in findings))

    def test_unauthorized_owner(self) -> None:
        # put bytes on tools
        p = self.tmp / "tools/Cargo.toml"
        text = p.read_text(encoding="utf-8")
        p.write_text(text + "\nbytes = { workspace = true }\n", encoding="utf-8")
        findings = run_zones(self.tmp)
        self.assertTrue(any(f.dependency == "bytes" and "not in authorized_owners" in f.message for f in findings))

    def test_star_version(self) -> None:
        p = self.tmp / "tools/Cargo.toml"
        text = p.read_text(encoding="utf-8")
        p.write_text(text + '\nbad = "*"\n', encoding="utf-8")
        # register bad so 009 fires not only 001
        reg = self.tmp / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
        reg.write_text(
            reg.read_text(encoding="utf-8")
            + textwrap.dedent(
                """
                [[dependency]]
                name = "bad"
                class = "G"
                authorized_owners = ["tools"]
                authorized_public_surface = "x"
                forbidden_zones = []
                error_policy = "n/a"
                hot_path_policy = "none"
                version_policy = "direct"
                status = "BINDING"
                """
            ),
            encoding="utf-8",
        )
        findings = run_lock(self.tmp)
        self.assertTrue(any(f.dependency == "bad" and "*" in f.message for f in findings))

    def test_git_dependency(self) -> None:
        p = self.tmp / "tools/Cargo.toml"
        text = p.read_text(encoding="utf-8")
        p.write_text(
            text + '\ngitty = { git = "https://example.com/foo.git", package = "gitty" }\n',
            encoding="utf-8",
        )
        reg = self.tmp / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
        reg.write_text(
            reg.read_text(encoding="utf-8")
            + textwrap.dedent(
                """
                [[dependency]]
                name = "gitty"
                class = "G"
                authorized_owners = ["tools"]
                authorized_public_surface = "x"
                forbidden_zones = []
                error_policy = "n/a"
                hot_path_policy = "none"
                version_policy = "git"
                status = "BINDING"
                """
            ),
            encoding="utf-8",
        )
        findings = run_lock(self.tmp)
        self.assertTrue(any(f.dependency == "gitty" and "git dependency" in f.message for f in findings))

    def test_missing_admission(self) -> None:
        adm = self.tmp / "docs/architecture/dependency-boundaries/dependency-admissions.toml"
        # drop http admission
        lines = [ln for ln in adm.read_text(encoding="utf-8").splitlines() if "http" not in ln]
        # crude: rewrite without http block
        _write(
            adm,
            """
            schema_version = 1
            [[admission]]
            dependency = "bytes"
            problem = "buffers"
            category = "A"
            owner = "exyonq-core"
            authorized_zones = ["exyonq-core"]
            public_type_impact = "Bytes"
            error_model = "n/a"
            hot_path_impact = "no_clone"
            version_policy = "workspace"
            status = "accepted"
            [[admission]]
            dependency = "tokio"
            problem = "runtime"
            category = "B"
            owner = "exyonq-core"
            authorized_zones = ["exyonq-core"]
            public_type_impact = "restricted"
            error_model = "map"
            hot_path_impact = "concrete"
            version_policy = "workspace"
            status = "accepted"
            [[admission]]
            dependency = "tempfile"
            problem = "tests"
            category = "I"
            owner = "exyonq-core"
            authorized_zones = ["exyonq-core"]
            public_type_impact = "dev"
            error_model = "n/a"
            hot_path_impact = "none"
            version_policy = "direct"
            status = "build-test-only"
            [[admission]]
            dependency = "libc"
            problem = "os"
            category = "D"
            owner = "exyonq-core"
            authorized_zones = ["exyonq-core"]
            public_type_impact = "no_reexport"
            error_model = "io"
            hot_path_impact = "mechanism"
            version_policy = "target"
            status = "accepted"
            """,
        )
        findings = run_adm(self.tmp)
        self.assertTrue(any(f.dependency == "http" and "missing admission" in f.message for f in findings))

    def test_invalid_registry_duplicate(self) -> None:
        reg = self.tmp / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml"
        reg.write_text(
            reg.read_text(encoding="utf-8")
            + textwrap.dedent(
                """
                [[dependency]]
                name = "bytes"
                class = "A"
                authorized_owners = ["exyonq-core"]
                authorized_public_surface = "Bytes"
                forbidden_zones = []
                error_policy = "n/a"
                hot_path_policy = "no_clone"
                version_policy = "workspace"
                status = "BINDING"
                """
            ),
            encoding="utf-8",
        )
        with self.assertRaises(SystemExit):
            load_owner_registry(self.tmp)


if __name__ == "__main__":
    unittest.main()
