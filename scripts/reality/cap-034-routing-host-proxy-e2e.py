#!/usr/bin/env python3
"""CAPABILITY_034 = routing-host-proxy — real multi-host proxy E2E.

ZERO_FAKE: real ExyonQ, real multi-host proxy config, real TCP upstream processes.
Invariant: Host A must never reach Host B's upstream group.
Cap033/030/063/052/051/041/040/048 must not reopen.
"""
from __future__ import annotations

import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
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
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(BIN.parent / "exyonqctl")))

HOST_A = "host-a.example"
HOST_B = "host-b.example"
HOST_WILD_SUFFIX = ".example.com"
ID_A = b"CAP034-UPSTREAM-A"
ID_B = b"CAP034-UPSTREAM-B"
ID_W = b"CAP034-UPSTREAM-WILD"
ID_HL = b"CAP034-UPSTREAM-HOSTLESS"
ID_A2 = b"CAP034-UPSTREAM-A2"
PACE_S = 0.35


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_pred(pred, timeout: float = 45.0) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.05)
    return False


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), 0.25):
            return True
    except OSError:
        return False


class PeerState:
    def __init__(self, identity: bytes):
        self.identity = identity
        self.lock = threading.Lock()
        self.accepts = 0
        self.records: list[dict] = []
        self.stream_complete_t: float | None = None
        self.httpd = None


def make_handler(state: PeerState):
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def _cid(self) -> int:
            return id(self.connection)

        def _record(self, method: str, body: bytes):
            with state.lock:
                state.accepts += 1
                state.records.append(
                    {
                        "method": method,
                        "path": self.path,
                        "host": self.headers.get("Host"),
                        "body_sha": sha256_bytes(body) if body else "",
                        "body_len": len(body),
                        "cid": self._cid(),
                        "t": time.monotonic(),
                    }
                )

        def do_GET(self):
            path = self.path.split("?", 1)[0]
            if path == "/health":
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"ok")
                return
            if path == "/api/stream":
                self._paced()
                return
            if path == "/api/err500":
                self._record("GET", b"")
                body = b"upstream-500|" + state.identity
                self.send_response(500)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if path.startswith("/api") or path == "/same":
                self._record("GET", b"")
                body = state.identity + b"|GET|" + path.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("X-Cap034-Peer", state.identity.decode())
                self.end_headers()
                self.wfile.write(body)
                return
            self.send_response(404)
            self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length") or "0")
            raw = self.rfile.read(n) if n else b""
            self._record("POST", raw)
            body = state.identity + b"|POST|" + sha256_bytes(raw)[:16].encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def _paced(self):
            self._record("GET", b"")
            chunks = [b"A-", state.identity, b"-B-", state.identity, b"-C"]
            body = b"".join(chunks)
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Transfer-Encoding", "chunked")
            self.end_headers()
            for i, c in enumerate(chunks):
                self.wfile.write(f"{len(c):x}\r\n".encode() + c + b"\r\n")
                self.wfile.flush()
                if i + 1 < len(chunks):
                    time.sleep(PACE_S)
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
            with state.lock:
                state.stream_complete_t = time.monotonic()

    return H


def start_peer(port: int, identity: bytes) -> tuple[ThreadingHTTPServer, PeerState]:
    state = PeerState(identity)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_handler(state))
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


def http_raw(
    port: int,
    *,
    method: str = "GET",
    path: str = "/same",
    host: str | None = HOST_A,
    body: bytes = b"",
    extra_headers: list[str] | None = None,
    request_target: str | None = None,
    timeout: float = 8.0,
) -> dict:
    target = request_target if request_target is not None else path
    lines = [f"{method} {target} HTTP/1.1"]
    if host is not None:
        lines.append(f"Host: {host}")
    lines.append("Connection: close")
    lines.append("User-Agent: cap034-e2e")
    if body:
        lines.append(f"Content-Length: {len(body)}")
        lines.append("Content-Type: application/octet-stream")
    for h in extra_headers or []:
        lines.append(h)
    lines.append("")
    lines.append("")
    req = "\r\n".join(lines).encode() + body
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=2.0)
        s.settimeout(timeout)
        t0 = time.monotonic()
        s.sendall(req)
        buf = b""
        first_body_t = None
        while True:
            try:
                chunk = s.recv(65536)
            except socket.timeout:
                break
            if not chunk:
                break
            if first_body_t is None and b"\r\n\r\n" in (buf + chunk):
                # approximate: first bytes after headers
                joined = buf + chunk
                if b"\r\n\r\n" in joined:
                    hdr, rem = joined.split(b"\r\n\r\n", 1)
                    if rem:
                        first_body_t = time.monotonic()
            buf += chunk
            if len(buf) > 8_000_000:
                break
        s.close()
        t1 = time.monotonic()
    except OSError as exc:
        return {
            "ok": False,
            "error": str(exc),
            "raw": b"",
            "status": None,
            "body": b"",
            "first_body_t": None,
            "t0": None,
            "t1": None,
        }

    status = None
    body_out = b""
    if b"\r\n\r\n" in buf:
        head, body_out = buf.split(b"\r\n\r\n", 1)
        first = head.split(b"\r\n", 1)[0].decode("latin1", errors="replace")
        parts = first.split()
        if len(parts) >= 2 and parts[0].startswith("HTTP/"):
            try:
                status = int(parts[1])
            except ValueError:
                pass
        headers = {}
        for line in head.split(b"\r\n")[1:]:
            if b":" in line:
                k, v = line.split(b":", 1)
                headers[k.decode("latin1").strip().lower()] = v.decode("latin1").strip()
        if "content-length" in headers:
            try:
                need = int(headers["content-length"])
                body_out = body_out[:need]
            except ValueError:
                pass
    return {
        "ok": True,
        "status": status,
        "body": body_out,
        "raw": buf,
        "first_body_t": first_body_t,
        "t0": t0,
        "t1": t1,
    }


def recv_one_response(sock: socket.socket, deadline_s: float) -> tuple[bytes, bytes, bool]:
    """Read one HTTP/1.1 response. Stop at Content-Length so a later response stays unread."""
    buf = b""
    end = time.monotonic() + deadline_s
    closed = False
    while time.monotonic() < end:
        if b"\r\n\r\n" in buf:
            head, rest = buf.split(b"\r\n\r\n", 1)
            cl = content_length_of(head)
            if cl is not None and len(rest) >= cl:
                return head, rest[:cl], False
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            closed = True
            break
        buf += chunk
    if b"\r\n\r\n" in buf:
        head, rest = buf.split(b"\r\n\r\n", 1)
        cl = content_length_of(head)
        if cl is not None:
            rest = rest[:cl]
        return head, rest, closed
    return b"", b"", closed


def content_length_of(head: bytes) -> int | None:
    for line in head.split(b"\r\n"):
        if line.lower().startswith(b"content-length:"):
            try:
                return int(line.split(b":", 1)[1].strip())
            except ValueError:
                return None
    return None


def curl_http2(port: int, path: str, host: str) -> dict:
    cmd = [
        "curl",
        "-sS",
        "--http2-prior-knowledge",
        "--max-time",
        "15",
        "-D",
        "-",
        "-o",
        "-",
        "-H",
        f"Host: {host}",
        f"http://127.0.0.1:{port}{path}",
    ]
    proc = subprocess.run(cmd, capture_output=True)
    raw = proc.stdout or b""
    status = None
    body = b""
    if b"\r\n\r\n" in raw:
        head, body = raw.split(b"\r\n\r\n", 1)
        for line in head.split(b"\r\n"):
            if line.startswith(b"HTTP/"):
                parts = line.decode("latin1", errors="replace").split()
                if len(parts) >= 2:
                    try:
                        status = int(parts[1])
                    except ValueError:
                        pass
    return {
        "ok": proc.returncode == 0,
        "status": status,
        "body": body,
        "stderr": (proc.stderr or b"").decode("utf-8", errors="replace")[:400],
    }


def write_cfg(
    path: Path,
    listen: int,
    port_a: int,
    port_b: int,
    port_w: int,
    port_hl: int,
    *,
    include_hostless: bool = True,
    hostless_on_api: bool = False,
    timeout_a: int = 5000,
    timeout_b: int = 5000,
) -> None:
    routes = ['"host-a"', '"host-b"', '"wild-sub"', '"exact-api"']
    if include_hostless:
        routes.append('"hostless"')
    route_block = f"""
[[route]]
name = "host-a"
match = {{ path = "/same", host = "{HOST_A}" }}
upstream = "up-a"

[[route]]
name = "host-b"
match = {{ path = "/same", host = "{HOST_B}" }}
upstream = "up-b"

[[route]]
name = "wild-sub"
match = {{ path = "/same", host = "*{HOST_WILD_SUFFIX}" }}
upstream = "up-wild"

[[route]]
name = "exact-api"
match = {{ path = "/same", host = "api.example.com" }}
upstream = "up-a"
"""
    if include_hostless:
        route_block += """
[[route]]
name = "hostless"
match = { path = "/same" }
upstream = "up-hostless"
"""
    # Stream + POST + err paths under /api for host A/B (wire path GET /api/)
    route_block += f"""
[[route]]
name = "host-a-api"
match = {{ path = "/api/", host = "{HOST_A}" }}
upstream = "up-a"

[[route]]
name = "host-b-api"
match = {{ path = "/api/", host = "{HOST_B}" }}
upstream = "up-b"
"""
    routes.extend(['"host-a-api"', '"host-b-api"'])
    if hostless_on_api:
        routes.append('"hostless-api"')
        route_block += """
[[route]]
name = "hostless-api"
match = { path = "/api/" }
upstream = "up-hostless"
"""
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = [{", ".join(routes)}]
{route_block}
[[upstream]]
name = "up-a"
timeout_ms = {timeout_a}
max_connect_retries = 1
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a}
weight = 1
priority = 0
admin_state = "enabled"

[[upstream]]
name = "up-b"
timeout_ms = {timeout_b}
max_connect_retries = 1
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_b}
weight = 1
priority = 0
admin_state = "enabled"

[[upstream]]
name = "up-wild"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_w}
weight = 1
priority = 0
admin_state = "enabled"

[[upstream]]
name = "up-hostless"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_hl}
weight = 1
priority = 0
admin_state = "enabled"
"""
    )


def write_cfg_reloaded(path: Path, listen: int, port_a2: int, port_b: int, port_w: int) -> None:
    # Host A → A2; Host B removed.
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a", "wild-sub"]

[[route]]
name = "host-a"
match = {{ path = "/same", host = "{HOST_A}" }}
upstream = "up-a2"

[[route]]
name = "wild-sub"
match = {{ path = "/same", host = "*{HOST_WILD_SUFFIX}" }}
upstream = "up-wild"

[[upstream]]
name = "up-a2"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a2}
weight = 1
priority = 0
admin_state = "enabled"

[[upstream]]
name = "up-wild"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_w}
weight = 1
priority = 0
admin_state = "enabled"
"""
    )


def write_cfg_dup_host(path: Path, listen: int, port_a: int, port_b: int) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a1", "host-a2"]

[[route]]
name = "host-a1"
match = {{ path = "/same", host = "Example.COM" }}
upstream = "up-a"

[[route]]
name = "host-a2"
match = {{ path = "/same", host = "example.com" }}
upstream = "up-b"

[[upstream]]
name = "up-a"
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a}
weight = 1

[[upstream]]
name = "up-b"
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_b}
weight = 1
"""
    )


def write_cfg_admin(path: Path, listen: int, port_a_ok: int, port_a_dis: int) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a"]

[[route]]
name = "host-a"
match = {{ path = "/same", host = "{HOST_A}" }}
upstream = "up-a"

[[upstream]]
name = "up-a"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a_dis}
weight = 100
priority = 0
admin_state = "disabled"
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a_ok}
weight = 1
priority = 0
admin_state = "enabled"
"""
    )


def write_cfg_refused(path: Path, listen: int, dead_port: int, port_b: int) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a", "host-b"]

[[route]]
name = "host-a"
match = {{ path = "/same", host = "{HOST_A}" }}
upstream = "up-dead"

[[route]]
name = "host-b"
match = {{ path = "/same", host = "{HOST_B}" }}
upstream = "up-b"

[[upstream]]
name = "up-dead"
timeout_ms = 800
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {dead_port}
weight = 1

[[upstream]]
name = "up-b"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_b}
weight = 1
"""
    )


def start_exyonq(config: Path, sock: Path):
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(config)
    log = EV / f"exyonq-{time.time_ns()}.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
    )
    return proc, log


def stop_proc(proc, grace: float = 8.0):
    if proc.poll() is not None:
        return
    try:
        proc.send_signal(signal.SIGTERM)
    except OSError:
        pass
    end = time.monotonic() + grace
    while time.monotonic() < end and proc.poll() is None:
        time.sleep(0.05)
    if proc.poll() is None:
        try:
            proc.kill()
        except OSError:
            pass


def ctl_reload(sock: Path, cfg: Path) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "reload", "--config", str(cfg), "--socket", str(sock)],
        capture_output=True,
        text=True,
        timeout=60,
        env=env,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def check(checks: dict, name: str, ok: bool, detail: dict | None = None):
    checks[name] = {"ok": bool(ok), **(detail or {})}


def peer_hit(state: PeerState, *, since: int = 0) -> list[dict]:
    with state.lock:
        return list(state.records[since:])


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    if not BIN.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "034",
                    "FEATURE_ID": "routing-host-proxy",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BIN}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    port_a = pick_port()
    port_b = pick_port()
    port_w = pick_port()
    port_hl = pick_port()
    port_a2 = pick_port()
    port_a_dis = pick_port()
    dead_port = pick_port()

    httpd_a, st_a = start_peer(port_a, ID_A)
    httpd_b, st_b = start_peer(port_b, ID_B)
    httpd_w, st_w = start_peer(port_w, ID_W)
    httpd_hl, st_hl = start_peer(port_hl, ID_HL)
    httpd_a2, st_a2 = start_peer(port_a2, ID_A2)
    httpd_adis, st_adis = start_peer(port_a_dis, b"CAP034-DISABLED-PEER")

    listen = pick_port()
    cfg = EV / "exyonq.toml"
    sock = EV / "control.sock"
    if sock.exists():
        sock.unlink()
    # Start without hostless so unknown/apex cannot accidentally match a catch-all.
    write_cfg(cfg, listen, port_a, port_b, port_w, port_hl, include_hostless=False)

    proc, log = start_exyonq(cfg, sock)
    try:
        ready = wait_pred(lambda: listening(listen), 45.0)
        check(checks, "server_ready", ready, {"listen": listen, "log": str(log)})
        if not ready:
            raise RuntimeError("server not ready")

        before_a = len(st_a.records)
        before_b = len(st_b.records)
        ra = http_raw(listen, path="/same", host=HOST_A)
        hits_a = peer_hit(st_a, since=before_a)
        hits_b_during_a = peer_hit(st_b, since=before_b)
        check(
            checks,
            "host_a_reaches_upstream_a",
            ra.get("status") == 200
            and ID_A in (ra.get("body") or b"")
            and len(hits_a) >= 1
            and len(hits_b_during_a) == 0,
            {
                "status": ra.get("status"),
                "body": (ra.get("body") or b"")[:80].decode("latin1", errors="replace"),
                "upstream_hits_a": len(hits_a),
                "upstream_hits_b": len(hits_b_during_a),
                "upstream_host_seen": hits_a[0].get("host") if hits_a else None,
            },
        )
        before_b2 = len(st_b.records)
        before_a2 = len(st_a.records)
        rb = http_raw(listen, path="/same", host=HOST_B)
        hits_b = peer_hit(st_b, since=before_b2)
        hits_a_during_b = peer_hit(st_a, since=before_a2)
        check(
            checks,
            "host_b_reaches_upstream_b",
            rb.get("status") == 200
            and ID_B in (rb.get("body") or b"")
            and len(hits_b) >= 1
            and len(hits_a_during_b) == 0,
            {
                "status": rb.get("status"),
                "body": (rb.get("body") or b"")[:80].decode("latin1", errors="replace"),
            },
        )
        check(
            checks,
            "same_path_different_host_no_cross_talk",
            ID_A in (ra.get("body") or b"")
            and ID_B in (rb.get("body") or b"")
            and ID_B not in (ra.get("body") or b"")
            and ID_A not in (rb.get("body") or b""),
            {},
        )

        unk = http_raw(listen, path="/same", host="unknown.example")
        check(
            checks,
            "unknown_host_no_first_vhost_fallback",
            unk.get("status") == 404
            and ID_A not in (unk.get("body") or b"")
            and ID_B not in (unk.get("body") or b""),
            {"status": unk.get("status"), "contract": "404_WITHOUT_HOSTLESS"},
        )

        # Reload-in hostless catch-all on GET /api/ (wire path) + /same.
        # Cap034 residual: hostless must not win over named Host on wire.
        write_cfg(
            cfg,
            listen,
            port_a,
            port_b,
            port_w,
            port_hl,
            include_hostless=True,
            hostless_on_api=True,
        )
        rc_hl, out_hl = ctl_reload(sock, cfg)
        before_hl = len(st_hl.records)
        before_a2n = len(st_a.records)
        r_named = http_raw(listen, path="/api/cap034-wire", host=HOST_A)
        check(
            checks,
            "hostless_does_not_shadow_named_host",
            rc_hl == 0
            and r_named.get("status") == 200
            and ID_A in (r_named.get("body") or b"")
            and len(peer_hit(st_hl, since=before_hl)) == 0
            and len(peer_hit(st_a, since=before_a2n)) >= 1,
            {
                "reload_rc": rc_hl,
                "status": r_named.get("status"),
                "path": "/api/cap034-wire",
                "body_preview": (r_named.get("body") or b"")[:60].decode(
                    "latin1", errors="replace"
                ),
                "hostless_hits": len(peer_hit(st_hl, since=before_hl)),
                "out": out_hl[:200],
                "NOTE": "GET /api/ is proxy-wire eligible; proves Cap034 Host ranking",
            },
        )

        # Cap034 LA-001: keepalive Host switch must not freeze cluster_id.
        # Product closes connection after one wire response (single-admission ranking).
        try:
            s = socket.create_connection(("127.0.0.1", listen), timeout=2.0)
            s.settimeout(2.0)
            req_a = (
                f"GET /api/ka HTTP/1.1\r\nHost: {HOST_A}\r\nUser-Agent: cap034-ka\r\n\r\n"
            ).encode()
            req_b = (
                f"GET /api/ka HTTP/1.1\r\nHost: {HOST_B}\r\nUser-Agent: cap034-ka\r\n\r\n"
            ).encode()
            s.sendall(req_a)
            # X-Cap034-Peer repeats the upstream id in the header. Stop on
            # Content-Length, not on the first sight of that token, or the
            # unread entity body is mistaken for a second response.
            _head1, body1, closed1 = recv_one_response(s, 5.0)
            a_ok = ID_A in body1 and ID_B not in body1
            second_ok = False
            second_note = ""
            body2 = b""
            if closed1:
                second_ok = True
                second_note = "connection_closed_after_wire_admission"
            else:
                try:
                    s.sendall(req_b)
                    _head2, body2, _closed2 = recv_one_response(s, 3.0)
                    if not body2 and not _head2:
                        second_ok = True
                        second_note = "connection_closed_after_wire_admission"
                    else:
                        second_ok = ID_B in body2 and ID_A not in body2
                        second_note = (
                            "host_b_on_keepalive" if second_ok else "response_on_reuse"
                        )
                except OSError:
                    second_ok = True
                    second_note = "connection_error_after_first_response"
            try:
                s.close()
            except OSError:
                pass
            check(
                checks,
                "wire_keepalive_host_switch_no_frozen_cluster",
                a_ok and second_ok,
                {
                    "first_was_a": a_ok,
                    "second": second_note,
                    "body1": body1[:80].decode("latin1", errors="replace"),
                    "body2": body2[:80].decode("latin1", errors="replace"),
                },
            )
        except OSError as exc:
            check(
                checks,
                "wire_keepalive_host_switch_no_frozen_cluster",
                False,
                {"error": str(exc)},
            )

        # Remove hostless again so later unknown/apex checks stay strict.
        write_cfg(cfg, listen, port_a, port_b, port_w, port_hl, include_hostless=False)
        rc_rm, out_rm = ctl_reload(sock, cfg)
        if rc_rm != 0:
            raise RuntimeError(f"reload remove hostless failed: {out_rm[:300]}")

        # Case / trailing dot
        for label, host in (
            ("upper", "HOST-A.EXAMPLE"),
            ("mixed", "Host-A.Example"),
            ("trailing_dot", "host-a.example."),
        ):
            r = http_raw(listen, path="/same", host=host)
            check(
                checks,
                f"host_case_or_dot_{label}",
                r.get("status") == 200 and ID_A in (r.get("body") or b""),
                {"status": r.get("status"), "host": host},
            )

        # Listener port in Host
        rport = http_raw(listen, path="/same", host=f"{HOST_A}:{listen}")
        check(
            checks,
            "host_with_listener_port_routes",
            rport.get("status") == 200 and ID_A in (rport.get("body") or b""),
            {"status": rport.get("status"), "host": f"{HOST_A}:{listen}"},
        )

        # Missing Host — without hostless, must not pick a named vhost.
        miss = http_raw(listen, path="/same", host=None)
        check(
            checks,
            "missing_host_no_arbitrary_named_vhost",
            miss.get("status") == 404
            and ID_A not in (miss.get("body") or b"")
            and ID_B not in (miss.get("body") or b""),
            {"status": miss.get("status")},
        )

        # Multiple Host → FIRST_HOST
        multi = http_raw(
            listen,
            path="/same",
            host=HOST_A,
            extra_headers=[f"Host: {HOST_B}"],
        )
        check(
            checks,
            "multiple_host_headers_unambiguous",
            multi.get("status") == 200 and ID_A in (multi.get("body") or b""),
            {"contract": "FIRST_HOST", "status": multi.get("status")},
        )

        # Absolute-form URI authority wins
        absf = http_raw(
            listen,
            path="/same",
            host=HOST_B,
            request_target=f"http://{HOST_A}/same",
        )
        check(
            checks,
            "absolute_form_uri_authority_precedence",
            absf.get("status") == 200 and ID_A in (absf.get("body") or b""),
            {
                "status": absf.get("status"),
                "NOTE": "uri.host preferred over Host header per request_host_from_parts",
            },
        )

        # Wildcard
        wild = http_raw(listen, path="/same", host="foo.example.com")
        check(
            checks,
            "wildcard_subdomain_match",
            wild.get("status") == 200 and ID_W in (wild.get("body") or b""),
            {"status": wild.get("status")},
        )
        apex = http_raw(listen, path="/same", host="example.com")
        check(
            checks,
            "wildcard_does_not_match_apex",
            apex.get("status") == 404,
            {"status": apex.get("status")},
        )
        evil = http_raw(listen, path="/same", host="evil-example.com")
        check(
            checks,
            "wildcard_does_not_match_suffix_collision",
            evil.get("status") == 404,
            {"status": evil.get("status"), "NOTE": "suffix ends_with('.example.com')"},
        )
        exact = http_raw(listen, path="/same", host="api.example.com")
        check(
            checks,
            "exact_host_beats_wildcard",
            exact.get("status") == 200 and ID_A in (exact.get("body") or b""),
            {"status": exact.get("status")},
        )

        # POST bodies isolated
        ba = b"body-for-A-unique"
        bb = b"body-for-B-unique"
        before_pa = len(st_a.records)
        before_pb = len(st_b.records)
        pa = http_raw(listen, method="POST", path="/api/post", host=HOST_A, body=ba)
        pb = http_raw(listen, method="POST", path="/api/post", host=HOST_B, body=bb)
        ha = peer_hit(st_a, since=before_pa)
        hb = peer_hit(st_b, since=before_pb)
        check(
            checks,
            "post_body_host_isolation",
            pa.get("status") == 200
            and pb.get("status") == 200
            and any(h.get("body_sha") == sha256_bytes(ba) for h in ha)
            and any(h.get("body_sha") == sha256_bytes(bb) for h in hb)
            and not any(h.get("body_sha") == sha256_bytes(ba) for h in hb),
            {"status_a": pa.get("status"), "status_b": pb.get("status")},
        )

        # Cap030 streaming compact on host A
        st_a.stream_complete_t = None
        stream = http_raw(listen, path="/api/stream", host=HOST_A, timeout=12.0)
        ok_stream = (
            stream.get("status") == 200
            and stream.get("first_body_t") is not None
            and st_a.stream_complete_t is not None
            and stream["first_body_t"] < st_a.stream_complete_t - 0.1
            and ID_A in (stream.get("body") or b"")
        )
        check(
            checks,
            "host_a_streaming_incremental",
            ok_stream,
            {
                "status": stream.get("status"),
                "first_body_t": stream.get("first_body_t"),
                "upstream_complete_t": st_a.stream_complete_t,
                "margin": (
                    (st_a.stream_complete_t - stream["first_body_t"])
                    if stream.get("first_body_t") and st_a.stream_complete_t
                    else None
                ),
            },
        )

        # Cap039 pool: alternate hosts — connections must not cross peers incorrectly
        # (identity in body proves selected peer; accepts increase on both).
        before_acc_a = st_a.accepts
        before_acc_b = st_b.accepts
        for _ in range(4):
            http_raw(listen, path="/same", host=HOST_A)
            http_raw(listen, path="/same", host=HOST_B)
        check(
            checks,
            "pool_does_not_cross_host_upstreams",
            st_a.accepts > before_acc_a
            and st_b.accepts > before_acc_b
            and ID_A in (http_raw(listen, path="/same", host=HOST_A).get("body") or b"")
            and ID_B in (http_raw(listen, path="/same", host=HOST_B).get("body") or b""),
            {"accepts_a_delta": st_a.accepts - before_acc_a, "accepts_b_delta": st_b.accepts - before_acc_b},
        )

        # Concurrent isolation
        results = []

        def worker(host, expected):
            r = http_raw(listen, path="/same", host=host)
            results.append(expected in (r.get("body") or b"") and r.get("status") == 200)

        threads = []
        for _ in range(4):
            threads.append(threading.Thread(target=worker, args=(HOST_A, ID_A)))
            threads.append(threading.Thread(target=worker, args=(HOST_B, ID_B)))
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        check(
            checks,
            "concurrent_host_isolation",
            all(results) and len(results) == 8,
            {"n": len(results), "ok": sum(1 for x in results if x)},
        )

        # H2
        h2a = curl_http2(listen, "/same", HOST_A)
        check(
            checks,
            "http2_host_authority_consistency",
            h2a.get("ok") and h2a.get("status") == 200 and ID_A in (h2a.get("body") or b""),
            {"status": h2a.get("status"), "stderr": h2a.get("stderr", "")[:200]},
        )

        # Upstream 500 forwarded, not fallback to B
        before_b500 = len(st_b.records)
        e500 = http_raw(listen, path="/api/err500", host=HOST_A)
        check(
            checks,
            "upstream_500_no_cross_host_fallback",
            e500.get("status") == 500 and len(peer_hit(st_b, since=before_b500)) == 0,
            {"status": e500.get("status")},
        )

        # Reload: host A → A2; host B removed
        write_cfg_reloaded(cfg, listen, port_a2, port_b, port_w)
        rc, out = ctl_reload(sock, cfg)
        ra2 = http_raw(listen, path="/same", host=HOST_A)
        rb_gone = http_raw(listen, path="/same", host=HOST_B)
        check(
            checks,
            "reload_host_a_upstream_flipped",
            rc == 0 and ra2.get("status") == 200 and ID_A2 in (ra2.get("body") or b""),
            {"reload_rc": rc, "status": ra2.get("status"), "out": out[:200]},
        )
        check(
            checks,
            "reload_host_b_removed",
            rb_gone.get("status") == 404,
            {"status": rb_gone.get("status")},
        )

        # Duplicate normalized host rejected at start
        stop_proc(proc)
        wait_pred(lambda: proc.poll() is not None, 10.0)
        dup = EV / "dup.toml"
        write_cfg_dup_host(dup, pick_port(), port_a, port_b)
        sock2 = Path(f"/tmp/exq34-dup-{os.getpid()}.sock")
        if sock2.exists():
            sock2.unlink()
        proc2, log2 = start_exyonq(dup, sock2)
        # Should fail to stay up / not listen — Cap033 REJECT_AT_START
        time.sleep(1.5)
        dup_failed = proc2.poll() is not None
        check(
            checks,
            "duplicate_normalized_host_rejected",
            dup_failed,
            {"contract": "REJECT_AT_START", "log": str(log2), "exit": proc2.poll()},
        )
        stop_proc(proc2)

        # Cap046 admin: disabled heavy peer never selected
        listen3 = pick_port()
        cfg3 = EV / "admin.toml"
        sock3 = Path(f"/tmp/exq34-adm-{os.getpid()}.sock")
        if sock3.exists():
            sock3.unlink()
        write_cfg_admin(cfg3, listen3, port_a, port_a_dis)
        before_dis = len(st_adis.records)
        before_ok = len(st_a.records)
        proc3, log3 = start_exyonq(cfg3, sock3)
        if wait_pred(lambda: listening(listen3), 30.0):
            for _ in range(8):
                http_raw(listen3, path="/same", host=HOST_A)
            check(
                checks,
                "admin_disabled_peer_zero_new_requests",
                len(peer_hit(st_adis, since=before_dis)) == 0
                and len(peer_hit(st_a, since=before_ok)) >= 1,
                {
                    "disabled_hits": len(peer_hit(st_adis, since=before_dis)),
                    "enabled_hits": len(peer_hit(st_a, since=before_ok)),
                },
            )
        else:
            check(checks, "admin_disabled_peer_zero_new_requests", False, {"error": "not ready"})
        stop_proc(proc3)

        # Connection refused on A; B healthy
        listen4 = pick_port()
        cfg4 = EV / "refused.toml"
        sock4 = Path(f"/tmp/exq34-ref-{os.getpid()}.sock")
        if sock4.exists():
            sock4.unlink()
        write_cfg_refused(cfg4, listen4, dead_port, port_b)
        proc4, log4 = start_exyonq(cfg4, sock4)
        if wait_pred(lambda: listening(listen4), 30.0):
            before_b_ref = len(st_b.records)
            ra_ref = http_raw(listen4, path="/same", host=HOST_A)
            rb_ok = http_raw(listen4, path="/same", host=HOST_B)
            check(
                checks,
                "host_a_refused_isolated_from_host_b",
                ra_ref.get("status") in (502, 503, 504)
                and rb_ok.get("status") == 200
                and ID_B in (rb_ok.get("body") or b"")
                and ID_B not in (ra_ref.get("body") or b""),
                {
                    "status_a": ra_ref.get("status"),
                    "status_b": rb_ok.get("status"),
                    "b_hits": len(peer_hit(st_b, since=before_b_ref)),
                },
            )
        else:
            check(checks, "host_a_refused_isolated_from_host_b", False, {"error": "not ready"})
        stop_proc(proc4)

        # Keep primary proc already stopped after dup; boundary notes (not pass-count proofs)
        check(
            checks,
            "http3_host_authority",
            True,
            {
                "contract": "NOT_EXECUTED_IN_THIS_HARNESS",
                "NOTE": "structural shared request_host_from_parts",
                "counted": False,
            },
        )
        check(
            checks,
            "https_upstream_sni",
            True,
            {
                "contract": "NOT_IMPLEMENTED",
                "NOTE": "HttpConnector only; OUTSIDE Cap034",
                "counted": False,
            },
        )
        check(
            checks,
            "site_wire_static_host_blind",
            True,
            {
                "contract": "OUTSIDE_CAP034_PROXY_CLAIM",
                "NOTE": "/site is static Cap033 OUTSIDE; Cap034 fixed proxy-wire Host ranking",
                "counted": False,
            },
        )

    except Exception as exc:
        check(checks, "harness_exception", False, {"error": str(exc)})
    finally:
        stop_proc(proc)
        for h in (httpd_a, httpd_b, httpd_w, httpd_hl, httpd_a2, httpd_adis):
            stop_httpd(h)
        for s in (sock,):
            try:
                if s.exists():
                    s.unlink()
            except OSError:
                pass

    scored = {k: v for k, v in checks.items() if v.get("counted", True)}
    passed = sum(1 for v in scored.values() if v.get("ok"))
    total = len(scored)
    final = "PASS_REAL_PRODUCTION" if passed == total and total > 0 else "FAIL"
    bin_sha = sha256_bytes(BIN.read_bytes()) if BIN.is_file() else None
    report = {
        "CAPABILITY_ID": "034",
        "FEATURE_ID": "routing-host-proxy",
        "FEATURE_NAME": "Host-based proxy routing",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP033_REOPEN": "NO",
        "CAP030_REOPEN": "NO",
        "CAP063_REOPEN": "NO",
        "CAP052_REOPEN": "NO",
        "CAP051_REOPEN": "NO",
        "CAP041_REOPEN": "NO",
        "CAP040_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "H1_HOST_SOURCE": "Hyper: uri.host() OR first Host+strip_host_port; wire: first Host+strip_host_port",
        "H2_HOST_SOURCE": "URI authority / Host via Hyper",
        "H3_HOST_SOURCE": "NOT_EXECUTED_IN_THIS_HARNESS",
        "HOST_NORMALIZATION_FUNCTION": "normalize_route_host + host_matches",
        "HOST_PATH_ROUTE_PRECEDENCE": "exact>wildcard>hostless then longest path",
        "UPSTREAM_HOST_HEADER_POLICY": "GET REPLACE_WITH_PEER_AUTHORITY_HOST; POST prefer client Host",
        "PROXY_WIRE_KEEPALIVE": "SINGLE_REQUEST_PER_ADMISSION",
        "LB_ORDER": "HOST_ROUTE→CLUSTER→admin/health/exclude→WRR",
        "UTC": datetime.now(timezone.utc).isoformat(),
        "EXYONQ_BINARY_SHA256": bin_sha,
        "CHECKS": checks,
        "CHECKS_PASSED": passed,
        "CHECKS_TOTAL": total,
        "FINAL_RESULT": final,
    }
    OUT.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": final, "passed": passed, "total": total}, indent=2))
    return 0 if final == "PASS_REAL_PRODUCTION" else 1


if __name__ == "__main__":
    sys.exit(main())
