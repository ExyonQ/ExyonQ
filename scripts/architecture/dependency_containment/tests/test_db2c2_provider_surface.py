#!/usr/bin/env python3
"""DB2C2 self-tests for provider-surface checkers (stdlib unittest)."""

from __future__ import annotations

import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

_HERE = Path(__file__).resolve().parent.parent
if str(_HERE) not in sys.path:
    sys.path.insert(0, str(_HERE))

from check_critical_provider_owners import run as run_011  # noqa: E402
from check_provider_errors import run as run_005  # noqa: E402
from check_public_provider_surface import run as run_003_004  # noqa: E402
from check_source_import_zones import run as run_002  # noqa: E402
from provider_common import scan_provider_hits  # noqa: E402
from rust_scan import iter_code_lines  # noqa: E402


def _write(p: Path, text: str) -> None:
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(textwrap.dedent(text).lstrip("\n"), encoding="utf-8")


def make_repo(tmp: Path) -> Path:
    _write(
        tmp / "Cargo.toml",
        """
        [workspace]
        members = ["core", "module-api", "mod-tls", "wasm-host", "config-ir", "cli", "config/merge"]
        [workspace.dependencies]
        bytes = "1"
        http = "1"
        hyper = "1"
        tokio = "1"
        rustls = "0.23"
        wasmtime = "40"
        anyhow = "1"
        """,
    )
    _write(tmp / "docs/architecture/.keep", "")
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-owner-registry.toml",
        """
        schema_version = 1
        [[dependency]]
        name = "bytes"
        class = "A"
        authorized_owners = ["exyonq-core", "exyonq-module-api"]
        authorized_public_surface = "Bytes"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "ok"
        version_policy = "workspace"
        status = "BINDING"
        [[dependency]]
        name = "http"
        class = "A"
        authorized_owners = ["exyonq-core", "exyonq-module-api"]
        authorized_public_surface = "selected"
        forbidden_zones = []
        error_policy = "n/a"
        hot_path_policy = "ok"
        version_policy = "workspace"
        status = "BINDING"
        [[dependency]]
        name = "hyper"
        class = "C"
        authorized_owners = ["exyonq-core", "exyonq-config-ir"]
        authorized_public_surface = "owners"
        forbidden_zones = ["exyonq-module-api"]
        error_policy = "review"
        hot_path_policy = "owner"
        version_policy = "workspace"
        status = "BINDING"
        known_debt_owners = ["exyonq-config-ir"]
        [[dependency]]
        name = "tokio"
        class = "B"
        authorized_owners = ["exyonq-core", "exyonq-module-api"]
        authorized_public_surface = "restricted"
        forbidden_zones = []
        error_policy = "map"
        hot_path_policy = "concrete"
        version_policy = "workspace"
        status = "BINDING"
        [[dependency]]
        name = "rustls"
        class = "B"
        authorized_owners = ["exyonq-mod-tls", "exyonq-core"]
        authorized_public_surface = "owner"
        forbidden_zones = ["exyonq-module-api"]
        error_policy = "map"
        hot_path_policy = "tls"
        version_policy = "workspace"
        status = "BINDING"
        [[dependency]]
        name = "wasmtime"
        class = "E"
        authorized_owners = ["exyonq-wasm-host"]
        authorized_public_surface = "leaf"
        forbidden_zones = []
        error_policy = "leaf"
        hot_path_policy = "none"
        version_policy = "exact"
        status = "BINDING"
        [[dependency]]
        name = "anyhow"
        class = "F"
        authorized_owners = ["exyonq", "exyonq-core"]
        authorized_public_surface = "zoned"
        forbidden_zones = []
        error_policy = "zoned"
        hot_path_policy = "control"
        version_policy = "workspace"
        status = "BINDING"
        """,
    )
    _write(
        tmp / "docs/architecture/dependency-boundaries/dependency-provider-surface-baseline.toml",
        """
        schema_version = 1
        [[baseline]]
        id = "KD-CORE-TLS-REEXPORT"
        provider = "rustls"
        kind = "reexport"
        file = "core/src/lib.rs"
        symbol_or_pattern = "pub use exyonq_mod_tls as tls"
        status = "KNOWN_DEBT"
        policy_reference = "test"
        expansion_allowed = false
        [[baseline]]
        id = "KD-HYPER-CONFIG-IR"
        provider = "hyper"
        kind = "import"
        file = "config-ir/src/lib.rs"
        symbol_or_pattern = "hyper::Uri"
        status = "KNOWN_DEBT"
        policy_reference = "test"
        expansion_allowed = false
        [[baseline]]
        id = "DBEX-003-STATIC-WIRE-TCPSTREAM"
        provider = "tokio"
        kind = "pub_type"
        file = "module-api/src/static_wire.rs"
        symbol_or_pattern = "tokio::net::TcpStream"
        status = "ACCEPTED"
        policy_reference = "DBEX-003"
        expansion_allowed = false
        [[baseline]]
        id = "DBEX-008-WASM-HOST-LEAF"
        provider = "wasmtime"
        kind = "import"
        file = "wasm-host/src/lib.rs"
        symbol_or_pattern = "wasmtime::"
        status = "ACCEPTED"
        policy_reference = "DBEX-008"
        expansion_allowed = false
        """,
    )
    # manifests
    for name, pkg in [
        ("core", "exyonq-core"),
        ("module-api", "exyonq-module-api"),
        ("mod-tls", "exyonq-mod-tls"),
        ("wasm-host", "exyonq-wasm-host"),
        ("config-ir", "exyonq-config-ir"),
        ("cli", "exyonq"),
        ("config/merge", "exyonq-config-merge"),
    ]:
        _write(tmp / name / "Cargo.toml", f'[package]\nname = "{pkg}"\nversion = "0.0.0"\n')
    return tmp


class LexTests(unittest.TestCase):
    def test_comments_and_strings_ignored(self):
        src = '''
        // use rustls::ServerConfig;
        /* use hyper::Error; */
        let s = "tokio::net::TcpStream";
        let r = r#"wasmtime::Engine"#;
        use bytes::Bytes;
        '''
        hits = scan_provider_hits(src)
        paths = {h.path for h in hits}
        self.assertIn("bytes::Bytes", paths)
        self.assertNotIn("rustls::ServerConfig", paths)
        self.assertTrue(all("wasmtime" not in h.path for h in hits))
        self.assertTrue(all(not h.path.startswith("tokio") for h in hits))

    def test_raw_strings_ignored(self):
        src = 'const X: &str = r##"hyper::Uri and rustls::Error"##;\nuse http::Method;\n'
        hits = scan_provider_hits(src)
        self.assertEqual({h.provider for h in hits}, {"http"})

    def test_ambiguous_alias_not_invented(self):
        # local alias without provider path — scanner must not claim rustls
        src = "type Error = crate::LocalError;\nfn f() -> Error { todo!() }\n"
        hits = scan_provider_hits(src)
        self.assertEqual(hits, [])


class GateScenarioTests(unittest.TestCase):
    def test_authorized_import(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "core/src/lib.rs", "use bytes::Bytes;\npub fn f(b: Bytes) {}\n")
            f = run_002(root)
            self.assertFalse(any(x.severity == "ERROR" for x in f))

    def test_import_forbidden_owner(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "module-api/src/lib.rs", "use hyper::Uri;\n")
            f = run_002(root)
            self.assertTrue(any(x.severity == "ERROR" and x.dependency == "hyper" for x in f))

    def test_pub_use_rustls_fails(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "core/src/lib.rs", "pub use rustls::ServerConfig;\n")
            f = run_003_004(root)
            self.assertTrue(
                any(x.gate_id == "GATE-DEP-003" and x.severity == "ERROR" for x in f)
            )

    def test_pub_crate_use_rustls_ok(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "mod-tls/src/lib.rs", "pub(crate) use rustls::ServerConfig;\n")
            f = run_003_004(root)
            self.assertFalse(
                any(x.gate_id == "GATE-DEP-003" and x.severity == "ERROR" for x in f)
            )

    def test_provider_type_in_pub_fn(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/lib.rs",
                "pub fn bad(x: hyper::Uri) {}\n",
            )
            f = run_003_004(root)
            self.assertTrue(
                any(x.gate_id == "GATE-DEP-004" and x.severity == "ERROR" for x in f)
            )

    def test_provider_type_in_pub_struct(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/lib.rs",
                "pub struct S { pub u: hyper::Uri }\n",
            )
            f = run_003_004(root)
            self.assertTrue(
                any(x.gate_id == "GATE-DEP-004" and x.severity == "ERROR" for x in f)
            )

    def test_provider_error_in_pub_enum(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/lib.rs",
                "pub enum E { Hyper(hyper::Error) }\n",
            )
            f = run_005(root)
            self.assertTrue(any(x.severity == "ERROR" and x.dependency == "hyper" for x in f))

    def test_anyhow_allowed_in_cli(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "cli/src/main.rs", "fn main() -> anyhow::Result<()> { Ok(()) }\n")
            f = run_005(root)
            self.assertFalse(any(x.dependency == "anyhow" and x.severity == "ERROR" for x in f))

    def test_anyhow_forbidden_on_facade(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/lib.rs",
                "pub fn f() -> anyhow::Result<()> { Ok(()) }\n",
            )
            f = run_005(root)
            self.assertTrue(any(x.dependency == "anyhow" and x.severity == "ERROR" for x in f))

    def test_dbex003_exact_accepted(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/static_wire.rs",
                "pub fn serve(stream: tokio::net::TcpStream) {}\n",
            )
            _write(root / "module-api/src/lib.rs", "pub mod static_wire;\n")
            f = run_011(root)
            self.assertTrue(any(x.status == "ACCEPTED" and x.dependency == "tokio" for x in f))
            self.assertFalse(any(x.severity == "ERROR" and x.dependency == "tokio" for x in f))

    def test_dbex003_expansion_fails(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/other.rs",
                "pub fn serve(stream: tokio::net::TcpStream) {}\n",
            )
            f = run_011(root)
            self.assertTrue(any(x.severity == "ERROR" and x.dependency == "tokio" for x in f))

    def test_core_tls_baseline_exact(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "core/src/lib.rs", "pub use exyonq_mod_tls as tls;\n")
            f = run_003_004(root)
            self.assertTrue(
                any(x.baseline_id == "KD-CORE-TLS-REEXPORT" and x.status == "KNOWN_DEBT" for x in f)
            )

    def test_new_tls_reexport_fails(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "core/src/lib.rs", "pub use exyonq_mod_tls as tls;\n")
            _write(root / "core/src/more.rs", "pub use exyonq_mod_tls as tls2;\n")
            f = run_011(root)
            self.assertTrue(
                any(x.severity == "ERROR" and "additional TLS" in x.message for x in f)
            )

    def test_wasmtime_from_core_errors(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "core/src/lib.rs", "use wasmtime::Engine;\n")
            f = run_011(root)
            self.assertTrue(any(x.dependency == "wasmtime" and x.severity == "ERROR" for x in f))

    def test_hyper_config_ir_known_debt(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "config-ir/src/lib.rs", "fn f(u: hyper::Uri) {}\n")
            f = run_002(root)
            self.assertTrue(
                any(x.status == "KNOWN_DEBT" and x.baseline_id == "KD-HYPER-CONFIG-IR" for x in f)
            )
            self.assertFalse(any(x.severity == "ERROR" for x in f))

    def test_hyper_new_config_zone_errors(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(root / "config/merge/src/lib.rs", "fn f(u: hyper::Uri) {}\n")
            f = run_011(root)
            self.assertTrue(any(x.dependency == "hyper" and x.severity == "ERROR" for x in f))

    def test_http_bytes_accepted(self):
        with tempfile.TemporaryDirectory() as td:
            root = make_repo(Path(td))
            _write(
                root / "module-api/src/lib.rs",
                "use bytes::Bytes;\nuse http::Method;\npub fn f(b: Bytes, m: Method) {}\n",
            )
            f = run_003_004(root)
            self.assertFalse(
                any(x.severity == "ERROR" and x.dependency in {"bytes", "http"} for x in f)
            )


if __name__ == "__main__":
    unittest.main()
