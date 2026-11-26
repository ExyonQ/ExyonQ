#!/usr/bin/env python3
"""Real proxy/datapath stress scenarios for Roadmap 0.9.2 F-01..F-05.

Each scenario starts the built `exyonq` binary with an isolated config,
control socket and TMPDIR, then drives real traffic through real sockets.
No mocks: the only helper listeners are test upstreams/clients.

Run on Linux (native amd64/arm64). The output contains one `F0x=PASS` line per
scenario plus named evidence; the shell wrapper rejects missing markers.
"""

import argparse
import asyncio
import base64
import hashlib
import ipaddress
import os
import resource
import shutil
import signal
import socket
import socketserver
import struct
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

GUID = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def log(message):
    print(message, flush=True)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_port(port, timeout=20.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def stop_process(proc):
    if proc is None or proc.poll() is not None:
        return
    proc.terminate()
    try:
        proc.wait(timeout=10)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


class Run:
    def __init__(self, binary, work):
        self.binary = str(Path(binary).resolve())
        self.work = Path(work)
        self.work.mkdir(parents=True, exist_ok=True)
        self.tmp = self.work / "tmp"
        self.tmp.mkdir(exist_ok=True)
        self.log_path = self.work / "exyonq.log"
        self.control = self.work / "control.sock"
        self.proc = None
        self.port = None

    def write_config(self, body):
        path = self.work / "exyonq.toml"
        path.write_text(body)
        return path

    def validate(self, config):
        env = os.environ.copy()
        env["TMPDIR"] = str(self.tmp)
        result = subprocess.run(
            [self.binary, "validate", "-c", str(config)],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            env=env,
            text=True,
        )
        return result.returncode, result.stdout

    def start(self, config, rust_log="info"):
        env = os.environ.copy()
        env["TMPDIR"] = str(self.tmp)
        env["EXYONQ_CONTROL_SOCKET"] = str(self.control)
        env["RUST_LOG"] = rust_log

        def child_limits():
            resource.setrlimit(resource.RLIMIT_NOFILE, (65535, 65535))

        self.log = open(self.log_path, "ab", buffering=0)
        self.proc = subprocess.Popen(
            [self.binary, "serve", "-c", str(config)],
            stdin=subprocess.DEVNULL,
            stdout=self.log,
            stderr=subprocess.STDOUT,
            env=env,
            preexec_fn=child_limits,
        )
        return self.proc

    def stop(self):
        stop_process(self.proc)
        self.proc = None
        if getattr(self, "log", None):
            self.log.close()
            self.log = None

    def log_text(self):
        return self.log_path.read_text(errors="replace") if self.log_path.exists() else ""


def base_config(port, upstream_port, *, waf="", ratelimit="", route_extra=""):
    return f"""config_version = 1

[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
upstream = "backend"
{route_extra}

[[upstream]]
name = "backend"
target = "http://127.0.0.1:{upstream_port}"
{waf}
{ratelimit}
"""


class HttpUpstream:
    """Minimal real HTTP/1.1 upstream. Reads the full body, answers 200."""

    def __init__(self, body_bytes=0):
        self.port = free_port()
        self.body_bytes = body_bytes
        outer = self

        class Handler(socketserver.BaseRequestHandler):
            def handle(self):
                sock = self.request
                sock.settimeout(60)
                data = b""
                try:
                    while b"\r\n\r\n" not in data:
                        chunk = sock.recv(65536)
                        if not chunk:
                            return
                        data += chunk
                    head, _, rest = data.partition(b"\r\n\r\n")
                    length = 0
                    for line in head.split(b"\r\n")[1:]:
                        if line.lower().startswith(b"content-length:"):
                            length = int(line.split(b":", 1)[1].strip())
                    received = len(rest)
                    while received < length:
                        chunk = sock.recv(min(65536, length - received))
                        if not chunk:
                            return
                        received += len(chunk)
                    body = b"ok"
                    sock.sendall(
                        b"HTTP/1.1 200 OK\r\nContent-Length: "
                        + str(len(body)).encode()
                        + b"\r\nConnection: close\r\n\r\n"
                        + body
                    )
                except (OSError, ValueError):
                    return

        self.server = socketserver.ThreadingTCPServer(("127.0.0.1", self.port), Handler)
        self.server.daemon_threads = True
        self.server.allow_reuse_address = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def read_response(sock, timeout=30):
    sock.settimeout(timeout)
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(65536)
        if not chunk:
            return data
        data += chunk
    head, _, body = data.partition(b"\r\n\r\n")
    length = 0
    for line in head.split(b"\r\n")[1:]:
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1].strip())
    while len(body) < length:
        chunk = sock.recv(65536)
        if not chunk:
            break
        body += chunk
    return head + b"\r\n\r\n" + body


def status_code(response):
    if not response:
        return 0
    try:
        return int(response.split(b" ", 2)[1])
    except (IndexError, ValueError):
        return 0


def rss_kib(pid):
    try:
        with open(f"/proc/{pid}/status", "r", encoding="ascii") as fh:
            for line in fh:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
    except OSError:
        pass
    try:
        return int(
            subprocess.check_output(
                ["ps", "-o", "rss=", "-p", str(pid)], text=True
            ).strip()
        )
    except (OSError, ValueError, subprocess.SubprocessError):
        return 0


def f01(binary, work, rss_limit_mib):
    upstream = HttpUpstream()
    run = Run(binary, work / "f01")
    port = free_port()
    run.port = port
    body_len = 32 * 1024 * 1024
    config = run.write_config(
        base_config(
            port,
            upstream.port,
            waf="[waf]\nenabled = false\n",
            route_extra=(
                "max_request_body_bytes = 67108864\n"
                "request_body_timeout_ms = 30000\n"
                "request_body_spool_bytes = 4194304\n"
            ),
        )
    )
    code, out = run.validate(config)
    if code != 0:
        log(f"F01_VALIDATE=FAIL\n{out}")
        return 1
    run.start(config)
    if not wait_port(port):
        log(f"F01_START=FAIL\n{run.log_text()[-2000:]}")
        return 1
    baseline = rss_kib(run.proc.pid)
    peak = baseline
    stop_sampling = threading.Event()

    def sample():
        nonlocal peak
        while not stop_sampling.is_set():
            peak = max(peak, rss_kib(run.proc.pid))
            time.sleep(0.01)

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.settimeout(60)
    header = (
        b"POST /upload HTTP/1.1\r\nHost: stress.local\r\n"
        b"Content-Type: application/octet-stream\r\n"
        b"Content-Length: " + str(body_len).encode() + b"\r\n\r\n"
    )
    sock.sendall(header)
    chunk = b"x" * 65536
    sent = 0
    while sent < body_len:
        this = min(len(chunk), body_len - sent)
        sock.sendall(chunk[:this])
        sent += this
        time.sleep(0.002)
    response = read_response(sock, timeout=60)
    sock.close()
    stop_sampling.set()
    sampler.join(timeout=2)
    rc = status_code(response)
    delta = peak - baseline
    limit_kib = rss_limit_mib * 1024
    leftovers = list(run.tmp.glob("exyonq-proxy-body-*"))
    ok = rc == 200 and delta <= limit_kib and not leftovers
    log(f"F01_BODY_BYTES={body_len}")
    log(f"F01_STATUS={rc}")
    log(f"F01_RSS_BASELINE_KIB={baseline}")
    log(f"F01_RSS_PEAK_KIB={peak}")
    log(f"F01_RSS_DELTA_KIB={delta}")
    log(f"F01_RSS_LIMIT_KIB={limit_kib}")
    log(f"F01_SPOOL_LEFTOVERS={len(leftovers)}")
    run.stop()
    upstream.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"F01={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def f02(binary, work):
    upstream = HttpUpstream()
    run = Run(binary, work / "f02")
    port = free_port()
    run.port = port
    rules = run.work / "local.rules"
    rules.write_text(
        'SecRule REQUEST_URI "@contains /blocked" "id:990001,deny"\n'
        "SecRule broken\n"
    )

    def config(fail_policy):
        return run.write_config(
            base_config(
                port,
                upstream.port,
                waf=(
                    "[waf]\n"
                    "enabled = true\n"
                    'mode = "block"\n'
                    f'fail_policy = "{fail_policy}"\n'
                    "\n[[waf.ruleset]]\n"
                    'id = "local"\n'
                    f'rules_file = "{rules}"\n'
                ),
            )
        )

    # Closed policy must reject the corpus at startup.
    closed = config("closed_for_invalid_rules")
    code, out = run.validate(closed)
    closed_obj = run.start(closed)
    time.sleep(1.0)
    closed_running = closed_obj.poll() is None
    closed_log = run.log_text()
    run.stop()
    closed_ok = (not closed_running) and (
        "fail closed" in closed_log or "discarded" in closed_log or code != 0
    )

    # Best-effort keeps the valid rule, reports parsed/discarded, and blocks
    # matching live traffic while allowing a non-matching request.
    best = config("best_effort")
    code, out = run.validate(best)
    if code != 0:
        log(f"F02_VALIDATE=FAIL\n{out}")
        return 1
    run.log_path.unlink(missing_ok=True)
    run.start(best, rust_log="info")
    if not wait_port(port):
        log(f"F02_START=FAIL\n{run.log_text()[-2000:]}")
        return 1

    def get(path):
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        s.sendall(
            f"GET {path} HTTP/1.1\r\nHost: stress.local\r\nConnection: close\r\n\r\n".encode()
        )
        response = read_response(s, timeout=10)
        s.close()
        return status_code(response)

    blocked = get("/blocked")
    allowed = get("/open")
    text = run.log_text()
    parsed = "parsed=1" in text and "discarded=1" in text
    ok = closed_ok and blocked == 403 and allowed == 200 and parsed
    log(f"F02_CLOSED_START_REJECTED={closed_ok}")
    log(f"F02_BEST_EFFORT_STATUS={code}")
    log(f"F02_BLOCKED_STATUS={blocked}")
    log(f"F02_ALLOWED_STATUS={allowed}")
    log(f"F02_STATS_LOGGED={parsed}")
    run.stop()
    upstream.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"F02={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def f04(binary, work):
    upstream = HttpUpstream()
    run = Run(binary, work / "f04")
    port = free_port()
    run.port = port
    config = run.write_config(
        base_config(
            port,
            upstream.port,
            waf="[waf]\nenabled = false\n",
            ratelimit=(
                "[modules.ratelimit.client]\n"
                "enabled = true\n"
                "max_connections_per_ip = 20000\n"
                "block_bad_req = true\n"
                "ban_seconds = 600\n"
            ),
        )
    )
    code, out = run.validate(config)
    if code != 0:
        log(f"F04_VALIDATE=FAIL\n{out}")
        return 1
    run.start(config, rust_log="debug")
    if not wait_port(port):
        log(f"F04_START=FAIL\n{run.log_text()[-2000:]}")
        return 1

    held = []
    ok = False
    try:
        # 9 998 live rows. Loopback /8 gives distinct source IPs so the
        # bounded per-IP table actually reaches MAX_CLIENTS.
        base = int(ipaddress.IPv4Address("127.1.0.1"))
        for i in range(9998):
            ip = str(ipaddress.IPv4Address(base + i))
            s = socket.socket()
            s.settimeout(5)
            s.bind((ip, 0))
            s.connect(("127.0.0.1", port))
            held.append(s)
        log(f"F04_HELD_CONNECTIONS={len(held)}")

        # Ban one IP with three malformed requests.
        banned_ip = "127.2.0.1"
        for _ in range(3):
            s = socket.socket()
            s.settimeout(2)
            s.bind((banned_ip, 0))
            s.connect(("127.0.0.1", port))
            s.sendall(b"GARBAGE\r\n\r\n")
            try:
                s.recv(256)
            except OSError:
                pass
            s.close()
        time.sleep(0.5)

        # Keep a separate active connection with a completed request.
        active = socket.socket()
        active.settimeout(10)
        active.bind(("127.2.0.2", 0))
        active.connect(("127.0.0.1", port))
        active.sendall(
            b"GET / HTTP/1.1\r\nHost: stress.local\r\nConnection: keep-alive\r\n\r\n"
        )
        active_response = read_response(active, timeout=10)
        active_status = status_code(active_response)

        # The next unseen IP must be refused because every row is protected.
        newcomer = socket.socket()
        newcomer.settimeout(2)
        newcomer.bind(("127.3.0.1", 0))
        newcomer.connect(("127.0.0.1", port))
        newcomer.sendall(b"GET / HTTP/1.1\r\nHost: stress.local\r\n\r\n")
        newcomer_response = b""
        try:
            newcomer_response = newcomer.recv(1024)
        except OSError:
            pass
        newcomer.close()

        # The banned IP must still be banned, not replaced.
        banned = socket.socket()
        banned.settimeout(2)
        banned.bind((banned_ip, 0))
        banned.connect(("127.0.0.1", port))
        banned.sendall(b"GET / HTTP/1.1\r\nHost: stress.local\r\n\r\n")
        banned_response = b""
        try:
            banned_response = banned.recv(1024)
        except OSError:
            pass
        banned.close()

        table_full = "TableFull" in run.log_text()
        ok = (
            active_status == 200
            and newcomer_response == b""
            and banned_response == b""
            and table_full
        )
        log(f"F04_ACTIVE_STATUS={active_status}")
        log(f"F04_NEWCOMER_RESPONSE_BYTES={len(newcomer_response)}")
        log(f"F04_BANNED_RESPONSE_BYTES={len(banned_response)}")
        log(f"F04_TABLE_FULL_LOGGED={table_full}")
        active.close()
    finally:
        for s in held:
            try:
                s.close()
            except OSError:
                pass
        run.stop()
        upstream.close()
        shutil.rmtree(run.work, ignore_errors=True)
    log(f"F04={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


class WsUpstream:
    def __init__(self):
        self.port = free_port()
        self.loop = asyncio.new_event_loop()
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()
        self.ready.wait(5)

    def _run(self):
        asyncio.set_event_loop(self.loop)
        self.loop.run_until_complete(self._serve())

    async def _serve(self):
        async def handler(reader, writer):
            try:
                request = await reader.readuntil(b"\r\n\r\n")
                key = b""
                for line in request.split(b"\r\n"):
                    if line.lower().startswith(b"sec-websocket-key:"):
                        key = line.split(b":", 1)[1].strip()
                        break
                accept = base64.b64encode(hashlib.sha1(key + GUID).digest())
                writer.write(
                    b"HTTP/1.1 101 Switching Protocols\r\n"
                    b"Upgrade: websocket\r\nConnection: Upgrade\r\n"
                    b"Sec-WebSocket-Accept: " + accept + b"\r\n\r\n"
                )
                await writer.drain()
                await reader.read()
            except (asyncio.IncompleteReadError, ConnectionError, OSError):
                pass
            finally:
                writer.close()
                try:
                    await writer.wait_closed()
                except (ConnectionError, OSError):
                    pass

        server = await asyncio.start_server(handler, "127.0.0.1", self.port)
        self.server = server
        self.ready.set()
        async with server:
            await server.serve_forever()

    def close(self):
        try:
            self.loop.call_soon_threadsafe(self.server.close)
            self.loop.call_soon_threadsafe(self.loop.stop)
        except AttributeError:
            pass


async def ws_handshake(port, source_ip="127.0.0.1"):
    reader, writer = await asyncio.open_connection(
        "127.0.0.1", port, local_addr=(source_ip, 0)
    )
    key = base64.b64encode(os.urandom(16)).decode()
    writer.write(
        (
            "GET /ws HTTP/1.1\r\n"
            "Host: stress.local\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode()
    )
    await writer.drain()
    response = await reader.readuntil(b"\r\n\r\n")
    return reader, writer, response


def f05(binary, work):
    upstream = WsUpstream()
    run = Run(binary, work / "f05")
    port = free_port()
    run.port = port
    config = run.write_config(
        base_config(
            port,
            upstream.port,
            waf="[waf]\nenabled = false\n",
            route_extra=(
                "websocket_idle_timeout_ms = 5000\n"
                "websocket_max_tunnels = 2000\n"
            ),
        )
    )
    code, out = run.validate(config)
    if code != 0:
        log(f"F05_VALIDATE=FAIL\n{out}")
        return 1
    run.start(config, rust_log="info")
    if not wait_port(port):
        log(f"F05_START=FAIL\n{run.log_text()[-2000:]}")
        return 1

    async def scenario():
        clients = []
        semaphore = asyncio.Semaphore(200)

        async def connect_one():
            async with semaphore:
                reader, writer, response = await ws_handshake(port)
                return reader, writer, response

        results = await asyncio.gather(*(connect_one() for _ in range(2000)))
        for reader, writer, response in results:
            if b" 101 " not in response:
                return False, f"handshake={response[:80]!r}", 0, 0
            clients.append((reader, writer))
        log("F05_TUNNELS=2000")

        reader, writer, response = await ws_handshake(port, "127.4.0.1")
        over_status = 0
        try:
            over_status = int(response.split(b" ", 2)[1])
        except (IndexError, ValueError):
            pass
        writer.close()
        try:
            await writer.wait_closed()
        except (ConnectionError, OSError):
            pass

        closed = 0
        deadline = time.monotonic() + 8
        for reader, writer in clients:
            try:
                frame = await asyncio.wait_for(reader.readexactly(4), timeout=8)
                if (
                    frame[0] == 0x88
                    and frame[1] == 0x02
                    and struct.unpack(">H", frame[2:])[0] == 1000
                ):
                    closed += 1
            except (asyncio.IncompleteReadError, asyncio.TimeoutError, ConnectionError, OSError):
                pass
            writer.close()
            try:
                await writer.wait_closed()
            except (ConnectionError, OSError):
                pass
        return over_status == 503 and closed == 2000, "", over_status, closed

    try:
        ok, detail, over_status, closed = asyncio.run(scenario())
    except Exception as exc:  # noqa: BLE001 - evidence must show the failure
        ok, detail, over_status, closed = False, repr(exc), 0, 0
    log(f"F05_OVERFLOW_STATUS={over_status}")
    log(f"F05_CLOSE_1000={closed}")
    if detail:
        log(f"F05_DETAIL={detail}")
    run.stop()
    upstream.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"F05={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", choices=["f01", "f02", "f04", "f05"])
    parser.add_argument("--binary", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--rss-limit-mib", type=int, default=16)
    args = parser.parse_args()

    if sys.platform != "linux":
        log(f"{args.scenario.upper()}=NOT_APPLICABLE_LINUX_REQUIRED")
        return 3
    resource.setrlimit(resource.RLIMIT_NOFILE, (65535, 65535))
    work = Path(args.work)
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    if args.scenario == "f01":
        return f01(args.binary, work, args.rss_limit_mib)
    if args.scenario == "f02":
        return f02(args.binary, work)
    if args.scenario == "f04":
        return f04(args.binary, work)
    return f05(args.binary, work)


if __name__ == "__main__":
    sys.exit(main())
