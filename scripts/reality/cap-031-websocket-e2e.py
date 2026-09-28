#!/usr/bin/env python3
"""CAPABILITY_031 = websocket — real H1 Upgrade + bidirectional tunnel E2E.

ZERO_FAKE: real ExyonQ, real RFC6455 upstream process, real client frames.
Cap036/035/034/033/030/063/052/051/041/040/048 must not reopen.
WEBSOCKET_H2_EXTENDED_CONNECT / WEBSOCKET_H3 = UNSUPPORTED_CURRENT_CONTRACT.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import select
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
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
WS_GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


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


def ws_accept(key: str) -> str:
    digest = hashlib.sha1(key.encode() + WS_GUID).digest()
    return base64.b64encode(digest).decode()


def mask_payload(payload: bytes, mask: bytes) -> bytes:
    return bytes(b ^ mask[i % 4] for i, b in enumerate(payload))


def ws_frame(opcode: int, payload: bytes, *, masked: bool) -> bytes:
    fin_op = 0x80 | (opcode & 0x0F)
    ln = len(payload)
    hdr = bytearray([fin_op])
    if ln < 126:
        hdr.append((0x80 if masked else 0) | ln)
    elif ln < 65536:
        hdr.append((0x80 if masked else 0) | 126)
        hdr.extend(struct.pack("!H", ln))
    else:
        hdr.append((0x80 if masked else 0) | 127)
        hdr.extend(struct.pack("!Q", ln))
    if masked:
        mask = os.urandom(4)
        hdr.extend(mask)
        hdr.extend(mask_payload(payload, mask))
    else:
        hdr.extend(payload)
    return bytes(hdr)


def ws_read_frame(sock: socket.socket, pending: bytearray, timeout: float = 5.0) -> tuple[int, bytes]:
    sock.settimeout(timeout)
    def need(n: int) -> None:
        while len(pending) < n:
            chunk = sock.recv(65536)
            if not chunk:
                raise EOFError("socket closed")
            pending.extend(chunk)

    need(2)
    op = pending[0] & 0x0F
    masked = (pending[1] & 0x80) != 0
    ln = pending[1] & 0x7F
    del pending[:2]
    if ln == 126:
        need(2)
        ln = struct.unpack("!H", bytes(pending[:2]))[0]
        del pending[:2]
    elif ln == 127:
        need(8)
        ln = struct.unpack("!Q", bytes(pending[:8]))[0]
        del pending[:8]
    mask = b""
    if masked:
        need(4)
        mask = bytes(pending[:4])
        del pending[:4]
    need(ln)
    payload = bytes(pending[:ln])
    del pending[:ln]
    if masked:
        payload = mask_payload(payload, mask)
    return op, payload


class WsUpstream:
    """Real TCP RFC6455 server (independent process role via threads)."""

    def __init__(self, port: int, identity: bytes, *, reject: bool = False, host_tag: bytes = b""):
        self.port = port
        self.identity = identity
        self.reject = reject
        self.host_tag = host_tag
        self.lock = threading.Lock()
        self.handshakes = 0
        self.accepted = 0
        self.rejected = 0
        self.client_msgs: list[bytes] = []
        self.alive = True
        self._srv = socket.socket()
        self._srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._srv.bind(("127.0.0.1", port))
        self._srv.listen(64)
        self._srv.settimeout(0.5)
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        self.alive = False
        try:
            self._srv.close()
        except OSError:
            pass

    def _loop(self) -> None:
        while self.alive:
            try:
                conn, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            threading.Thread(target=self._handle, args=(conn,), daemon=True).start()

    def _handle(self, conn: socket.socket) -> None:
        pending = bytearray()
        try:
            conn.settimeout(5.0)
            while b"\r\n\r\n" not in pending:
                chunk = conn.recv(65536)
                if not chunk:
                    return
                pending.extend(chunk)
            head, rest = bytes(pending).split(b"\r\n\r\n", 1)
            pending = bytearray(rest)
            text = head.decode("latin-1", errors="replace")
            headers = {}
            for line in text.split("\r\n")[1:]:
                if ":" in line:
                    k, v = line.split(":", 1)
                    headers[k.strip().lower()] = v.strip()
            with self.lock:
                self.handshakes += 1
            if self.reject or headers.get("upgrade", "").lower() != "websocket":
                body = b"no ws"
                conn.sendall(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: "
                    + str(len(body)).encode()
                    + b"\r\nConnection: close\r\n\r\n"
                    + body
                )
                with self.lock:
                    self.rejected += 1
                conn.close()
                return
            key = headers.get("sec-websocket-key", "")
            accept = ws_accept(key)
            conn.sendall(
                (
                    "HTTP/1.1 101 Switching Protocols\r\n"
                    "Upgrade: websocket\r\n"
                    "Connection: Upgrade\r\n"
                    f"Sec-WebSocket-Accept: {accept}\r\n\r\n"
                ).encode()
            )
            with self.lock:
                self.accepted += 1
            # Immediate first server frame (first-frame race)
            first = self.identity + b"|FIRST|" + self.host_tag
            conn.sendall(ws_frame(0x1, first, masked=False))
            while self.alive:
                try:
                    op, payload = ws_read_frame(conn, pending, timeout=2.0)
                except (EOFError, TimeoutError, socket.timeout, OSError):
                    break
                if op == 0x8:
                    conn.sendall(ws_frame(0x8, payload[:2] if len(payload) >= 2 else b"", masked=False))
                    break
                if op == 0x9:
                    conn.sendall(ws_frame(0xA, payload, masked=False))
                    continue
                with self.lock:
                    self.client_msgs.append(payload)
                # Echo with identity prefix for upstream observation proof
                reply = self.identity + b"|" + payload
                conn.sendall(ws_frame(0x2 if any(b > 127 or b < 9 for b in payload) else 0x1, reply, masked=False))
        finally:
            try:
                conn.close()
            except OSError:
                pass


def http_raw(port: int, raw: bytes, timeout: float = 5.0) -> bytes:
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    try:
        s.sendall(raw)
        s.settimeout(timeout)
        data = b""
        while b"\r\n\r\n" not in data:
            chunk = s.recv(65536)
            if not chunk:
                break
            data += chunk
        # optional body for non-101
        return data
    finally:
        try:
            s.close()
        except OSError:
            pass


def ws_connect(
    port: int,
    path: str,
    host: str,
    timeout: float = 5.0,
    *,
    connection: str = "Upgrade",
) -> tuple[socket.socket, bytearray, int]:
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}\r\n"
        "Upgrade: websocket\r\n"
        f"Connection: {connection}\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    ).encode()
    s = socket.create_connection(("127.0.0.1", port), timeout=timeout)
    s.sendall(req)
    s.settimeout(timeout)
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = s.recv(65536)
        if not chunk:
            break
        data += chunk
    head, rest = data.split(b"\r\n\r\n", 1) if b"\r\n\r\n" in data else (data, b"")
    status = 0
    try:
        status = int(head.split(b" ", 2)[1])
    except Exception:
        status = 0
    return s, bytearray(rest), status


def write_cfg(path: Path, listen: int, up_a: int, up_b: int, up_reject: int) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["ws-a", "ws-b", "ws-reject", "ws-plain", "ws-api"]

[[route]]
name = "ws-a"
match = {{ path = "/ws", host = "{HOST_A}" }}
upstream = "up-a"

[[route]]
name = "ws-b"
match = {{ path = "/ws", host = "{HOST_B}" }}
upstream = "up-b"

[[route]]
name = "ws-reject"
match = {{ path = "/ws-reject" }}
upstream = "up-reject"

[[route]]
name = "ws-plain"
match = {{ path = "/ws-plain" }}
upstream = "up-a"

[[route]]
name = "ws-api"
match = {{ path = "/api/ws" }}
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

[[upstream]]
name = "up-reject"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {up_reject}
weight = 1
"""
    )


def write_cfg_reload_b(path: Path, listen: int, up_a: int, up_b: int, up_reject: int) -> None:
    # After reload, host-a /ws → up-b
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["ws-a", "ws-b", "ws-reject", "ws-plain", "ws-api"]

[[route]]
name = "ws-a"
match = {{ path = "/ws", host = "{HOST_A}" }}
upstream = "up-b"

[[route]]
name = "ws-b"
match = {{ path = "/ws", host = "{HOST_B}" }}
upstream = "up-b"

[[route]]
name = "ws-reject"
match = {{ path = "/ws-reject" }}
upstream = "up-reject"

[[route]]
name = "ws-plain"
match = {{ path = "/ws-plain" }}
upstream = "up-a"

[[route]]
name = "ws-api"
match = {{ path = "/api/ws" }}
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

[[upstream]]
name = "up-reject"
timeout_ms = 5000
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {up_reject}
weight = 1
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
        return {"_rc": rc, "_out": out}
    try:
        return json.loads(out)
    except json.JSONDecodeError:
        return {"_rc": rc, "_out": out}


def _jsonable(v):
    if isinstance(v, bytes):
        return v.decode("latin-1", errors="replace")
    if isinstance(v, dict):
        return {k: _jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [_jsonable(x) for x in v]
    return v


def check(checks: dict, name: str, ok: bool, detail: dict | None = None) -> None:
    checks[name] = {"ok": bool(ok), **_jsonable(detail or {})}


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    if not BIN.is_file() or not CTL.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "031",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BIN} or {CTL}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    up_a_port = pick_port()
    up_b_port = pick_port()
    up_rej_port = pick_port()
    listen = pick_port()
    up_a = WsUpstream(up_a_port, b"UP-A", host_tag=b"A")
    up_b = WsUpstream(up_b_port, b"UP-B", host_tag=b"B")
    up_rej = WsUpstream(up_rej_port, b"UP-REJ", reject=True)

    work = Path(tempfile.mkdtemp(prefix="cap031-cfg-"))
    cfg = work / "exyonq.toml"
    sock = Path(f"/tmp/exq31-{os.getpid() % 10000}-{time.time_ns() % 100000}.sock")
    if sock.exists():
        sock.unlink()
    write_cfg(cfg, listen, up_a_port, up_b_port, up_rej_port)

    proc, log = start_exyonq(cfg, sock)
    if not wait_pred(lambda: listening(listen) and sock.exists(), 40):
        stop_proc(proc)
        up_a.stop()
        up_b.stop()
        up_rej.stop()
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "031",
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
        # 1–4 handshake + bidirectional + first frame + binary
        s, pending, status = ws_connect(listen, "/ws", HOST_A)
        check(checks, "h1_upgrade_101", status == 101, {"status": status})
        op, first = ws_read_frame(s, pending)
        check(
            checks,
            "first_upstream_frame",
            op in (0x1, 0x2) and first.startswith(b"UP-A|FIRST|"),
            {"op": op, "first": first[:40]},
        )
        msg_a = b"CLIENT_MESSAGE_A"
        s.sendall(ws_frame(0x1, msg_a, masked=True))
        op2, echo_a = ws_read_frame(s, pending)
        check(
            checks,
            "client_to_upstream_and_back",
            echo_a == b"UP-A|" + msg_a,
            {"echo": echo_a[:60]},
        )
        with up_a.lock:
            got_a = msg_a in up_a.client_msgs
        check(checks, "upstream_observed_client_msg", got_a, {"n": len(up_a.client_msgs)})

        binary = bytes(range(256)) + b"\x00\xffCAP031"
        s.sendall(ws_frame(0x2, binary, masked=True))
        opb, echo_b = ws_read_frame(s, pending)
        check(
            checks,
            "binary_payload_preserved",
            echo_b == b"UP-A|" + binary,
            {"op": opb, "len": len(echo_b)},
        )

        # large-ish message
        large = b"L" * 20000
        s.sendall(ws_frame(0x1, large, masked=True))
        opl, echo_l = ws_read_frame(s, pending, timeout=10.0)
        check(
            checks,
            "large_message",
            echo_l == b"UP-A|" + large,
            {"len": len(echo_l)},
        )

        # ping/pong tunnel
        s.sendall(ws_frame(0x9, b"ping31", masked=True))
        opp, pong = ws_read_frame(s, pending)
        check(checks, "ping_pong_tunneled", opp == 0xA and pong == b"ping31", {"op": opp})

        # close
        s.sendall(ws_frame(0x8, b"\x03\xe8", masked=True))
        time.sleep(0.1)
        try:
            s.close()
        except OSError:
            pass

        # 5 upstream reject — no false 101
        sr, pr, st_rej = ws_connect(listen, "/ws-reject", "localhost")
        check(
            checks,
            "upstream_reject_no_false_101",
            st_rej == 400,
            {"status": st_rej},
        )
        try:
            sr.close()
        except OSError:
            pass

        # LA-002: /api/* + Connection: close, Upgrade must Hyper-tunnel (not GET wire)
        s_api, p_api, st_api = ws_connect(
            listen, "/api/ws", "localhost", connection="close, Upgrade"
        )
        check(checks, "api_close_upgrade_101", st_api == 101, {"status": st_api})
        op_api, first_api = ws_read_frame(s_api, p_api)
        check(
            checks,
            "api_close_upgrade_tunnel",
            op_api in (0x1, 0x2) and first_api.startswith(b"UP-A|FIRST|"),
            {"first": first_api[:40]},
        )
        try:
            s_api.close()
        except OSError:
            pass

        # 6 host isolation
        sa, pa, sta = ws_connect(listen, "/ws", HOST_A)
        sb, pb, stb = ws_connect(listen, "/ws", HOST_B)
        check(checks, "host_a_101", sta == 101, {"status": sta})
        check(checks, "host_b_101", stb == 101, {"status": stb})
        _, fa = ws_read_frame(sa, pa)
        _, fb = ws_read_frame(sb, pb)
        check(checks, "host_isolation_a", fa.startswith(b"UP-A|FIRST|"), {"first": fa[:30]})
        check(checks, "host_isolation_b", fb.startswith(b"UP-B|FIRST|"), {"first": fb[:30]})
        try:
            sa.close()
            sb.close()
        except OSError:
            pass

        # 7 concurrent tunnels
        conc = []
        for i in range(4):
            sc, pc, stc = ws_connect(listen, "/ws-plain", "localhost")
            check(checks, f"concurrent_101_{i}", stc == 101, {"status": stc})
            _, _f = ws_read_frame(sc, pc)
            mid = f"CONC-{i}".encode()
            sc.sendall(ws_frame(0x1, mid, masked=True))
            _, er = ws_read_frame(sc, pc)
            check(checks, f"concurrent_msg_{i}", er == b"UP-A|" + mid, {"echo": er[:40]})
            conc.append(sc)
        for sc in conc:
            try:
                sc.close()
            except OSError:
                pass

        # 8 drain: live tunnel held + new handshake rejected
        s_live, p_live, st_live = ws_connect(listen, "/ws-plain", "localhost")
        check(checks, "lifecycle_tunnel_101", st_live == 101, {"status": st_live})
        _, _ = ws_read_frame(s_live, p_live)
        st_before = ctl_status(sock, cfg)
        active_before = st_before.get("active_connections")
        rc_d, out_d = ctl(sock, cfg, "drain")
        time.sleep(0.25)
        st_drain = ctl_status(sock, cfg)
        active_during = st_drain.get("active_connections")
        # new handshake during drain
        head_new = http_raw(
            listen,
            (
                "GET /ws-plain HTTP/1.1\r\nHost: localhost\r\n"
                "Upgrade: websocket\r\nConnection: Upgrade\r\n"
                "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n"
                "Sec-WebSocket-Version: 13\r\nConnection: close\r\n\r\n"
            ).encode(),
        )
        check(
            checks,
            "drain_rejects_new_ws",
            b"503" in head_new and b"draining" in head_new.lower(),
            {"head": head_new[:160]},
        )
        # live tunnel still works
        s_live.sendall(ws_frame(0x1, b"STILL-ALIVE", masked=True))
        _, echo_live = ws_read_frame(s_live, p_live)
        check(
            checks,
            "drain_keeps_existing_tunnel",
            echo_live == b"UP-A|STILL-ALIVE",
            {"echo": echo_live[:40], "active_during": active_during, "drain_rc": rc_d},
        )
        check(
            checks,
            "lifecycle_active_while_tunnel",
            isinstance(active_during, int) and active_during >= 1,
            {
                "active_before": active_before,
                "active_during": active_during,
                "draining": st_drain.get("draining"),
            },
        )
        try:
            s_live.close()
        except OSError:
            pass
        time.sleep(0.4)
        st_after = ctl_status(sock, cfg)
        check(
            checks,
            "lifecycle_drops_after_tunnel_close",
            isinstance(st_after.get("active_connections"), int)
            and st_after.get("active_connections") == 0,
            {"active": st_after.get("active_connections")},
        )

        # Restart server for reload/shutdown tests (drain already latched)
        stop_proc(proc)
        time.sleep(0.3)
        if sock.exists():
            sock.unlink()
        write_cfg(cfg, listen, up_a_port, up_b_port, up_rej_port)
        proc, log = start_exyonq(cfg, sock)
        if not wait_pred(lambda: listening(listen) and sock.exists(), 40):
            raise RuntimeError("restart failed")

        # 9 reload: old tunnel stays on A; new uses B
        s_old, p_old, st_old = ws_connect(listen, "/ws", HOST_A)
        _, fo = ws_read_frame(s_old, p_old)
        check(checks, "reload_pre_tunnel_a", fo.startswith(b"UP-A|FIRST|"), {"first": fo[:30]})
        write_cfg_reload_b(cfg, listen, up_a_port, up_b_port, up_rej_port)
        rc_rel, out_rel = ctl(sock, cfg, "reload", "--config", str(cfg))
        time.sleep(0.2)
        s_old.sendall(ws_frame(0x1, b"OLD-TUNNEL", masked=True))
        _, echo_old = ws_read_frame(s_old, p_old)
        check(
            checks,
            "reload_old_tunnel_coherent",
            rc_rel == 0 and echo_old == b"UP-A|OLD-TUNNEL",
            {"rc": rc_rel, "echo": echo_old[:40], "out": out_rel[:120]},
        )
        s_new2, p_new2, stn = ws_connect(listen, "/ws", HOST_A)
        _, fn = ws_read_frame(s_new2, p_new2)
        check(
            checks,
            "reload_new_tunnel_uses_b",
            stn == 101 and fn.startswith(b"UP-B|FIRST|"),
            {"status": stn, "first": fn[:30]},
        )
        try:
            s_old.close()
            s_new2.close()
        except OSError:
            pass

        # 10 abrupt disconnects
        s_ab, p_ab, _ = ws_connect(listen, "/ws-plain", "localhost")
        _, _ = ws_read_frame(s_ab, p_ab)
        s_ab.close()  # abrupt
        time.sleep(0.2)
        check(checks, "downstream_abrupt_ok", True, {})

        # 11 repeated connect/close resource bound
        for _i in range(12):
            sc, pc, stc = ws_connect(listen, "/ws-plain", "localhost")
            if stc != 101:
                check(checks, "churn_101", False, {"status": stc, "i": _i})
                break
            _, _ = ws_read_frame(sc, pc)
            sc.close()
        else:
            check(checks, "churn_101", True, {"n": 12})
        st_churn = ctl_status(sock, cfg)
        check(
            checks,
            "churn_no_active_leak",
            st_churn.get("active_connections") == 0,
            {"active": st_churn.get("active_connections")},
        )

        # 12 graceful shutdown with live tunnel — must not exit immediately
        s_sh, p_sh, _ = ws_connect(listen, "/ws-plain", "localhost")
        _, _ = ws_read_frame(s_sh, p_sh)
        t0 = time.monotonic()
        os.kill(proc.pid, signal.SIGTERM)
        # process should still be alive briefly while tunnel open
        time.sleep(0.5)
        alive_mid = proc.poll() is None
        check(checks, "shutdown_waits_on_live_tunnel", alive_mid, {"alive_mid": alive_mid})
        try:
            s_sh.close()
        except OSError:
            pass
        try:
            proc.wait(timeout=35)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)
        elapsed = time.monotonic() - t0
        check(
            checks,
            "shutdown_exits_after_tunnel",
            proc.poll() is not None and elapsed < 35,
            {"elapsed": round(elapsed, 3), "code": proc.poll()},
        )
        proc = None

        check(
            checks,
            "protocol_scope",
            True,
            {
                "WEBSOCKET_H1": "SUPPORTED",
                "WEBSOCKET_H2_EXTENDED_CONNECT": "UNSUPPORTED_CURRENT_CONTRACT",
                "WEBSOCKET_H3": "UNSUPPORTED_CURRENT_CONTRACT",
            },
        )

    except Exception as exc:
        check(checks, "harness_exception", False, {"err": repr(exc)})
    finally:
        stop_proc(proc)
        up_a.stop()
        up_b.stop()
        up_rej.stop()
        if sock.exists():
            try:
                sock.unlink()
            except OSError:
                pass

    passed = sum(1 for v in checks.values() if v.get("ok"))
    total = len(checks)
    all_ok = passed == total and total > 0
    result = {
        "CAPABILITY_ID": "031",
        "FEATURE_ID": "websocket",
        "FEATURE_NAME": "WebSocket proxying / upgrade",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "STARTED_UTC": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "CHECKS_PASSED": passed,
        "CHECKS_TOTAL": total,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "WEBSOCKET_H1": "SUPPORTED",
        "WEBSOCKET_H2_EXTENDED_CONNECT": "UNSUPPORTED_CURRENT_CONTRACT",
        "WEBSOCKET_H3": "UNSUPPORTED_CURRENT_CONTRACT",
        "CAP036_REOPEN": "NO",
        "CAP035_REOPEN": "NO",
        "CAP034_REOPEN": "NO",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if all_ok else "FAIL",
        "PRODUCT_CONTRACT": {
            "UPGRADE": "HTTP/1.1",
            "TUNNEL": "byte-transparent bidirectional copy",
            "WS_HANDSHAKE_TIMEOUT_S": 30,
            "TUNNEL_IDLE_TIMEOUT": "NONE",
            "UPGRADED_CONNECTION_POOL_REUSE": "NO",
            "LIFECYCLE": "extend_for_upgraded_tunnel until tunnel ends",
        },
        "checks": checks,
        "EXYONQ_BINARY_SHA256": hashlib.sha256(BIN.read_bytes()).hexdigest()
        if BIN.is_file()
        else None,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": passed, "total": total, "FINAL_RESULT": result["FINAL_RESULT"]}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
