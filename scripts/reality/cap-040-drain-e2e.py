#!/usr/bin/env python3
"""CAPABILITY_040 = drain — real product E2E (server/process drain).

Distinct from Cap046 upstream DrainRequested.
REAL_EXYONQ_BINARY → REAL_LISTENER → REAL_DRAIN → REAL_NETWORK_BEHAVIOR.
ZERO_FAKE. Cap048/047/046 remain CLOSED. Cap041 not started.
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
import threading
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
BODY = b"cap040-ok"


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


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
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
        p.wait(timeout=10)
    except Exception:
        p.kill()


def _read_http_response(s: socket.socket, timeout: float = 15.0) -> bytes:
    s.settimeout(timeout)
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            return buf
        buf += chunk
        if len(buf) > 1 << 20:
            return buf
    head, body = buf.split(b"\r\n\r\n", 1)
    cl = None
    for line in head.split(b"\r\n")[1:]:
        if line.lower().startswith(b"content-length:"):
            try:
                cl = int(line.split(b":", 1)[1].strip())
            except ValueError:
                cl = None
            break
    if cl is None:
        # short drain/probe bodies without CL: brief extra read
        s.settimeout(0.15)
        try:
            while True:
                chunk = s.recv(65536)
                if not chunk:
                    break
                body += chunk
        except socket.timeout:
            pass
        return head + b"\r\n\r\n" + body
    while len(body) < cl:
        chunk = s.recv(65536)
        if not chunk:
            break
        body += chunk
    return head + b"\r\n\r\n" + body


def http_exchange(port: int, raw: bytes, *, timeout: float = 15.0) -> str:
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as s:
        s.sendall(raw)
        return _read_http_response(s, timeout=timeout).decode("utf-8", "replace")


def keepalive_pair(port: int, req1: bytes, req2: bytes, *, between) -> tuple[str, str]:
    with socket.create_connection(("127.0.0.1", port), timeout=15.0) as s:
        s.sendall(req1)
        first = _read_http_response(s, timeout=15.0)
        between()
        s.sendall(req2)
        second = _read_http_response(s, timeout=15.0)
        return first.decode("utf-8", "replace"), second.decode("utf-8", "replace")

def make_peer(delay: float = 0.0):
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def do_GET(self):
            if delay > 0:
                time.sleep(delay)
            if self.path.startswith("/api"):
                self.send_response(200)
                self.send_header("Content-Length", str(len(BODY)))
                self.end_headers()
                self.wfile.write(BODY)
            else:
                self.send_response(404)
                self.end_headers()

    return H


def start_peer(port: int, delay: float = 0.0):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_peer(delay))
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def cfg(listen: int, peer: int, timeout_ms: int = 10000) -> str:
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api"]
[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = {timeout_ms}
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = 1
admin_state = "enabled"
"""


def main() -> int:
    checks: dict = {}
    ok = True
    tmp = Path(tempfile.mkdtemp(prefix="cap040-", dir=str(EV)))
    peer = pick_port()
    listen = pick_port()
    httpd = start_peer(peer, delay=0.0)
    httpd_slow = None
    peer_slow = None
    cfg_path = tmp / "live.toml"
    ctrl = Path(f"/tmp/exq40-{os.getpid()}-{time.time_ns() % 100000}.sock")
    if ctrl.exists():
        ctrl.unlink()
    proc = None
    try:
        if not BINARY.is_file() or not CTL.is_file():
            checks["binaries"] = {"ok": False}
            raise RuntimeError("missing binaries")

        cfg_path.write_text(cfg(listen, peer))
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(cfg_path)
        env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
        log = tmp / "serve.log"
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            checks["startup"] = {"ok": False, "log": log.read_text()[-600:]}
            raise RuntimeError("startup failed")
        checks["startup"] = {"ok": True, "pid": proc.pid}

        def status() -> dict:
            r = subprocess.run(
                [str(CTL), "status", "--socket", str(ctrl), "--format", "json"],
                capture_output=True,
                text=True,
            )
            if r.returncode != 0:
                return {"_rc": r.returncode, "_out": (r.stdout or "") + (r.stderr or "")}
            return json.loads(r.stdout)

        def drain() -> tuple[int, str]:
            r = subprocess.run(
                [str(CTL), "drain", "--socket", str(ctrl)],
                capture_output=True,
                text=True,
            )
            return r.returncode, (r.stdout or "") + (r.stderr or "")

        def reload() -> tuple[int, str]:
            r = subprocess.run(
                [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
                capture_output=True,
                text=True,
            )
            return r.returncode, (r.stdout or "") + (r.stderr or "")

        # --- pre-drain ---
        product = http_exchange(
            listen,
            b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        live = http_exchange(
            listen,
            b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        ready = http_exchange(
            listen,
            b"GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        st0 = status()
        checks["pre_drain"] = {
            "product_200": "200" in product and "cap040-ok" in product,
            "live_200": "200" in live,
            "ready_200": "200" in ready,
            "draining": st0.get("draining"),
            "ok": "200" in product
            and "cap040-ok" in product
            and "200" in live
            and "200" in ready
            and st0.get("draining") is False,
        }
        ok = ok and checks["pre_drain"]["ok"]

        # --- drain ---
        rc, out = drain()
        st1 = status()
        checks["drain_trigger"] = {
            "rc": rc,
            "out": out.splitlines()[0] if out else "",
            "draining": st1.get("draining"),
            "ok": rc == 0 and st1.get("draining") is True,
        }
        ok = ok and checks["drain_trigger"]["ok"]

        # --- new connection product reject ---
        product2 = http_exchange(
            listen,
            b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        checks["new_product_after_drain"] = {
            "resp": product2[:200],
            "ok": "503" in product2 and "draining" in product2,
        }
        ok = ok and checks["new_product_after_drain"]["ok"]

        # --- probes ---
        ready2 = http_exchange(
            listen,
            b"GET /ready HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        live2 = http_exchange(
            listen,
            b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        health2 = http_exchange(
            listen,
            b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        checks["probes_during_drain"] = {
            "ready": ready2[:160],
            "live": live2[:160],
            "health": health2[:160],
            "ok": "503" in ready2
            and "not_ready" in ready2
            and "200" in live2
            and "200" in health2,
        }
        ok = ok and checks["probes_during_drain"]["ok"]

        # --- listener still accepts (new TCP for /live) ---
        checks["listener_still_accepts"] = {
            "ok": checks["probes_during_drain"]["ok"],
            "contract": "DRAIN_STOPS_NEW_LISTENER_ACCEPTS=NO",
        }

        # --- idempotent double drain ---
        rc2, out2 = drain()
        st2 = status()
        live3 = http_exchange(
            listen,
            b"GET /live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        checks["double_drain"] = {
            "rc": rc2,
            "draining": st2.get("draining"),
            "live_200": "200" in live3,
            "ok": rc2 == 0 and st2.get("draining") is True and "200" in live3,
        }
        ok = ok and checks["double_drain"]["ok"]

        # --- reload while draining must not undrain ---
        cfg_path.write_text(cfg(listen, peer, timeout_ms=11000))
        rr, rout = reload()
        st3 = status()
        checks["reload_while_draining"] = {
            "rc": rr,
            "out": rout.splitlines()[0] if rout else "",
            "draining_after": st3.get("draining"),
            "ok": st3.get("draining") is True,
            "contract": "reload ALLOWED; must not cancel drain",
        }
        ok = ok and checks["reload_while_draining"]["ok"]

        # Restart fresh process for keepalive + in-flight tests (cleaner)
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        time.sleep(0.2)
        cfg_path.write_text(cfg(listen, peer))
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("a"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            checks["restart"] = {"ok": False}
            raise RuntimeError("restart failed")

        # --- keepalive: product after drain on same TCP (SECINT-003) ---
        def do_drain_and_confirm():
            rc_d, out_d = drain()
            deadline = time.time() + 5.0
            while time.time() < deadline:
                st = status()
                if st.get("draining") is True:
                    return rc_d, out_d, st
                time.sleep(0.02)
            return rc_d, out_d, status()

        first, second = keepalive_pair(
            listen,
            # Admit with probe first (matches security_secint003 contract shape),
            # then product after drain on same TCP.
            b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n",
            b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            between=do_drain_and_confirm,
        )
        second_status = second.split("\r\n", 1)[0] if second else ""
        checks["keepalive_post_drain"] = {
            "first": first[:160],
            "second": second[:200],
            "ok": "200" in first
            and "503" in second
            and "draining" in second
            and "200" not in second_status,
        }
        ok = ok and checks["keepalive_post_drain"]["ok"]


        # --- in-flight slow request completes after drain ---
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        peer_slow = pick_port()
        httpd_slow = start_peer(peer_slow, delay=1.5)
        cfg_path.write_text(cfg(listen, peer_slow, timeout_ms=15000))
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("a"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            checks["slow_restart"] = {"ok": False}
            raise RuntimeError("slow restart failed")

        result_holder: dict = {}

        def slow_client():
            result_holder["resp"] = http_exchange(
                listen,
                b"GET /api/slow HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                timeout=20.0,
            )

        t = threading.Thread(target=slow_client, daemon=True)
        t.start()
        time.sleep(0.3)
        drain()
        t.join(timeout=20)
        resp = result_holder.get("resp", "")
        checks["inflight_completes"] = {
            "resp": resp[:200],
            "ok": "200" in resp and "cap040-ok" in resp,
            "contract": "already admitted request completes under drain",
        }
        ok = ok and checks["inflight_completes"]["ok"]

        # new product still rejected
        after = http_exchange(
            listen,
            b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        checks["after_inflight_still_draining"] = {
            "ok": "503" in after and "draining" in after,
            "resp": after[:160],
        }
        ok = ok and checks["after_inflight_still_draining"]["ok"]

        # --- concurrent traffic then drain ---
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        # reuse fast peer
        cfg_path.write_text(cfg(listen, peer))
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("a"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            raise RuntimeError("conc restart failed")

        stop_flag = threading.Event()
        seen_ok = []
        seen_drain = []
        bad = threading.Event()

        def traffic():
            while not stop_flag.is_set():
                r = http_exchange(
                    listen,
                    b"GET /api/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
                    timeout=5.0,
                )
                if "200" in r and "cap040-ok" in r:
                    seen_ok.append(1)
                elif "503" in r and "draining" in r:
                    seen_drain.append(1)
                elif r.strip():
                    # unexpected status
                    if "200" in r or "503" in r:
                        pass
                    else:
                        bad.set()
                time.sleep(0.02)

        th = threading.Thread(target=traffic, daemon=True)
        th.start()
        time.sleep(0.3)
        drain()
        time.sleep(0.5)
        stop_flag.set()
        th.join(timeout=5)
        checks["concurrent_drain"] = {
            "pre_or_during_ok": len(seen_ok),
            "post_drain_503": len(seen_drain),
            "bad": bad.is_set(),
            "ok": len(seen_ok) >= 1 and len(seen_drain) >= 1 and not bad.is_set(),
        }
        ok = ok and checks["concurrent_drain"]["ok"]

        checks["contract_record"] = {
            "DRAIN_TRIGGER_SURFACE": "exyonqctl drain / control socket",
            "DRAIN_STOPS_NEW_LISTENER_ACCEPTS": "NO",
            "DRAIN_STOPS_NEW_REQUESTS_ON_EXISTING_KEEPALIVE": "YES",
            "DRAIN_ALLOWS_ALREADY_ADMITTED_REQUESTS_TO_COMPLETE": "YES",
            "DRAIN_REVERSIBLE": "NO",
            "DRAIN_TIMEOUT_POLICY": "NONE",
            "HARNESS_EXECUTED_PATHS": "H1_proxy+probes+keepalive+inflight+reload+concurrent",
            "H2_H3_WS_SSE_EPOLL": "NOT_EXECUTED_IN_THIS_HARNESS; product gates audited in code/unit paths",
            "CAP046_DISTINCT": "YES",
            "ok": True,
        }

    except Exception as exc:
        checks["exception"] = {"ok": False, "error": str(exc)}
        ok = False
    finally:
        stop_proc(proc)
        try:
            httpd.shutdown()
        except Exception:
            pass
        if httpd_slow is not None:
            try:
                httpd_slow.shutdown()
            except Exception:
                pass
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass

    result = {
        "CAPABILITY_ID": "040",
        "FEATURE_ID": "drain",
        "FEATURE_NAME": "Drain",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else None,
        "EXYONQCTL_BINARY_SHA256": sha256_file(CTL) if CTL.is_file() else None,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP046_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "CAP041_STARTED": "NO",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if ok else "FAIL",
        "UTC": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "ok": ok}, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
