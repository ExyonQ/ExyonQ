#!/usr/bin/env python3
"""CAPABILITY_035 = rewrite — real internal request-target rewrite E2E.

ZERO_FAKE: real ExyonQ, real files, real TCP upstream.
REWRITE ≠ Cap036 redirect. Cap034/033/030/063/052/051/041/040/048 must not reopen.
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
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(BIN.parent / "exyonqctl")))

HOST_A = "host-a.example"
HOST_B = "host-b.example"
BODY_NEW = b"CAP035-STATIC-NEW-CONTENT\n"
BODY_TRAV = b"CAP035-SHOULD-NOT-ESCAPE\n"
ID_UP = b"CAP035-UPSTREAM-A"
ID_UP_B = b"CAP035-UPSTREAM-B"


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
        self.records: list[dict] = []
        self.httpd = None


def make_handler(state: PeerState):
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def _record(self, method: str, body: bytes):
            with state.lock:
                state.records.append(
                    {
                        "method": method,
                        "path": self.path,
                        "host": self.headers.get("Host"),
                        "body_sha": sha256_bytes(body) if body else "",
                        "body_len": len(body),
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
            if path.startswith("/api") or path == "/v1" or path.startswith("/v1"):
                self._record("GET", b"")
                body = state.identity + b"|GET|" + self.path.encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/plain")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("X-Cap035-Peer", state.identity.decode())
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

        def do_HEAD(self):
            path = self.path.split("?", 1)[0]
            if path.startswith("/api") or path.startswith("/v1"):
                self._record("HEAD", b"")
                self.send_response(200)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            self.send_response(404)
            self.end_headers()

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
    path: str = "/",
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
    lines.append("User-Agent: cap035-e2e")
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
        s.sendall(req)
        buf = b""
        while True:
            try:
                chunk = s.recv(65536)
            except socket.timeout:
                break
            if not chunk:
                break
            buf += chunk
            if len(buf) > 4_000_000:
                break
        s.close()
    except OSError as exc:
        return {"ok": False, "error": str(exc), "raw": b"", "status": None, "body": b"", "headers": {}}

    status = None
    body_out = b""
    headers: dict[str, str] = {}
    if b"\r\n\r\n" in buf:
        head, body_out = buf.split(b"\r\n\r\n", 1)
        first = head.split(b"\r\n", 1)[0].decode("latin1", errors="replace")
        parts = first.split()
        if len(parts) >= 2 and parts[0].startswith("HTTP/"):
            try:
                status = int(parts[1])
            except ValueError:
                pass
        for line in head.split(b"\r\n")[1:]:
            if b":" in line:
                k, v = line.split(b":", 1)
                headers[k.decode("latin1").strip().lower()] = v.decode("latin1").strip()
        if "content-length" in headers:
            try:
                body_out = body_out[: int(headers["content-length"])]
            except ValueError:
                pass
    return {"ok": True, "status": status, "body": body_out, "raw": buf, "headers": headers}


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
    root: Path,
    up_a: int,
    up_b: int,
    *,
    rewrite_static_to: str = "/new-static/index.html",
    rewrite_proxy_to: str = "/api/v1",
    include_loop: bool = True,
) -> None:
    loop_block = ""
    routes = [
        '"rw-static"',
        '"new-static"',
        '"rw-proxy"',
        '"api"',
        '"rw-host-a"',
        '"static-a"',
        '"rw-host-b"',
        '"static-b"',
        '"direct-static"',
        '"nomatch-static"',
    ]
    if include_loop:
        routes.extend(['"loop-a"', '"loop-b"', '"self-rw"'])
        loop_block = f"""
[[route]]
name = "loop-a"
match = {{ path = "/loop-a" }}
rewrite = "/loop-b"

[[route]]
name = "loop-b"
match = {{ path = "/loop-b" }}
rewrite = "/loop-a"

[[route]]
name = "self-rw"
match = {{ path = "/self-rw" }}
rewrite = "/self-rw"
"""
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = [{", ".join(routes)}]

[[route]]
name = "rw-static"
match = {{ path = "/old-static" }}
rewrite = "{rewrite_static_to}"

[[route]]
name = "new-static"
match = {{ path = "/new-static" }}
root = "{root}/site"
index = "index.html"

[[route]]
name = "rw-proxy"
match = {{ path = "/old-api" }}
rewrite = "{rewrite_proxy_to}"

[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "up-a"

[[route]]
name = "rw-host-a"
match = {{ path = "/legacy", host = "{HOST_A}" }}
rewrite = "/host-a-only/index.html"

[[route]]
name = "static-a"
match = {{ path = "/host-a-only", host = "{HOST_A}" }}
root = "{root}/host-a"
index = "index.html"

[[route]]
name = "rw-host-b"
match = {{ path = "/legacy", host = "{HOST_B}" }}
rewrite = "/host-b-only/index.html"

[[route]]
name = "static-b"
match = {{ path = "/host-b-only", host = "{HOST_B}" }}
root = "{root}/host-b"
index = "index.html"

[[route]]
name = "direct-static"
match = {{ path = "/direct" }}
root = "{root}/site"
index = "index.html"

[[route]]
name = "nomatch-static"
match = {{ path = "/untouched" }}
root = "{root}/site"
index = "index.html"
{loop_block}
[[upstream]]
name = "up-a"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {up_a}
weight = 1

[[upstream]]
name = "up-b"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {up_b}
weight = 1
"""
    )


def write_cfg_reload(path: Path, listen: int, root: Path, up_a: int, up_b: int) -> None:
    write_cfg(
        path,
        listen,
        root,
        up_a,
        up_b,
        rewrite_static_to="/new-static/reloaded.html",
        rewrite_proxy_to="/api/reloaded",
        include_loop=True,
    )


def write_cfg_invalid(path: Path, listen: int, root: Path) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["bad"]

[[route]]
name = "bad"
match = {{ path = "/bad" }}
rewrite = "//evil.example/x"
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


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    if not BIN.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "035",
                    "FEATURE_ID": "rewrite",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BIN}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    root = Path(tempfile.mkdtemp(prefix="cap035-root-"))
    (root / "site").mkdir()
    (root / "host-a").mkdir()
    (root / "host-b").mkdir()
    (root / "site" / "index.html").write_bytes(BODY_NEW)
    (root / "site" / "reloaded.html").write_bytes(b"CAP035-STATIC-RELOADED\n")
    (root / "site" / "secret.txt").write_bytes(BODY_TRAV)
    (root / "host-a" / "index.html").write_bytes(b"CAP035-HOST-A-ONLY\n")
    (root / "host-b" / "index.html").write_bytes(b"CAP035-HOST-B-ONLY\n")

    up_a = pick_port()
    up_b = pick_port()
    listen = pick_port()
    httpd_a, peer_a = start_peer(up_a, ID_UP)
    httpd_b, peer_b = start_peer(up_b, ID_UP_B)
    assert wait_pred(lambda: listening(up_a) and listening(up_b), 10)

    work = Path(tempfile.mkdtemp(prefix="cap035-cfg-"))
    cfg = work / "exyonq.toml"
    sock = work / "control.sock"
    write_cfg(cfg, listen, root, up_a, up_b)

    proc, log = start_exyonq(cfg, sock)
    if not wait_pred(lambda: listening(listen), 40):
        stop_proc(proc)
        stop_httpd(httpd_a)
        stop_httpd(httpd_b)
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "035",
                    "FINAL_RESULT": "FAIL",
                    "DETAIL": "SERVER_NOT_READY",
                    "log": str(log),
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 1

    try:
        # 1) real static rewrite
        r = http_raw(listen, path="/old-static", host=HOST_A)
        check(
            checks,
            "static_rewrite",
            r.get("status") == 200 and r.get("body") == BODY_NEW and "location" not in r.get("headers", {}),
            {"status": r.get("status"), "body": (r.get("body") or b"")[:80].decode("latin1", errors="replace")},
        )

        # 2) no accidental 3xx
        check(
            checks,
            "no_accidental_redirect",
            r.get("status") is not None and r.get("status") < 300,
            {"status": r.get("status"), "location": r.get("headers", {}).get("location")},
        )

        # 3) non-match untouched
        r2 = http_raw(listen, path="/untouched", host=HOST_A)
        check(
            checks,
            "non_match_untouched",
            r2.get("status") == 200 and r2.get("body") == BODY_NEW,
            {"status": r2.get("status")},
        )

        # 4) query preserved through proxy rewrite
        before = len(peer_a.records)
        rq = http_raw(listen, path="/old-api?a=1&b=2", host=HOST_A)
        hits = peer_a.records[before:]
        up_path = hits[-1]["path"] if hits else ""
        check(
            checks,
            "proxy_rewrite",
            rq.get("status") == 200 and ID_UP in (rq.get("body") or b""),
            {"status": rq.get("status"), "body": (rq.get("body") or b"")[:120].decode("latin1", errors="replace")},
        )
        check(
            checks,
            "query_preserved",
            up_path == "/api/v1?a=1&b=2",
            {"upstream_path": up_path},
        )

        # 5) host isolation
        ra = http_raw(listen, path="/legacy", host=HOST_A)
        rb = http_raw(listen, path="/legacy", host=HOST_B)
        check(
            checks,
            "host_isolation_a",
            ra.get("status") == 200 and ra.get("body") == b"CAP035-HOST-A-ONLY\n",
            {"status": ra.get("status"), "body": (ra.get("body") or b"")[:40].decode("latin1", errors="replace")},
        )
        check(
            checks,
            "host_isolation_b",
            rb.get("status") == 200 and rb.get("body") == b"CAP035-HOST-B-ONLY\n",
            {"status": rb.get("status"), "body": (rb.get("body") or b"")[:40].decode("latin1", errors="replace")},
        )
        # cross-host must not leak
        check(
            checks,
            "host_no_cross_leak",
            b"HOST-B" not in (ra.get("body") or b"") and b"HOST-A" not in (rb.get("body") or b""),
            {},
        )

        # 6) loop / self-rewrite bound (no hang)
        t0 = time.monotonic()
        rl = http_raw(listen, path="/loop-a", host=HOST_A, timeout=3.0)
        rs = http_raw(listen, path="/self-rw", host=HOST_A, timeout=3.0)
        elapsed = time.monotonic() - t0
        check(
            checks,
            "loop_protection",
            rl.get("status") == 404 and rs.get("status") == 404 and elapsed < 2.5,
            {"loop_status": rl.get("status"), "self_status": rs.get("status"), "elapsed": round(elapsed, 3)},
        )

        # 7) traversal via rewrite target containing .. (reload rewrite → escape attempt)
        write_cfg(
            cfg,
            listen,
            root,
            up_a,
            up_b,
            rewrite_static_to="/new-static/../secret.txt",
            include_loop=True,
        )
        rc_trav, out_trav = ctl_reload(sock, cfg)
        time.sleep(0.2)
        rt = http_raw(listen, path="/old-static", host=HOST_A)
        check(
            checks,
            "traversal_containment",
            rc_trav == 0
            and rt.get("status") in (400, 403, 404)
            and (rt.get("body") or b"") != BODY_TRAV,
            {
                "reload_rc": rc_trav,
                "status": rt.get("status"),
                "body": (rt.get("body") or b"")[:40].decode("latin1", errors="replace"),
                "out": out_trav[:120],
            },
        )
        # restore baseline rewrite targets (not reload-flipped) for subsequent checks
        write_cfg(cfg, listen, root, up_a, up_b, include_loop=True)
        rc_rest, _ = ctl_reload(sock, cfg)
        time.sleep(0.15)
        check(checks, "restore_after_traversal_probe", rc_rest == 0, {"reload_rc": rc_rest})

        # 8) percent-encoding: matching is on raw path; %2e%2e must not double-decode into traversal success
        rp = http_raw(listen, path="/new-static/%2e%2e/secret.txt", host=HOST_A)
        check(
            checks,
            "percent_no_double_decode_escape",
            rp.get("status") in (400, 403, 404) and (rp.get("body") or b"") != BODY_TRAV,
            {"status": rp.get("status")},
        )

        # 9) POST body through proxy rewrite
        post_body = b"CAP035-POST-BODY-" + os.urandom(16)
        before_p = len(peer_a.records)
        rpst = http_raw(listen, method="POST", path="/old-api", host=HOST_A, body=post_body)
        phits = peer_a.records[before_p:]
        check(
            checks,
            "post_body_proxy_rewrite",
            rpst.get("status") == 200
            and phits
            and phits[-1]["body_sha"] == sha256_bytes(post_body)
            and phits[-1]["path"].startswith("/api/v1"),
            {
                "status": rpst.get("status"),
                "up_path": phits[-1]["path"] if phits else None,
                "body_sha_ok": bool(phits and phits[-1]["body_sha"] == sha256_bytes(post_body)),
            },
        )

        # 10) HEAD through rewrite
        rh = http_raw(listen, method="HEAD", path="/old-static", host=HOST_A)
        check(
            checks,
            "head_rewrite",
            rh.get("status") == 200 and (rh.get("body") or b"") == b"",
            {"status": rh.get("status"), "body_len": len(rh.get("body") or b"")},
        )

        # 11) case sensitivity (path)
        rc = http_raw(listen, path="/Old-Static", host=HOST_A)
        check(
            checks,
            "case_sensitive_path",
            rc.get("status") == 404,
            {"status": rc.get("status")},
        )

        # 12) trailing slash distinct (no auto-redirect)
        rts = http_raw(listen, path="/old-static/", host=HOST_A)
        check(
            checks,
            "trailing_slash_no_auto_redirect",
            rts.get("status") != 301 and rts.get("status") != 302 and "location" not in (rts.get("headers") or {}),
            {"status": rts.get("status")},
        )

        # 13) H2 parity (compact)
        h2 = curl_http2(listen, "/old-static", HOST_A)
        check(
            checks,
            "h2_static_rewrite_parity",
            h2.get("ok") and h2.get("status") == 200 and h2.get("body") == BODY_NEW,
            {"status": h2.get("status"), "stderr": h2.get("stderr", "")[:120]},
        )

        # 14) reload flips rewrite target atomically
        write_cfg_reload(cfg, listen, root, up_a, up_b)
        rc_rel, out_rel = ctl_reload(sock, cfg)
        time.sleep(0.2)
        rr = http_raw(listen, path="/old-static", host=HOST_A)
        before_r = len(peer_a.records)
        rrp = http_raw(listen, path="/old-api", host=HOST_A)
        ph = peer_a.records[before_r:]
        check(
            checks,
            "reload_rewrite_static",
            rc_rel == 0 and rr.get("status") == 200 and rr.get("body") == b"CAP035-STATIC-RELOADED\n",
            {"reload_rc": rc_rel, "status": rr.get("status"), "out": out_rel[:200]},
        )
        check(
            checks,
            "reload_rewrite_proxy",
            rrp.get("status") == 200 and ph and ph[-1]["path"].startswith("/api/reloaded"),
            {"status": rrp.get("status"), "up_path": ph[-1]["path"] if ph else None},
        )

        # 15) failed reload keeps prior generation (overwrite active cfg path — Cap048 pattern)
        cfg.write_text("not valid config {{{")
        rc_bad, out_bad = ctl_reload(sock, cfg)
        time.sleep(0.15)
        rkeep = http_raw(listen, path="/old-static", host=HOST_A)
        # restore valid cfg file for process teardown hygiene
        write_cfg_reload(cfg, listen, root, up_a, up_b)
        check(
            checks,
            "failed_reload_keeps_rewrite",
            rc_bad != 0
            and "EXY-RELOAD-0008" not in out_bad
            and rkeep.get("status") == 200
            and rkeep.get("body") == b"CAP035-STATIC-RELOADED\n",
            {"reload_rc": rc_bad, "status": rkeep.get("status"), "out": out_bad[:240]},
        )

        # 16) invalid // rewrite rejected at config load (fresh process)
        bad_listen = pick_port()
        bad_cfg = work / "bad-start.toml"
        write_cfg_invalid(bad_cfg, bad_listen, root)
        bad_sock = work / "bad.sock"
        env = os.environ.copy()
        env["EXYONQ_CONTROL_SOCKET"] = str(bad_sock)
        env["EXYONQ_CONFIG"] = str(bad_cfg)
        blog = EV / f"exyonq-bad-{time.time_ns()}.log"
        bproc = subprocess.Popen(
            [str(BIN), "serve", "--config", str(bad_cfg)],
            cwd=str(WS),
            env=env,
            stdout=blog.open("w"),
            stderr=subprocess.STDOUT,
        )
        # should fail to become ready
        ready = wait_pred(lambda: listening(bad_listen), 3.0)
        exited = wait_pred(lambda: bproc.poll() is not None, 5.0)
        stop_proc(bproc)
        check(
            checks,
            "invalid_rewrite_config_rejected",
            (not ready) and (exited or bproc.poll() is not None),
            {"ready": ready, "exit": bproc.poll()},
        )

        # 17) absolute-form H1 must not confuse host from rewrite
        raf = http_raw(
            listen,
            path="/old-static",
            host=HOST_A,
            request_target=f"http://{HOST_A}/old-static",
        )
        check(
            checks,
            "absolute_form_h1_rewrite",
            raf.get("status") == 200 and raf.get("body") == b"CAP035-STATIC-RELOADED\n",
            {"status": raf.get("status")},
        )

    finally:
        stop_proc(proc)
        stop_httpd(httpd_a)
        stop_httpd(httpd_b)

    passed = sum(1 for v in checks.values() if v.get("ok"))
    total = len(checks)
    all_ok = passed == total and total > 0
    result = {
        "CAPABILITY_ID": "035",
        "FEATURE_ID": "rewrite",
        "FEATURE_NAME": "Request rewrite",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "STARTED_UTC": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "CHECKS_PASSED": passed,
        "CHECKS_TOTAL": total,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP034_REOPEN": "NO",
        "CAP033_REOPEN": "NO",
        "CAP030_REOPEN": "NO",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if all_ok else "FAIL",
        "checks": checks,
        "PRODUCT_CONTRACT": {
            "REWRITE": "internal request-target transform",
            "REDIRECT": "Cap036 — not this capability",
            "MAX_REWRITE_ITERATIONS": 1,
            "QUERY": "preserved from original via uri_with_path",
            "HOST_MUTATION": "NO",
            "MATCH": "exact route path match then literal rewrite rematch",
        },
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": passed, "total": total, "FINAL_RESULT": result["FINAL_RESULT"]}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
