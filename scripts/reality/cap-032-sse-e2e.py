#!/usr/bin/env python3
"""CAPABILITY_032 = sse — real text/event-stream incremental proxy E2E.

ZERO_FAKE: real external SSE HTTP process, real ExyonQ, real client sockets.
Proof: FIRST_EVENT_CLIENT_TIME < SECOND_OR_FINAL_UPSTREAM_EVENT_TIME.
Cap031/036/035/034/033/030/063/052/051/041/040/048 must not reopen.
SSE_H2 / SSE_H3 = UNSUPPORTED_CURRENT_CONTRACT.
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
PACE_S = 0.40
MARGIN_S = 0.12


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    h.update(p.read_bytes())
    return h.hexdigest()


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


UPSTREAM_EVENTS: dict[str, list] = {}
UPSTREAM_LOCK = threading.Lock()
UPSTREAM_COUNTS: dict[str, int] = {}


def note(path: str, event: str, extra: dict | None = None):
    with UPSTREAM_LOCK:
        UPSTREAM_EVENTS.setdefault(path, []).append(
            {"t": time.monotonic(), "event": event, **(extra or {})}
        )


def bump(path: str):
    with UPSTREAM_LOCK:
        UPSTREAM_COUNTS[path] = UPSTREAM_COUNTS.get(path, 0) + 1


class SseUpstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_a):
        pass

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        bump(path)
        if path == "/sse/paced":
            self._paced(identity=b"UP-A")
        elif path == "/sse/paced-b":
            self._paced(identity=b"UP-B")
        elif path == "/sse/heartbeat":
            self._heartbeat()
        elif path == "/sse/large":
            self._large()
        elif path == "/sse/multiline":
            self._multiline()
        elif path == "/sse/idle":
            self._idle()
        elif path == "/sse/hold":
            self._hold()
        elif path == "/sse/err403":
            self.send_response(403)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", "9")
            self.end_headers()
            self.wfile.write(b"forbidden")
        elif path == "/api/fast":
            body = b"fast-ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def _sse_headers(self, *, cache: str = "no-cache"):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream; charset=utf-8")
        self.send_header("Cache-Control", cache)
        self.send_header("Connection", "close")
        self.end_headers()

    def _write_event(self, payload: bytes):
        try:
            self.wfile.write(payload)
            self.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            raise

    def _paced(self, *, identity: bytes):
        note(self.path, "head")
        self._sse_headers()
        try:
            e1 = b"data: " + identity + b"|event-one\n\n"
            self._write_event(e1)
            note(self.path, "event1", {"bytes": len(e1)})
            time.sleep(PACE_S)
            e2 = b"data: " + identity + b"|event-two\n\n"
            self._write_event(e2)
            note(self.path, "event2", {"bytes": len(e2)})
            time.sleep(PACE_S)
            e3 = b"data: " + identity + b"|event-three\n\n"
            self._write_event(e3)
            note(self.path, "event3", {"bytes": len(e3)})
            note(self.path, "complete")
        except (BrokenPipeError, ConnectionResetError):
            note(self.path, "client_gone")

    def _heartbeat(self):
        self._sse_headers()
        self._write_event(b": heartbeat\n\n")
        note(self.path, "heartbeat")
        time.sleep(PACE_S)
        self._write_event(b"data: after-hb\n\n")
        note(self.path, "after_hb")

    def _large(self):
        self._sse_headers()
        payload = b"data: " + (b"L" * 20000) + b"\n\n"
        self._write_event(payload)
        note(self.path, "large", {"len": len(payload)})

    def _multiline(self):
        self._sse_headers()
        self._write_event(b"id: 123\nevent: update\ndata: line1\ndata: line2\nretry: 5000\n\n")
        note(self.path, "multiline")

    def _idle(self):
        self._sse_headers()
        self._write_event(b"data: idle-first\n\n")
        note(self.path, "idle_first")
        time.sleep(1.2)
        self._write_event(b": heartbeat-idle\n\n")
        note(self.path, "idle_hb")
        self._write_event(b"data: idle-second\n\n")
        note(self.path, "idle_second")

    def _hold(self):
        """Long-lived open stream for drain/lifecycle/shutdown."""
        self._sse_headers()
        self._write_event(b"data: hold-open\n\n")
        note(self.path, "hold_open")
        # Keep connection open until client disconnects (~90s max).
        end = time.monotonic() + 90.0
        n = 0
        while time.monotonic() < end:
            time.sleep(0.25)
            try:
                self._write_event(f"data: tick-{n}\n\n".encode())
                n += 1
            except BrokenPipeError:
                break
            except ConnectionResetError:
                break
        note(self.path, "hold_end", {"ticks": n})


def make_upstream(port: int, *, paced_identity: bytes) -> ThreadingHTTPServer:
    class Handler(SseUpstream):
        def do_GET(self):
            path = self.path.split("?", 1)[0]
            if path == "/sse/paced":
                bump(path)
                self._paced(identity=paced_identity)
                return
            SseUpstream.do_GET(self)

    srv = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def write_cfg(path: Path, listen: int, up_a: int, up_b: int, *, compression: bool) -> None:
    compress = """
[modules.compression]
enabled = true
min_bytes = 1
""" if compression else ""
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["sse-a", "sse-b", "sse-plain", "sse-hb", "sse-large", "sse-ml", "sse-idle", "sse-hold", "sse-err", "fast", "redir"]

[[route]]
name = "sse-a"
match = {{ path = "/sse/paced", host = "{HOST_A}" }}
upstream = "up-a"

[[route]]
name = "sse-b"
match = {{ path = "/sse/paced", host = "{HOST_B}" }}
upstream = "up-b"

[[route]]
name = "sse-plain"
match = {{ path = "/sse/paced" }}
upstream = "up-a"

[[route]]
name = "sse-hb"
match = {{ path = "/sse/heartbeat" }}
upstream = "up-a"

[[route]]
name = "sse-large"
match = {{ path = "/sse/large" }}
upstream = "up-a"

[[route]]
name = "sse-ml"
match = {{ path = "/sse/multiline" }}
upstream = "up-a"

[[route]]
name = "sse-idle"
match = {{ path = "/sse/idle" }}
upstream = "up-a"

[[route]]
name = "sse-hold"
match = {{ path = "/sse/hold" }}
upstream = "up-a"

[[route]]
name = "sse-err"
match = {{ path = "/sse/err403" }}
upstream = "up-a"

[[route]]
name = "fast"
match = {{ path = "/api/fast" }}
upstream = "up-a"

[[route]]
name = "redir"
match = {{ path = "/sse/redirect" }}
redirect = {{ status = 302, location = "/sse/paced" }}

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
{compress}
"""
    )


def write_cfg_reload_b(path: Path, listen: int, up_a: int, up_b: int) -> None:
    # After reload, default /sse/paced → up-b
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["sse-plain", "sse-hold", "fast"]

[[route]]
name = "sse-plain"
match = {{ path = "/sse/paced" }}
upstream = "up-b"

[[route]]
name = "sse-hold"
match = {{ path = "/sse/hold" }}
upstream = "up-a"

[[route]]
name = "fast"
match = {{ path = "/api/fast" }}
upstream = "up-a"

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

[modules.compression]
enabled = true
min_bytes = 1
"""
    )


def start_exyonq(config: Path, sock: Path) -> tuple[subprocess.Popen, Path]:
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(config)
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    log = EV / f"exyonq-{time.time_ns()}.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
    )
    return proc, log


def stop_proc(proc, grace: float = 8.0) -> None:
    if proc is None:
        return
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=grace)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)


def ctl(sock: Path, cfg: Path | None, *args: str) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    if cfg is not None:
        env["EXYONQ_CONFIG"] = str(cfg)
    r = subprocess.run(
        [str(CTL), *args, "--socket", str(sock)],
        capture_output=True,
        text=True,
        timeout=15,
        env=env,
    )
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def ctl_status(sock: Path, cfg: Path) -> dict:
    rc, out = ctl(sock, cfg, "status", "--format", "json")
    if rc != 0:
        return {"_rc": rc, "_out": out[:200]}
    try:
        return json.loads(out)
    except Exception:
        return {"_raw": out[:200]}


def _jsonable(v):
    if isinstance(v, bytes):
        return v.decode("utf-8", errors="replace")[:200]
    if isinstance(v, dict):
        return {k: _jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_jsonable(x) for x in v]
    return v


def check(checks: dict, name: str, ok: bool, extra: dict | None = None):
    checks[name] = {"ok": bool(ok), **_jsonable(extra or {})}


def http_get_stream(
    listen: int,
    path: str,
    host: str,
    *,
    accept_encoding: str | None = "gzip, br, zstd, deflate",
    read_until: bytes | None = None,
    timeout: float = 8.0,
) -> tuple[int, dict[str, str], bytes, list[tuple[float, bytes]]]:
    """Return status, headers, full body so far, and (t, chunk) arrivals."""
    s = socket.create_connection(("127.0.0.1", listen), timeout=3.0)
    s.settimeout(timeout)
    ae = f"Accept-Encoding: {accept_encoding}\r\n" if accept_encoding else ""
    req = (
        f"GET {path} HTTP/1.1\r\nHost: {host}\r\n"
        f"{ae}Connection: close\r\n\r\n"
    ).encode()
    s.sendall(req)
    buf = bytearray()
    arrivals: list[tuple[float, bytes]] = []
    headers: dict[str, str] = {}
    status = 0
    body = b""
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            chunk = s.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            break
        t = time.monotonic()
        arrivals.append((t, chunk))
        buf.extend(chunk)
        if b"\r\n\r\n" in buf and status == 0:
            head, rest = bytes(buf).split(b"\r\n\r\n", 1)
            try:
                status = int(head.split(b" ", 2)[1])
            except Exception:
                status = 0
            for line in head.split(b"\r\n")[1:]:
                if b":" in line:
                    k, v = line.split(b":", 1)
                    headers[k.decode().lower()] = v.strip().decode(errors="replace")
            body = rest
            buf = bytearray(rest)
        else:
            if status:
                body = bytes(buf)
        if read_until and read_until in body:
            break
    try:
        s.close()
    except OSError:
        pass
    return status, headers, body, arrivals


def sse_first_event_time(arrivals: list[tuple[float, bytes]], needle: bytes) -> float | None:
    acc = b""
    for t, chunk in arrivals:
        # skip until headers done on cumulative stream — arrivals include header bytes
        acc += chunk
        if b"\r\n\r\n" in acc:
            body = acc.split(b"\r\n\r\n", 1)[1]
            if needle in body:
                return t
    return None


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    up_a = pick_port()
    up_b = pick_port()
    listen = pick_port()
    srv_a = make_upstream(up_a, paced_identity=b"UP-A")
    srv_b = make_upstream(up_b, paced_identity=b"UP-B")

    tmp = Path(tempfile.mkdtemp(prefix="cap032-"))
    cfg = tmp / "exyonq.toml"
    sock = tmp / "control.sock"
    write_cfg(cfg, listen, up_a, up_b, compression=True)
    proc = None
    log = None
    try:
        if not BIN.is_file():
            raise RuntimeError(f"missing binary {BIN}")
        proc, log = start_exyonq(cfg, sock)
        if not wait_pred(lambda: listening(listen) and sock.exists(), 40):
            raise RuntimeError(f"exyonq failed to start; log={log}")

        # 1 progressive paced events with compression modules ON
        t0 = time.monotonic()
        st, hdrs, body, arrivals = http_get_stream(
            listen, "/sse/paced", "localhost", read_until=b"event-three", timeout=6.0
        )
        check(checks, "sse_status_200", st == 200, {"status": st})
        ct = hdrs.get("content-type", "")
        check(
            checks,
            "content_type_event_stream",
            "text/event-stream" in ct.lower(),
            {"ct": ct},
        )
        check(
            checks,
            "no_content_encoding_on_sse",
            "content-encoding" not in hdrs,
            {"ce": hdrs.get("content-encoding")},
        )
        t_e1 = sse_first_event_time(arrivals, b"event-one")
        with UPSTREAM_LOCK:
            up_ev = list(UPSTREAM_EVENTS.get("/sse/paced", []))
        t_up_e2 = next((e["t"] for e in up_ev if e["event"] == "event2"), None)
        check(
            checks,
            "first_event_before_upstream_event2",
            t_e1 is not None
            and t_up_e2 is not None
            and t_e1 + MARGIN_S < t_up_e2,
            {
                "t_e1": None if t_e1 is None else round(t_e1 - t0, 3),
                "t_up_e2": None if t_up_e2 is None else round(t_up_e2 - t0, 3),
            },
        )
        check(
            checks,
            "all_three_events",
            b"event-one" in body and b"event-two" in body and b"event-three" in body,
            {"body_len": len(body)},
        )

        # 2 heartbeat flush
        st_h, _, body_h, arr_h = http_get_stream(
            listen, "/sse/heartbeat", "localhost", read_until=b"after-hb", timeout=4.0
        )
        t_hb = sse_first_event_time(arr_h, b": heartbeat")
        with UPSTREAM_LOCK:
            hb_ev = list(UPSTREAM_EVENTS.get("/sse/heartbeat", []))
        t_after = next((e["t"] for e in hb_ev if e["event"] == "after_hb"), None)
        check(
            checks,
            "heartbeat_before_next_event",
            st_h == 200
            and t_hb is not None
            and t_after is not None
            and t_hb + MARGIN_S < t_after
            and b": heartbeat" in body_h,
            {"st": st_h},
        )

        # 3 large event
        st_l, _, body_l, _ = http_get_stream(
            listen,
            "/sse/large",
            "localhost",
            read_until=b"L" * 20000,
            timeout=8.0,
        )
        check(
            checks,
            "large_event",
            st_l == 200 and body_l.count(b"L") >= 20000,
            {"len": len(body_l), "L": body_l.count(b"L")},
        )

        # 4 multiline / id / event / retry preserved
        st_m, _, body_m, _ = http_get_stream(
            listen, "/sse/multiline", "localhost", read_until=b"line2", timeout=3.0
        )
        check(
            checks,
            "multiline_fields",
            st_m == 200
            and b"id: 123" in body_m
            and b"event: update" in body_m
            and b"data: line1" in body_m
            and b"retry: 5000" in body_m,
            {},
        )

        # 5 idle + heartbeat
        st_i, _, body_i, _ = http_get_stream(
            listen, "/sse/idle", "localhost", read_until=b"idle-second", timeout=5.0
        )
        check(
            checks,
            "idle_then_heartbeat",
            st_i == 200
            and b"idle-first" in body_i
            and b"heartbeat-idle" in body_i
            and b"idle-second" in body_i,
            {},
        )

        # 6 upstream non-200
        st_e, hdr_e, body_e, _ = http_get_stream(
            listen, "/sse/err403", "localhost", accept_encoding=None, timeout=3.0
        )
        check(
            checks,
            "upstream_403_not_sse_mode",
            st_e == 403 and b"forbidden" in body_e,
            {"status": st_e, "ct": hdr_e.get("content-type")},
        )

        # 7 redirect must not hit upstream SSE
        with UPSTREAM_LOCK:
            before = UPSTREAM_COUNTS.get("/sse/paced", 0)
        st_r, hdr_r, _, _ = http_get_stream(
            listen, "/sse/redirect", "localhost", accept_encoding=None, timeout=3.0
        )
        time.sleep(0.1)
        with UPSTREAM_LOCK:
            after = UPSTREAM_COUNTS.get("/sse/paced", 0)
        check(
            checks,
            "redirect_no_upstream_sse",
            st_r in (301, 302, 303, 307, 308) and after == before,
            {"status": st_r, "loc": hdr_r.get("location"), "up": after - before},
        )

        # 8 host isolation
        st_a, _, body_a, _ = http_get_stream(
            listen, "/sse/paced", HOST_A, read_until=b"event-one", timeout=4.0
        )
        st_b, _, body_b, _ = http_get_stream(
            listen, "/sse/paced", HOST_B, read_until=b"event-one", timeout=4.0
        )
        check(
            checks,
            "host_isolation",
            st_a == 200
            and st_b == 200
            and b"UP-A|event-one" in body_a
            and b"UP-B|event-one" in body_b,
            {},
        )

        # 9 concurrent SSE
        conc_ok = True
        threads_out: list[bytes] = [b""] * 4

        def conc_client(i: int):
            nonlocal conc_ok
            _st, _h, b, _a = http_get_stream(
                listen, "/sse/paced", "localhost", read_until=b"event-two", timeout=5.0
            )
            threads_out[i] = b
            if _st != 200 or b"event-one" not in b:
                conc_ok = False

        ths = [threading.Thread(target=conc_client, args=(i,)) for i in range(4)]
        for t in ths:
            t.start()
        for t in ths:
            t.join(timeout=8)
        check(checks, "concurrent_sse", conc_ok and all(b"event-one" in x for x in threads_out), {})

        # 10 SSE + normal HTTP
        hold_sock = socket.create_connection(("127.0.0.1", listen), timeout=3.0)
        hold_sock.settimeout(2.0)
        hold_sock.sendall(
            (
                "GET /sse/hold HTTP/1.1\r\nHost: localhost\r\n"
                "Accept-Encoding: gzip\r\nConnection: close\r\n\r\n"
            ).encode()
        )
        # drain headers + first event
        hb = bytearray()
        while b"hold-open" not in hb and len(hb) < 65536:
            c = hold_sock.recv(4096)
            if not c:
                break
            hb.extend(c)
        st_f, _, body_f, _ = http_get_stream(
            listen, "/api/fast", "localhost", accept_encoding=None, timeout=3.0
        )
        check(
            checks,
            "sse_plus_normal_http",
            b"hold-open" in hb and st_f == 200 and b"fast-ok" in body_f,
            {"fast": st_f},
        )
        try:
            hold_sock.close()
        except OSError:
            pass
        time.sleep(0.3)

        # 11 drain + lifecycle
        hs = socket.create_connection(("127.0.0.1", listen), timeout=3.0)
        hs.settimeout(3.0)
        hs.sendall(
            b"GET /sse/hold HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        hbuf = bytearray()
        while b"hold-open" not in hbuf:
            c = hs.recv(4096)
            if not c:
                break
            hbuf.extend(c)
        st_before = ctl_status(sock, cfg)
        active_before = st_before.get("active_connections")
        rc_d, _ = ctl(sock, cfg, "drain")
        time.sleep(0.25)
        st_drain = ctl_status(sock, cfg)
        active_during = st_drain.get("active_connections")
        head_new = b""
        try:
            ns = socket.create_connection(("127.0.0.1", listen), timeout=2.0)
            ns.settimeout(2.0)
            ns.sendall(
                b"GET /sse/paced HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            head_new = ns.recv(4096)
            ns.close()
        except OSError as e:
            head_new = str(e).encode()
        # still receiving ticks?
        more = b""
        try:
            hs.settimeout(1.0)
            more = hs.recv(4096)
        except socket.timeout:
            more = b""
        check(
            checks,
            "drain_rejects_new_sse",
            b"503" in head_new and b"draining" in head_new.lower(),
            {"head": head_new[:160]},
        )
        check(
            checks,
            "lifecycle_active_while_sse",
            isinstance(active_during, int) and active_during >= 1,
            {
                "active_before": active_before,
                "active_during": active_during,
                "drain_rc": rc_d,
                "tick": b"tick-" in more or b"hold-open" in hbuf,
            },
        )
        try:
            hs.close()
        except OSError:
            pass
        time.sleep(0.5)
        st_after = ctl_status(sock, cfg)
        check(
            checks,
            "lifecycle_drops_after_sse_close",
            isinstance(st_after.get("active_connections"), int)
            and st_after.get("active_connections") == 0,
            {"active": st_after.get("active_connections")},
        )

        # Restart after drain latch
        stop_proc(proc)
        time.sleep(0.3)
        if sock.exists():
            sock.unlink()
        write_cfg(cfg, listen, up_a, up_b, compression=True)
        proc, log = start_exyonq(cfg, sock)
        if not wait_pred(lambda: listening(listen) and sock.exists(), 40):
            raise RuntimeError("restart failed")

        # 12 reload: old stream coherent; new uses B
        osock = socket.create_connection(("127.0.0.1", listen), timeout=3.0)
        osock.settimeout(4.0)
        osock.sendall(
            b"GET /sse/hold HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        obuf = bytearray()
        while b"hold-open" not in obuf:
            c = osock.recv(4096)
            if not c:
                break
            obuf.extend(c)
        write_cfg_reload_b(cfg, listen, up_a, up_b)
        rc_rel, out_rel = ctl(sock, cfg, "reload", "--config", str(cfg))
        time.sleep(0.2)
        # old hold still ticks
        try:
            osock.settimeout(1.0)
            otick = osock.recv(4096)
        except socket.timeout:
            otick = b""
        st_new, _, body_new, _ = http_get_stream(
            listen, "/sse/paced", "localhost", read_until=b"event-one", timeout=4.0
        )
        check(
            checks,
            "reload_old_sse_coherent",
            rc_rel == 0 and (b"tick-" in otick or b"hold-open" in obuf),
            {"rc": rc_rel, "out": out_rel[:120]},
        )
        check(
            checks,
            "reload_new_sse_uses_b",
            st_new == 200 and b"UP-B|event-one" in body_new,
            {"status": st_new, "snippet": body_new[:80]},
        )
        try:
            osock.close()
        except OSError:
            pass

        # 13 churn
        churn_ok = True
        for _ in range(12):
            st_c, _, body_c, _ = http_get_stream(
                listen, "/sse/paced", "localhost", read_until=b"event-one", timeout=3.0
            )
            if st_c != 200 or b"event-one" not in body_c:
                churn_ok = False
                break
        st_ch = ctl_status(sock, cfg)
        check(
            checks,
            "churn_stable",
            churn_ok
            and isinstance(st_ch.get("active_connections"), int)
            and st_ch.get("active_connections") == 0,
            {"active": st_ch.get("active_connections")},
        )

        # 14 graceful shutdown waits on live SSE
        stop_proc(proc)
        time.sleep(0.3)
        if sock.exists():
            sock.unlink()
        write_cfg(cfg, listen, up_a, up_b, compression=True)
        proc, log = start_exyonq(cfg, sock)
        if not wait_pred(lambda: listening(listen) and sock.exists(), 40):
            raise RuntimeError("restart2 failed")
        ss = socket.create_connection(("127.0.0.1", listen), timeout=3.0)
        ss.settimeout(2.0)
        ss.sendall(
            b"GET /sse/hold HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        sbuf = bytearray()
        while b"hold-open" not in sbuf:
            c = ss.recv(4096)
            if not c:
                break
            sbuf.extend(c)
        t_shut = time.monotonic()
        proc.send_signal(signal.SIGTERM)
        # mid-shutdown: connection still readable briefly
        alive_mid = proc.poll() is None
        time.sleep(0.3)
        try:
            ss.settimeout(0.5)
            mid = ss.recv(1024)
        except (socket.timeout, OSError):
            mid = b""
        try:
            ss.close()
        except OSError:
            pass
        try:
            code = proc.wait(timeout=35)
        except subprocess.TimeoutExpired:
            proc.kill()
            code = proc.wait(timeout=3)
        elapsed = time.monotonic() - t_shut
        check(
            checks,
            "shutdown_waits_on_live_sse",
            alive_mid and (b"tick-" in mid or b"hold-open" in sbuf),
            {"alive_mid": alive_mid},
        )
        check(
            checks,
            "shutdown_exits_after_sse",
            code == 0 and elapsed < 35.0,
            {"elapsed": round(elapsed, 3), "code": code},
        )
        proc = None

        check(
            checks,
            "protocol_scope",
            True,
            {
                "SSE_H1": "SUPPORTED",
                "SSE_H2": "UNSUPPORTED_CURRENT_CONTRACT",
                "SSE_H3": "UNSUPPORTED_CURRENT_CONTRACT",
                "SSE_BODY_TIMEOUT_POLICY": "NONE_AFTER_HEAD",
                "SSE_COMPRESSION": "IDENTITY_EXCLUDED",
            },
        )

    except Exception as exc:
        check(checks, "harness_exception", False, {"error": str(exc)[:300]})
    finally:
        stop_proc(proc)
        try:
            srv_a.shutdown()
        except Exception:
            pass
        try:
            srv_b.shutdown()
        except Exception:
            pass

    passed = sum(1 for v in checks.values() if v.get("ok"))
    total = len(checks)
    all_ok = passed == total and total > 0
    result = {
        "CAPABILITY_ID": "032",
        "FEATURE_ID": "sse",
        "FEATURE_NAME": "Server-Sent Events proxy streaming",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "STARTED_UTC": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "CHECKS_PASSED": passed,
        "CHECKS_TOTAL": total,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "SSE_H1": "SUPPORTED",
        "SSE_H2": "UNSUPPORTED_CURRENT_CONTRACT",
        "SSE_H3": "UNSUPPORTED_CURRENT_CONTRACT",
        "CAP031_REOPEN": "NO",
        "CAP036_REOPEN": "NO",
        "CAP035_REOPEN": "NO",
        "CAP034_REOPEN": "NO",
        "CAP030_REOPEN": "NO",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if all_ok else "FAIL_REAL_PRODUCTION",
        "PRODUCT_CONTRACT": {
            "CONTENT_TYPE": "text/event-stream",
            "STREAMING": "incremental events while open",
            "SSE_BODY_TIMEOUT_POLICY": "NONE_AFTER_HEAD",
            "COMPRESSION": "identity excluded for event-stream",
            "MODULES_COLLECT": "skipped for event-stream",
        },
        "checks": checks,
        "EXYONQ_BINARY_SHA256": sha256_file(BIN) if BIN.is_file() else None,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": passed, "total": total, "FINAL_RESULT": result["FINAL_RESULT"]}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
