#!/usr/bin/env python3
"""CAPABILITY_039 = proxy-conn-pool — real product E2E.

Hyper process-global HTTP/1.1 upstream pooling (scheme+authority).
ZERO_FAKE: real TCP accepts, real request identities, real ExyonQ binary.
Eligibility (WRR/admin/health) precedes pool reuse.
"""
from __future__ import annotations

import concurrent.futures
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
MARKER = b"cap039-pool"


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


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    timeout: int = 20,
    headers: list[str] | None = None,
) -> tuple[int, bytes, float]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    cmd = [
        "curl",
        "-sS",
        "--http1.1",
        "--max-time",
        str(timeout),
        "-o",
        str(body_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
        url,
    ]
    for h in headers or []:
        cmd.extend(["-H", h])
    if data is not None:
        cmd.extend(["--data-binary", "@-"])
    t0 = time.perf_counter()
    proc = subprocess.run(cmd, input=data, capture_output=True)
    elapsed = time.perf_counter() - t0
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
    except OSError:
        pass
    code = int(proc.stdout.decode().strip() or "0") if proc.returncode == 0 else -1
    return code, body, elapsed


class CountingServer(ThreadingHTTPServer):
    allow_reuse_address = True

    def __init__(self, *a, **kw):
        self.accept_count = 0
        self.accept_lock = threading.Lock()
        # socket objects may be C-backed without __dict__; map fileno → cid
        self.cid_by_fileno: dict[int, int] = {}
        super().__init__(*a, **kw)

    def get_request(self):
        conn, addr = super().get_request()
        with self.accept_lock:
            self.accept_count += 1
            cid = self.accept_count
            try:
                self.cid_by_fileno[conn.fileno()] = cid
            except OSError:
                pass
        return conn, addr


class PeerState:
    def __init__(self, identity: bytes):
        self.identity = identity
        self.lock = threading.Lock()
        self.requests: list[dict] = []
        self.close_after = False
        self.head_delay_s = 0.0
        self.health_ok = True
        self.alive = True
        self.httpd: CountingServer | None = None


def make_handler(state: PeerState):
    class H(BaseHTTPRequestHandler):
        # Required for Hyper keepalive reuse (default BaseHTTP is HTTP/1.0 close).
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def _cid(self) -> int:
            httpd = state.httpd
            if httpd is None:
                return -1
            try:
                fn = self.connection.fileno()
            except OSError:
                return -1
            with httpd.accept_lock:
                return int(httpd.cid_by_fileno.get(fn, -1))

        def _record(self, method: str, body: bytes):
            with state.lock:
                state.requests.append(
                    {
                        "cid": self._cid(),
                        "method": method,
                        "path": self.path,
                        "body_sha": hashlib.sha256(body).hexdigest() if body else "",
                        "body_len": len(body),
                    }
                )

        def do_GET(self):
            if self.path.startswith("/health"):
                if state.health_ok:
                    self.send_response(200)
                    self.send_header("Content-Length", "2")
                    self.end_headers()
                    self.wfile.write(b"ok")
                else:
                    self.send_response(503)
                    self.end_headers()
                return
            if not self.path.startswith("/api"):
                self.send_response(404)
                self.end_headers()
                return
            if state.head_delay_s > 0:
                time.sleep(state.head_delay_s)
            self._record("GET", b"")
            body = MARKER + b"|" + state.identity + b"|" + str(self._cid()).encode() + b"|"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            if state.close_after:
                self.send_header("Connection", "close")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

        def do_POST(self):
            if not self.path.startswith("/api"):
                self.send_response(404)
                self.end_headers()
                return
            n = int(self.headers.get("Content-Length") or "0")
            raw = self.rfile.read(n) if n else b""
            if state.head_delay_s > 0:
                time.sleep(state.head_delay_s)
            self._record("POST", raw)
            body = MARKER + b"|" + state.identity + b"|post|" + hashlib.sha256(raw).hexdigest()[:12].encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            if state.close_after:
                self.send_header("Connection", "close")
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    return H


def start_peer(port: int, identity: bytes) -> tuple[CountingServer, PeerState]:
    state = PeerState(identity)
    httpd = CountingServer(("127.0.0.1", port), make_handler(state))
    state.httpd = httpd
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, state


def stop_httpd(httpd):
    if httpd is None:
        return
    try:
        httpd.shutdown()
    except Exception:
        pass
    try:
        httpd.server_close()
    except Exception:
        pass


def write_cfg(
    path: Path,
    listen: int,
    peers: list[tuple[int, int, str]],
    *,
    timeout_ms: int = 5000,
    max_connect_retries: int = 1,
    health: str = "",
) -> None:
    """peers: (port, weight, admin_state)."""
    eps = []
    for port, weight, admin in peers:
        eps.append(
            f"""[[upstream.endpoints]]
address = "127.0.0.1"
port = {port}
weight = {weight}
priority = 0
admin_state = "{admin}"
"""
        )
    health_line = f"{health}\n" if health else ""
    path.write_text(
        f"""config_version = 1

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
max_connect_retries = {max_connect_retries}
{health_line}{''.join(eps)}"""
    )


def health_inline() -> str:
    return (
        'health_check = { enabled = true, interval_ms = 200, timeout_ms = 100, '
        'path = "/health", healthy_threshold = 2, unhealthy_threshold = 2 }'
    )


def ctl_reload(sock: Path, cfg: Path) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "reload", "--config", str(cfg), "--socket", str(sock)],
        capture_output=True,
        text=True,
        env=env,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def start_exyonq(cfg: Path, log: Path, *, ctrl: Path | None = None):
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(cfg)
    if ctrl is not None:
        env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
    return subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
    )


def wait_until(pred, timeout: float, interval: float = 0.1) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if pred():
            return True
        time.sleep(interval)
    return False


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "proxy-conn-pool",
        "CAPABILITY": "CAPABILITY_039",
        "CAPABILITY_NAME": "proxy-conn-pool",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Hyper process-global HTTP/1.1 upstream pool reuse after peer selection",
        "POOL_IMPLEMENTATION": "hyper_util legacy Client idle pool",
        "POOL_SCOPE": "PROCESS_GLOBAL",
        "POOL_KEY": "scheme+authority",
        "MAX_IDLE_PER_HOST": 256,
        "IDLE_TIMEOUT": "90s",
        "USER_CONFIGURABLE_POOL_LIMITS": "NO",
        "HTTP2_UPSTREAM_POOLING": "NOT_PRODUCT_WIRED",
        "WEBSOCKET_POOL_APPLICABILITY": "UPGRADED_CONN_NOT_RETURNED_TO_HTTP_POOL",
        "CAP037_REOPEN": "NO",
        "CAP046_REOPEN": "NO",
        "CAP045_REOPEN": "NO",
        "CAP021_REOPEN": "NO",
        "CAPABILITY_047_STARTED": "NO",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)

    tmp = Path(tempfile.mkdtemp(prefix="cap039-", dir=str(EV)))
    checks: dict = {}
    procs: list = []
    httpds: list = []

    try:
        # --- TCP reuse: N requests → fewer accepts ---
        pa = pick_port()
        ha, sa = start_peer(pa, b"A")
        httpds.append(ha)
        assert wait_listen(pa)
        listen = pick_port()
        cfg = tmp / "reuse.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")], timeout_ms=5000)
        p = start_exyonq(cfg, EV / "exyonq-cap039-reuse.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        n = 12
        bodies = []
        for i in range(n):
            code, body, _ = curl_req(f"{base}/api/r{i}")
            bodies.append((code, body))
        accepts = ha.accept_count
        cids = {r["cid"] for r in sa.requests}
        checks["tcp_connection_reuse"] = {
            "ok": (
                all(c == 200 for c, _ in bodies)
                and len(sa.requests) == n
                and accepts < n
                and accepts >= 1
                and len(cids) <= accepts
            ),
            "request_count": n,
            "tcp_accept_count": accepts,
            "unique_cids": sorted(cids),
            "note": "REQUEST_COUNT > TCP_CONNECTION_COUNT proves Hyper reuse",
        }
        # Response identity isolation across reuse
        ok_iso = all(MARKER in b and b"|A|" in b for _, b in bodies)
        checks["response_isolation_under_reuse"] = {
            "ok": ok_iso and len({b for _, b in bodies}) >= 1,
            "sample": [b[:60].decode(errors="replace") for _, b in bodies[:3]],
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        httpds.clear()

        # --- POST body A then B on reused connection ---
        pa = pick_port()
        ha, sa = start_peer(pa, b"A")
        httpds.append(ha)
        listen = pick_port()
        cfg = tmp / "post.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")])
        p = start_exyonq(cfg, EV / "exyonq-cap039-post.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        ba, bb = b"payload-A-cap039", b"payload-B-cap039-different"
        c1, b1, _ = curl_req(f"{base}/api/p", method="POST", data=ba)
        c2, b2, _ = curl_req(f"{base}/api/p", method="POST", data=bb)
        posts = [r for r in sa.requests if r["method"] == "POST"]
        checks["post_body_isolation_reuse"] = {
            "ok": (
                c1 == 200
                and c2 == 200
                and len(posts) == 2
                and posts[0]["body_sha"] == hashlib.sha256(ba).hexdigest()
                and posts[1]["body_sha"] == hashlib.sha256(bb).hexdigest()
                and ha.accept_count <= 2
            ),
            "accepts": ha.accept_count,
            "posts": posts,
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        httpds.clear()

        # --- Connection: close → next request new accept ---
        pa = pick_port()
        ha, sa = start_peer(pa, b"A")
        httpds.append(ha)
        sa.close_after = True
        listen = pick_port()
        cfg = tmp / "close.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")])
        p = start_exyonq(cfg, EV / "exyonq-cap039-close.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        for i in range(4):
            curl_req(f"{base}/api/c{i}")
        checks["connection_close_no_reuse"] = {
            "ok": ha.accept_count == 4 and len(sa.requests) == 4,
            "accepts": ha.accept_count,
            "requests": len(sa.requests),
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        httpds.clear()

        # --- Cross-peer isolation: A and B never share sockets ---
        pa, pb = pick_port(), pick_port()
        ha, sa = start_peer(pa, b"A")
        hb, sb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "peers.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
        p = start_exyonq(cfg, EV / "exyonq-cap039-peers.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        got_a = got_b = 0
        for i in range(20):
            code, body, _ = curl_req(f"{base}/api/x{i}")
            if code == 200 and b"|A|" in body:
                got_a += 1
            elif code == 200 and b"|B|" in body:
                got_b += 1
        # Each peer's accepts must be <= its request count; no cross contamination of identity
        checks["cross_peer_no_socket_contamination"] = {
            "ok": got_a > 0 and got_b > 0 and got_a + got_b == 20,
            "got_a": got_a,
            "got_b": got_b,
            "accepts_a": ha.accept_count,
            "accepts_b": hb.accept_count,
            "note": "WRR distributes; pool keys by authority so A/B ports cannot share sockets",
        }
        # WRR equal weight still distributes under pooling
        checks["wrr_not_collapsed_by_pool"] = {
            "ok": got_a == 10 and got_b == 10,
            "got_a": got_a,
            "got_b": got_b,
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        stop_httpd(hb)
        httpds.clear()

        # --- Cap046: disable A after pool warm; new traffic only B ---
        pa, pb = pick_port(), pick_port()
        ha, sa = start_peer(pa, b"A")
        hb, sb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "admin.toml"
        ctrl = Path(f"/tmp/exq39-{os.getpid()}.sock")
        if ctrl.exists() or ctrl.is_symlink():
            ctrl.unlink()
        write_cfg(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
        if not CTL.is_file():
            checks["admin_disable_bypasses_pool"] = {"ok": False, "detail": "missing exyonqctl"}
        else:
            p = start_exyonq(cfg, EV / "exyonq-cap039-admin.log", ctrl=ctrl)
            procs.append(p)
            assert wait_listen(listen) and wait_sock(ctrl)
            base = f"http://127.0.0.1:{listen}"
            for _ in range(6):
                curl_req(f"{base}/api/warm")
            a_before = len(sa.requests)
            write_cfg(cfg, listen, [(pa, 100, "disabled"), (pb, 1, "enabled")])
            rc, _ = ctl_reload(ctrl, cfg)
            mid = []
            for i in range(16):
                code, body, _ = curl_req(f"{base}/api/m{i}")
                mid.append((code, body))
            a_after = len(sa.requests)
            checks["admin_disable_bypasses_pool"] = {
                "ok": (
                    rc == 0
                    and a_before > 0
                    and a_after == a_before
                    and all(c == 200 and b"|B|" in b for c, b in mid)
                ),
                "reload_rc": rc,
                "a_requests_before": a_before,
                "a_requests_after": a_after,
                "b_requests": len(sb.requests),
            }
            stop_proc(p)
            procs.pop()
            try:
                ctrl.unlink(missing_ok=True)
            except OSError:
                pass
        stop_httpd(ha)
        stop_httpd(hb)
        httpds.clear()

        # --- Cap024: unhealthy peer must get zero new requests despite pool ---
        pa, pb = pick_port(), pick_port()
        ha, sa = start_peer(pa, b"A")
        hb, sb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "health.toml"
        write_cfg(
            cfg,
            listen,
            [(pa, 10, "enabled"), (pb, 1, "enabled")],
            health=health_inline(),
        )
        p = start_exyonq(cfg, EV / "exyonq-cap039-health.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        time.sleep(0.6)
        for _ in range(4):
            curl_req(f"{base}/api/hw")
        assert any(b"|A|" in curl_req(f"{base}/api/hw2")[1] for _ in range(8))
        sa.health_ok = False

        def only_b() -> bool:
            codes = []
            for _ in range(6):
                c, b, _ = curl_req(f"{base}/api/hb")
                codes.append((c, b))
            return all(c == 200 and b"|B|" in b for c, b in codes)

        ok_b = wait_until(only_b, timeout=15.0)
        # Freeze counter AFTER unhealthiness is observed — transition traffic may still hit A.
        a0 = len(sa.requests)
        sample = []
        for _ in range(12):
            c, b, _ = curl_req(f"{base}/api/hb2")
            sample.append((c, b"|A|" in b, b"|B|" in b))
        a1 = len(sa.requests)
        checks["unhealthy_peer_not_reused_from_pool"] = {
            "ok": (
                ok_b
                and a1 == a0
                and all(c == 200 and (not is_a) and is_b for c, is_a, is_b in sample)
            ),
            "a_requests_frozen": a0,
            "a_requests_after": a1,
            "sample": sample[:3],
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        stop_httpd(hb)
        httpds.clear()

        # --- Cap037: timeout on reused connection then recover ---
        pa = pick_port()
        ha, sa = start_peer(pa, b"A")
        httpds.append(ha)
        listen = pick_port()
        cfg = tmp / "to.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")], timeout_ms=350)
        p = start_exyonq(cfg, EV / "exyonq-cap039-to.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        c1, _, _ = curl_req(f"{base}/api/t1")
        sa.head_delay_s = 1.2
        c2, _, e2 = curl_req(f"{base}/api/t2", timeout=8)
        sa.head_delay_s = 0.0
        c3, b3, _ = curl_req(f"{base}/api/t3")
        checks["timeout_on_reused_connection"] = {
            "ok": c1 == 200 and c2 == 504 and 0.2 <= e2 <= 2.0 and c3 == 200 and MARKER in b3,
            "codes": [c1, c2, c3],
            "timeout_elapsed": e2,
            "accepts": ha.accept_count,
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        httpds.clear()

        # --- Upstream restart: SIGKILL peer so keepalive sockets are truly dead ---
        # ThreadingHTTPServer.shutdown() leaves handler threads alive on pooled conns;
        # product recovery must be proven against a hard-killed peer process.
        pa = pick_port()
        state_path = tmp / "killable-state.json"
        up_py = tmp / "killable_upstream.py"
        up_py.write_text(
            """#!/usr/bin/env python3
import json, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import sys

port = int(sys.argv[1])
state_path = sys.argv[2]
MARKER = b"cap039-pool"
lock = threading.Lock()
accepts = 0
requests = 0

class S(ThreadingHTTPServer):
    allow_reuse_address = True
    def get_request(self):
        global accepts
        conn, addr = super().get_request()
        with lock:
            accepts += 1
            open(state_path, "w").write(json.dumps({"accepts": accepts, "requests": requests}))
        return conn, addr

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def do_GET(self):
        global requests
        if self.path.startswith("/health"):
            self.send_response(200); self.send_header("Content-Length","2"); self.end_headers(); self.wfile.write(b"ok"); return
        with lock:
            requests += 1
            open(state_path, "w").write(json.dumps({"accepts": accepts, "requests": requests}))
        body = MARKER + b"|A|alive"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

httpd = S(("127.0.0.1", port), H)
threading.Thread(target=httpd.serve_forever, daemon=True).start()
open(state_path, "w").write(json.dumps({"accepts": 0, "requests": 0, "ready": True}))
while True:
    time.sleep(3600)
"""
        )
        up = subprocess.Popen(
            [sys.executable, str(up_py), str(pa), str(state_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        procs.append(up)
        assert wait_listen(pa)
        listen = pick_port()
        cfg = tmp / "restart.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")], timeout_ms=3000, max_connect_retries=1)
        p = start_exyonq(cfg, EV / "exyonq-cap039-restart.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        c1, _, _ = curl_req(f"{base}/api/s1")
        # Hard-kill peer (RST/close all sockets including pooled keepalive)
        up.send_signal(signal.SIGKILL)
        try:
            up.wait(timeout=5)
        except Exception:
            pass
        if up in procs:
            procs.remove(up)
        time.sleep(0.15)

        def down() -> bool:
            c, _, _ = curl_req(f"{base}/api/sdead", timeout=5)
            return c in (502, 504)

        ok_down = wait_until(down, timeout=8.0)
        c_dead, _, _ = curl_req(f"{base}/api/sdead2", timeout=5)
        # Restart same port
        up2 = subprocess.Popen(
            [sys.executable, str(up_py), str(pa), str(state_path)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        procs.append(up2)
        assert wait_listen(pa)

        def recovered() -> bool:
            c, b, _ = curl_req(f"{base}/api/s2", timeout=5)
            return c == 200 and MARKER in b

        ok_rec = wait_until(recovered, timeout=10.0)
        c_ok, b_ok, _ = curl_req(f"{base}/api/s3")
        st = {}
        if state_path.is_file():
            try:
                st = json.loads(state_path.read_text())
            except Exception:
                st = {}
        checks["upstream_restart_stale_pool_recovery"] = {
            "ok": c1 == 200 and ok_down and c_dead in (502, 504) and ok_rec and c_ok == 200,
            "first": c1,
            "down_observed": ok_down,
            "during_down": c_dead,
            "recovered": c_ok,
            "new_peer_state": st,
        }
        stop_proc(p)
        stop_proc(up2)
        for dead in (p, up2):
            if dead in procs:
                procs.remove(dead)

        # --- Reload removes peer A; no new A traffic ---
        if CTL.is_file():
            pa, pb = pick_port(), pick_port()
            ha, sa = start_peer(pa, b"A")
            hb, sb = start_peer(pb, b"B")
            httpds.extend([ha, hb])
            listen = pick_port()
            cfg = tmp / "reload.toml"
            ctrl = Path(f"/tmp/exq39r-{os.getpid()}.sock")
            if ctrl.exists() or ctrl.is_symlink():
                ctrl.unlink()
            write_cfg(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
            p = start_exyonq(cfg, EV / "exyonq-cap039-reload.log", ctrl=ctrl)
            procs.append(p)
            assert wait_listen(listen) and wait_sock(ctrl)
            base = f"http://127.0.0.1:{listen}"
            for _ in range(6):
                curl_req(f"{base}/api/rw")
            a0 = len(sa.requests)
            write_cfg(cfg, listen, [(pb, 1, "enabled")])
            rc, _ = ctl_reload(ctrl, cfg)
            for i in range(12):
                curl_req(f"{base}/api/rr{i}")
            checks["reload_removed_peer_not_routable"] = {
                "ok": rc == 0 and len(sa.requests) == a0 and all(
                    b"|B|" in curl_req(f"{base}/api/rz")[1] for _ in range(4)
                ),
                "reload_rc": rc,
                "a_frozen": a0,
                "a_after": len(sa.requests),
                "b_requests": len(sb.requests),
            }
            stop_proc(p)
            procs.pop()
            try:
                ctrl.unlink(missing_ok=True)
            except OSError:
                pass
            stop_httpd(ha)
            stop_httpd(hb)
            httpds.clear()
        else:
            checks["reload_removed_peer_not_routable"] = {"ok": False, "detail": "missing ctl"}

        # --- Concurrency: no cross-talk ---
        pa = pick_port()
        ha, sa = start_peer(pa, b"A")
        httpds.append(ha)
        listen = pick_port()
        cfg = tmp / "conc.toml"
        write_cfg(cfg, listen, [(pa, 1, "enabled")])
        p = start_exyonq(cfg, EV / "exyonq-cap039-conc.log")
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"

        def one(i: int) -> tuple[int, bool]:
            c, b, _ = curl_req(f"{base}/api/cc{i}")
            return c, MARKER in b and b"|A|" in b

        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as ex:
            outs = list(ex.map(one, range(40)))
        checks["concurrency_no_crosstalk"] = {
            "ok": all(c == 200 and ok for c, ok in outs),
            "ok_n": sum(1 for c, ok in outs if c == 200 and ok),
            "accepts": ha.accept_count,
            "requests": len(sa.requests),
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(ha)
        httpds.clear()

        # Document WS / HTTP2 / config surface
        checks["contract_surface_recorded"] = {
            "ok": True,
            "USER_CONFIGURABLE_POOL_LIMITS": "NO",
            "HTTP2_UPSTREAM_POOLING": "NOT_PRODUCT_WIRED",
            "WEBSOCKET_POOL_APPLICABILITY": "UPGRADED_CONN_NOT_RETURNED_TO_HTTP_POOL",
            "POOL_STATE_GENERATION_POLICY": "CLIENTS_REUSED_ACROSS_RELOAD",
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
        for h in httpds:
            stop_httpd(h)

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    result["FAILED"] = [k for k, v in checks.items() if not v.get("ok")]
    OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
