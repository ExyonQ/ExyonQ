#!/usr/bin/env python3
"""Q-05 / ADR-058 — N WebSockets survive reload then drain (no lost echo frames).

ZERO_FAKE: real exyonq + real echo upstream + real RFC6455 client frames.
Default N=100 (WS_COUNT). Cap031 already covers single-tunnel; this proves scale.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
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
HEAD = os.environ.get("HEAD", "UNKNOWN")
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "debug" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL", str(WS / "target" / "debug" / "exyonqctl")))
N = int(os.environ.get("WS_COUNT", "100"))
WS_GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
HOST = "q05.example"


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_pred(pred, timeout: float = 60.0) -> bool:
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
    return base64.b64encode(hashlib.sha1(key.encode() + WS_GUID).digest()).decode()


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


def ws_read_frame(sock: socket.socket, pending: bytearray, timeout: float = 8.0) -> tuple[int, bytes]:
    sock.settimeout(timeout)

    def need(n: int) -> None:
        while len(pending) < n:
            chunk = sock.recv(65536)
            if not chunk:
                raise EOFError("eof")
            pending.extend(chunk)

    need(2)
    b0, b1 = pending[0], pending[1]
    del pending[:2]
    opcode = b0 & 0x0F
    masked = bool(b1 & 0x80)
    ln = b1 & 0x7F
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
    return opcode, payload


class EchoUpstream:
    def __init__(self, port: int) -> None:
        self.port = port
        self.alive = True
        self._srv = socket.socket()
        self._srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self._srv.bind(("127.0.0.1", port))
        self._srv.listen(256)
        self._srv.settimeout(0.5)
        self._t = threading.Thread(target=self._loop, daemon=True)
        self._t.start()

    def _loop(self) -> None:
        while self.alive:
            try:
                conn, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            threading.Thread(target=self._client, args=(conn,), daemon=True).start()

    def _client(self, conn: socket.socket) -> None:
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
            headers = {}
            for ln in head.decode("latin-1", errors="replace").split("\r\n")[1:]:
                if ":" in ln:
                    k, v = ln.split(":", 1)
                    headers[k.strip().lower()] = v.strip()
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
            conn.sendall(ws_frame(0x1, b"UP|FIRST", masked=False))
            while self.alive:
                try:
                    op, payload = ws_read_frame(conn, pending, timeout=30.0)
                except (EOFError, TimeoutError, socket.timeout, OSError):
                    break
                if op == 0x8:
                    break
                if op == 0x9:
                    conn.sendall(ws_frame(0xA, payload, masked=False))
                    continue
                conn.sendall(ws_frame(0x1, b"UP|" + payload, masked=False))
        finally:
            try:
                conn.close()
            except OSError:
                pass

    def stop(self) -> None:
        self.alive = False
        try:
            self._srv.close()
        except OSError:
            pass


def ws_connect(port: int) -> tuple[socket.socket, bytearray, int]:
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET /ws HTTP/1.1\r\nHost: {HOST}\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
        f"Sec-WebSocket-Version: 13\r\n\r\n"
    ).encode()
    s = socket.create_connection(("127.0.0.1", port), timeout=5.0)
    s.sendall(req)
    s.settimeout(8.0)
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


def write_cfg(path: Path, listen: int, up: int, timeout_ms: int = 5000) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["ws"]

[[route]]
name = "ws"
match = {{ path = "/ws", host = "{HOST}" }}
upstream = "echo"

[[upstream]]
name = "echo"
timeout_ms = {timeout_ms}
max_connect_retries = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {up}
weight = 1
""",
        encoding="utf-8",
    )


def echo_roundtrip(sock: socket.socket, pending: bytearray, msg: bytes) -> bool:
    sock.sendall(ws_frame(0x1, msg, masked=True))
    try:
        while True:
            op, payload = ws_read_frame(sock, pending, timeout=10.0)
            if op in (0x1, 0x2):
                if payload.startswith(b"UP|FIRST"):
                    continue
                return payload == b"UP|" + msg
            if op == 0x8:
                return False
    except (EOFError, TimeoutError, OSError):
        return False


def ctl(sock: Path, cfg: Path, *args: str) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    r = subprocess.run(
        [str(CTL), *args, "--socket", str(sock)],
        capture_output=True,
        text=True,
        timeout=30,
        env=env,
    )
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def main() -> int:
    if not BIN.is_file() or not CTL.is_file():
        print(f"FAIL: missing {BIN} or {CTL}", file=sys.stderr)
        return 2
    EV.mkdir(parents=True, exist_ok=True)
    listen = pick_port()
    up_port = pick_port()
    upstream = EchoUpstream(up_port)
    td = Path(tempfile.mkdtemp(prefix="q05-ws-", dir=str(EV)))
    cfg = td / "exyonq.toml"
    write_cfg(cfg, listen, up_port, timeout_ms=5000)
    sock = Path(f"/tmp/exyonq-q05-{os.getpid()}-{listen}.sock")
    if sock.exists():
        sock.unlink()
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(cfg)
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_LOG_LEVEL"] = "warn"
    log_path = td / "exyonq.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(cfg)],
        cwd=str(WS),
        env=env,
        stdout=log_path.open("w", encoding="utf-8"),
        stderr=subprocess.STDOUT,
    )
    checks: dict = {}
    tunnels: list[tuple[socket.socket, bytearray]] = []
    verdict = "FAIL"
    try:
        if not wait_pred(lambda: listening(listen) and sock.exists(), 45.0):
            checks["listen"] = {"ok": False}
        else:
            # Open N tunnels; drain FIRST frames.
            opened = 0
            for i in range(N):
                s, pending, status = ws_connect(listen)
                if status != 101:
                    s.close()
                    continue
                # consume optional FIRST
                try:
                    op, payload = ws_read_frame(s, pending, timeout=3.0)
                    if not (op in (0x1, 0x2) and payload.startswith(b"UP|FIRST")):
                        pending[0:0] = ws_frame(op, payload, masked=False)  # shouldn't happen
                except (EOFError, TimeoutError, OSError):
                    pass
                tunnels.append((s, pending))
                opened += 1
            checks["open_n"] = {"ok": opened == N, "opened": opened, "want": N}

            # Baseline echo on all
            base_ok = 0
            for i, (s, p) in enumerate(tunnels):
                if echo_roundtrip(s, p, f"pre-{i}".encode()):
                    base_ok += 1
            checks["pre_reload_echo"] = {"ok": base_ok == opened, "ok_n": base_ok}

            # Reload with fingerprint-changing but WS-safe timeout bump
            write_cfg(cfg, listen, up_port, timeout_ms=5001)
            rc_rel, out_rel = ctl(sock, cfg, "config", "reload", str(cfg))
            checks["reload"] = {"ok": rc_rel == 0, "rc": rc_rel, "out": out_rel[:400]}

            post_rel = 0
            for i, (s, p) in enumerate(tunnels):
                if echo_roundtrip(s, p, f"post-rel-{i}".encode()):
                    post_rel += 1
            checks["post_reload_echo"] = {
                "ok": post_rel == opened,
                "ok_n": post_rel,
                "lost": opened - post_rel,
            }

            # Drain: new WS rejected; existing still echo
            rc_d, out_d = ctl(sock, cfg, "drain")
            checks["drain_cmd"] = {"ok": rc_d == 0, "rc": rc_d, "out": out_d[:300]}
            try:
                sn, _, st_new = ws_connect(listen)
                sn.close()
            except OSError:
                st_new = -1
            # Also raw check for 503 body
            try:
                raw_s = socket.create_connection(("127.0.0.1", listen), timeout=2.0)
                key = base64.b64encode(os.urandom(16)).decode()
                raw_s.sendall(
                    (
                        f"GET /ws HTTP/1.1\r\nHost: {HOST}\r\nUpgrade: websocket\r\n"
                        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
                        f"Sec-WebSocket-Version: 13\r\n\r\n"
                    ).encode()
                )
                raw_s.settimeout(3.0)
                head = raw_s.recv(4096)
                raw_s.close()
                new_rejected = b"503" in head or st_new in (503, 0, -1)
            except OSError:
                new_rejected = True
                head = b""
            checks["drain_rejects_new"] = {
                "ok": new_rejected,
                "status": st_new,
                "head": head[:120].decode("latin-1", errors="replace"),
            }

            post_dr = 0
            for i, (s, p) in enumerate(tunnels):
                if echo_roundtrip(s, p, f"post-drain-{i}".encode()):
                    post_dr += 1
            checks["post_drain_echo"] = {
                "ok": post_dr == opened,
                "ok_n": post_dr,
                "lost": opened - post_dr,
            }

            verdict = (
                "PASS"
                if all(c.get("ok") for c in checks.values())
                else "FAIL"
            )
    finally:
        for s, _ in tunnels:
            try:
                s.close()
            except OSError:
                pass
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=3)
        upstream.stop()
        if sock.exists():
            try:
                sock.unlink()
            except OSError:
                pass

    result = {
        "slice": "Q-05",
        "adr": "ADR-058",
        "head": HEAD,
        "ws_count": N,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
        "verdict": verdict,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": verdict, "out": str(OUT), "n": N}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
