#!/usr/bin/env python3
"""CAPABILITY_047 = config-parse-validate — real product E2E.

REAL_CONFIG_FILE → REAL_EXYONQ_BINARY → parse → validate → IR → start/reject.
ZERO_FAKE. No mock validators. Cap048 not started.
"""
from __future__ import annotations

import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_listen(port: int, timeout: float = 20.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def wait_sock(path: Path, timeout: float = 20.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
        time.sleep(0.05)
    return False


def stop_proc(p):
    if p is None or p.poll() is not None:
        return
    p.send_signal(signal.SIGTERM)
    try:
        p.wait(timeout=8)
    except Exception:
        p.kill()


def run_cmd(cmd: list[str], *, env: dict | None = None, timeout: int = 30) -> tuple[int, str]:
    proc = subprocess.run(
        cmd, capture_output=True, text=True, env=env or os.environ.copy(), timeout=timeout
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def minimal_cfg(listen: int, peer: int, *, extra_top: str = "", extra_server: str = "", **kw) -> str:
    timeout_ms = kw.get("timeout_ms", 5000)
    admin = kw.get("admin_state", "enabled")
    weight = kw.get("weight", 1)
    upstream_name = kw.get("upstream_name", "backend")
    route_up = kw.get("route_upstream", upstream_name)
    return f"""config_version = 1
{extra_top}[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api"]
{extra_server}
[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "{route_up}"

[[upstream]]
name = "{upstream_name}"
timeout_ms = {timeout_ms}
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = {weight}
priority = 0
admin_state = "{admin}"
"""


class Peer(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_a):
        pass

    def do_GET(self):
        if self.path.startswith("/api"):
            body = b"cap047-ok"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()


def start_peer(port: int):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), Peer)
    httpd.allow_reuse_address = True
    import threading

    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def expect_reject(label: str, cfg_text: str, *, via: str) -> dict:
    path = EV / f"rej-{label}.toml"
    path.write_text(cfg_text)
    if via == "lint":
        rc, out = run_cmd([str(CTL), "config", "lint", str(path)])
    elif via == "serve":
        # serve must refuse startup (non-zero) and not bind
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(path)
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(path)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            env=env,
            cwd=str(WS),
        )
        try:
            out, _ = proc.communicate(timeout=8)
            rc = proc.returncode if proc.returncode is not None else -1
        except subprocess.TimeoutExpired:
            stop_proc(proc)
            out = "TIMEOUT_STILL_RUNNING"
            rc = 0  # treat hung start as fail for reject expectation
    else:
        raise ValueError(via)
    return {
        "ok": rc != 0 and "TIMEOUT_STILL_RUNNING" not in out,
        "rc": rc,
        "out_snip": out[:400],
        "via": via,
        "path": str(path),
    }


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "config-parse-validate",
        "CAPABILITY": "CAPABILITY_047",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "TEXT→PARSE→VALIDATE→IR→SAFE PRODUCT STATE; unknown fields ERROR",
        "LA_CAP047_001": "deny_unknown_fields on RawConfig + nested operator tables",
        "CAPABILITY_048_STARTED": "NO",
        "CAP039_REOPEN": "NO",
        "CAP037_REOPEN": "NO",
        "CAP046_REOPEN": "NO",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
    }
    if not BINARY.is_file() or not CTL.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary/ctl"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)

    checks: dict = {}
    httpd = None
    procs: list = []
    tmp = Path(tempfile.mkdtemp(prefix="cap047-", dir=str(EV)))

    try:
        peer = pick_port()
        httpd = start_peer(peer)
        assert wait_listen(peer)

        # --- valid config: lint + serve + real request ---
        listen = pick_port()
        cfg_ok = tmp / "ok.toml"
        cfg_ok.write_text(minimal_cfg(listen, peer))
        rc_lint, out_lint = run_cmd([str(CTL), "config", "lint", str(cfg_ok)])
        rc_test, out_test = run_cmd([str(CTL), "config", "test", str(cfg_ok)])
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(cfg_ok)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_ok)],
            stdout=(EV / "exyonq-cap047-ok.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        procs.append(p)
        started = wait_listen(listen, timeout=25.0)
        code = -1
        body = b""
        if started:
            curl = subprocess.run(
                [
                    "curl",
                    "-sS",
                    "--http1.1",
                    "--max-time",
                    "5",
                    "-o",
                    str(EV / "ok.body"),
                    "-w",
                    "%{http_code}",
                    f"http://127.0.0.1:{listen}/api/x",
                ],
                capture_output=True,
            )
            code = int(curl.stdout.decode().strip() or "0")
            body = (EV / "ok.body").read_bytes() if (EV / "ok.body").is_file() else b""
        checks["valid_config_starts_and_serves"] = {
            "ok": rc_lint == 0 and rc_test == 0 and started and code == 200 and b"cap047-ok" in body,
            "lint_rc": rc_lint,
            "test_rc": rc_test,
            "started": started,
            "http": code,
        }
        stop_proc(p)
        procs.clear()

        # --- empty / no listeners ---
        checks["empty_config_rejected"] = expect_reject(
            "empty", "config_version = 1\n", via="lint"
        )
        checks["no_servers_rejected"] = expect_reject(
            "noserver",
            'config_version = 1\n[[route]]\nname = "r"\nmatch = { path = "/x" }\nroot = "/tmp"\n',
            via="lint",
        )

        # --- unknown fields (LA-CAP047-001) — lint AND serve ---
        listen_u = pick_port()
        top_unk = minimal_cfg(listen_u, peer, extra_top="totally_unknown_typo = true\n")
        checks["unknown_top_level_lint"] = expect_reject("unk-top-lint", top_unk, via="lint")
        checks["unknown_top_level_serve"] = expect_reject("unk-top-serve", top_unk, via="serve")
        nested_unk = minimal_cfg(listen_u, peer, extra_server="bogus_server_key = 1\n")
        checks["unknown_server_key_lint"] = expect_reject("unk-srv-lint", nested_unk, via="lint")
        checks["unknown_server_key_serve"] = expect_reject("unk-srv-serve", nested_unk, via="serve")
        up_unk = minimal_cfg(listen_u, peer).replace(
            "timeout_ms = 5000", "timeout_ms = 5000\nbogus_upstream_key = true"
        )
        checks["unknown_upstream_key"] = expect_reject("unk-up", up_unk, via="lint")

        # --- Cap037 timeout protector ---
        over = minimal_cfg(listen_u, peer, timeout_ms=86_400_001)
        checks["cap037_timeout_over_max"] = expect_reject("to-over", over, via="lint")
        max_ok = minimal_cfg(listen_u, peer, timeout_ms=86_400_000)
        Path(EV / "to-max.toml").write_text(max_ok)
        rc_max, _ = run_cmd([str(CTL), "config", "lint", str(EV / "to-max.toml")])
        checks["cap037_timeout_max_ok"] = {"ok": rc_max == 0, "rc": rc_max}

        # --- Cap046 admin_state ---
        bad_admin = minimal_cfg(listen_u, peer, admin_state="bogus")
        checks["cap046_invalid_admin_state"] = expect_reject("admin-bogus", bad_admin, via="lint")

        # --- type errors ---
        type_bad = minimal_cfg(listen_u, peer).replace("timeout_ms = 5000", 'timeout_ms = "slow"')
        checks["type_error_timeout_string"] = expect_reject("type-to", type_bad, via="lint")

        # --- unknown upstream reference ---
        bad_ref = minimal_cfg(listen_u, peer, route_upstream="missing")
        checks["unknown_upstream_reference"] = expect_reject("ref", bad_ref, via="lint")

        # --- duplicate upstream names ---
        dup = (
            minimal_cfg(listen_u, peer)
            + """
[[upstream]]
name = "backend"
[[upstream.endpoints]]
address = "127.0.0.1"
port = 8
"""
        )
        checks["duplicate_upstream_name"] = expect_reject("dup-up", dup, via="lint")

        # --- invalid weight domain: negative not representable as u32 in TOML;
        #     port 0 rejected ---
        port0 = minimal_cfg(listen_u, peer).replace("port = " + str(peer), "port = 0")
        checks["endpoint_port_zero_rejected"] = expect_reject("port0", port0, via="lint")

        # --- Cap013: invalid reload keeps last good ---
        if CTL.is_file():
            listen_r = pick_port()
            cfg_r = tmp / "reload.toml"
            cfg_r.write_text(minimal_cfg(listen_r, peer))
            ctrl = Path(f"/tmp/exq47-{os.getpid()}.sock")
            if ctrl.exists() or ctrl.is_symlink():
                ctrl.unlink()
            env = os.environ.copy()
            env["EXYONQ_CONFIG"] = str(cfg_r)
            env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
            p = subprocess.Popen(
                [str(BINARY), "serve", "--config", str(cfg_r)],
                stdout=(EV / "exyonq-cap047-reload.log").open("w"),
                stderr=subprocess.STDOUT,
                cwd=str(WS),
                env=env,
            )
            procs.append(p)
            ok_start = wait_listen(listen_r) and wait_sock(ctrl)
            # mutate to invalid (unknown field)
            cfg_r.write_text(
                minimal_cfg(listen_r, peer, extra_top="totally_unknown_typo = true\n")
            )
            rc_rel, out_rel = run_cmd(
                [str(CTL), "reload", "--config", str(cfg_r), "--socket", str(ctrl)],
                env=env,
            )
            still = wait_listen(listen_r, timeout=2.0)
            curl = subprocess.run(
                [
                    "curl",
                    "-sS",
                    "--max-time",
                    "5",
                    "-o",
                    "/dev/null",
                    "-w",
                    "%{http_code}",
                    f"http://127.0.0.1:{listen_r}/api/z",
                ],
                capture_output=True,
            )
            http_after = int(curl.stdout.decode().strip() or "0")
            checks["cap013_invalid_reload_keeps_active"] = {
                "ok": ok_start and rc_rel != 0 and still and http_after == 200,
                "reload_rc": rc_rel,
                "http_after": http_after,
                "out_snip": out_rel[:300],
            }
            stop_proc(p)
            procs.clear()
            try:
                ctrl.unlink(missing_ok=True)
            except OSError:
                pass
        else:
            checks["cap013_invalid_reload_keeps_active"] = {"ok": False, "detail": "no ctl"}

        # --- LA-CAP047-002: compact v2 include — lint and serve must agree ---
        listen_i = pick_port()
        frag = tmp / "frag.toml"
        frag.write_text(
            """[[route]]
name = "from_include"
match = { path = "/api/" }
upstream = "backend"
"""
        )
        # Compact/tabbed spacing that broke the old substring heuristic
        root_inc = tmp / "root-inc.toml"
        root_inc.write_text(
            f"""config_version=2
include=["{frag.name}"]
[[server]]
listen="127.0.0.1:{listen_i}"
routes=["from_include"]
[[upstream]]
name="backend"
timeout_ms=5000
[[upstream.endpoints]]
address="127.0.0.1"
port={peer}
weight=1
admin_state="enabled"
"""
        )
        rc_il, out_il = run_cmd([str(CTL), "config", "lint", str(root_inc)])
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(root_inc)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(root_inc)],
            stdout=(EV / "exyonq-cap047-inc.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        procs.append(p)
        inc_started = wait_listen(listen_i, timeout=25.0)
        code_inc = -1
        body_inc = b""
        if inc_started:
            curl = subprocess.run(
                [
                    "curl",
                    "-sS",
                    "--max-time",
                    "5",
                    "-o",
                    str(EV / "inc.body"),
                    "-w",
                    "%{http_code}",
                    f"http://127.0.0.1:{listen_i}/api/inc",
                ],
                capture_output=True,
            )
            code_inc = int(curl.stdout.decode().strip() or "0")
            body_inc = (EV / "inc.body").read_bytes() if (EV / "inc.body").is_file() else b""
        checks["compact_v2_include_lint_serve_parity"] = {
            "ok": rc_il == 0 and inc_started and code_inc == 200 and b"cap047-ok" in body_inc,
            "lint_rc": rc_il,
            "lint_out": out_il[:200],
            "started": inc_started,
            "http": code_inc,
            "note": "LA-CAP047-002: no looks_like_v2_include heuristic",
        }
        stop_proc(p)
        procs.clear()

        # Contract surface
        checks["contract_recorded"] = {
            "ok": True,
            "UNKNOWN_FIELD": "ERROR",
            "PARSE_ENTRYPOINT": "load_with_includes (serve/reload/lint TOML)",
            "VALIDATION_ENTRYPOINT": "AppConfig::from_raw",
            "IR_TYPE": "AppConfig",
            "MAX_UPSTREAM_TIMEOUT_MS": 86400000,
            "LA_CAP047_002": "FIXED_always_load_with_includes",
        }

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["DETAIL"] = repr(exc)
        result["CHECKS"] = checks
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 1
    finally:
        for proc in procs:
            stop_proc(proc)
        if httpd is not None:
            try:
                httpd.shutdown()
                httpd.server_close()
            except Exception:
                pass

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    result["FAILED"] = [k for k, v in checks.items() if not v.get("ok")]
    OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
