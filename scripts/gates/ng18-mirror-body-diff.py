#!/usr/bin/env python3
"""NG-18 / ADR-064 Phase D — mirror_request_body differential corpus.

Real origin, real raw-socket mirror sink (captures exact method/target/body),
real NGINX 1.27 proxy and real ExyonQ proxy. Compares the main response and
every mirrored request byte-for-byte (body hash + length + query + method).

Phases:
  on    mirror /sink1;                      (mirror_request_body defaults to on)
  off   mirror /sink1; mirror_request_body off;
  multi mirror /sink1; mirror /sink2;
"""
import argparse
import hashlib
import os
import shutil
import socket
import socketserver
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORK = Path("/tmp/ng18-mirror-body-diff")
ORIGIN_PORT = 19110
SINK_PORT = 19111
NGINX_PORT = 18111
EXY_PORT = 18112
CONTAINER = "ng18-mirror-body"

SINK_LOCK = threading.Lock()
SINK_LOG = []
SINK_STATUS = 204
SINK_DELAY = 0.0


def _recv_until(conn, marker: bytes) -> bytes:
    buf = b""
    while marker not in buf:
        chunk = conn.recv(65536)
        if not chunk:
            return buf
        buf += chunk
    return buf


def _read_chunked(conn, buf: bytes) -> bytes:
    out = b""
    while True:
        while b"\r\n" not in buf:
            chunk = conn.recv(65536)
            if not chunk:
                return out
            buf += chunk
        line, buf = buf.split(b"\r\n", 1)
        size = int(line.split(b";")[0] or b"0", 16)
        if size == 0:
            while b"\r\n" not in buf:
                chunk = conn.recv(65536)
                if not chunk:
                    return out
                buf += chunk
            return out
        while len(buf) < size + 2:
            chunk = conn.recv(65536)
            if not chunk:
                return out
            buf += chunk
        out += buf[:size]
        buf = buf[size + 2 :]


def read_request(conn) -> dict | None:
    buf = _recv_until(conn, b"\r\n\r\n")
    if b"\r\n\r\n" not in buf:
        return None
    head, rest = buf.split(b"\r\n\r\n", 1)
    lines = head.split(b"\r\n")
    try:
        method, target, _ = lines[0].decode("latin1").split(" ")
    except ValueError:
        return None
    headers = {}
    for line in lines[1:]:
        name, _, value = line.decode("latin1").partition(":")
        headers[name.strip().lower()] = value.strip()
    body = rest
    if headers.get("transfer-encoding", "").lower() == "chunked":
        body = _read_chunked(conn, body)
    elif "content-length" in headers:
        need = int(headers["content-length"])
        while len(body) < need:
            chunk = conn.recv(65536)
            if not chunk:
                break
            body += chunk
    return {"method": method, "target": target, "headers": headers, "body": body}


class _Base(socketserver.BaseRequestHandler):
    def handle(self):
        try:
            entry = read_request(self.request)
        except OSError:
            return
        if entry is None:
            return
        self.server.on_request(entry)
        status, body = self.server.reply()
        reason = "No Content" if status == 204 else "OK" if status == 200 else "Error"
        head = (
            f"HTTP/1.1 {status} {reason}\r\n"
            f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
        ).encode()
        try:
            self.request.sendall(head + body)
        except OSError:
            pass


class SinkServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def on_request(self, entry):
        with SINK_LOCK:
            SINK_LOG.append(entry)

    def reply(self):
        with SINK_LOCK:
            status = SINK_STATUS
            delay = SINK_DELAY
        if delay:
            time.sleep(delay)
        return status, b""


class OriginServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def on_request(self, _entry):
        pass

    def reply(self):
        return 200, b"main-body"


def sink_snapshot() -> list[dict]:
    with SINK_LOCK:
        return list(SINK_LOG)


def sink_clear() -> None:
    with SINK_LOCK:
        SINK_LOG.clear()


def wait_sink(count: int, timeout: float = 4.0) -> list[dict]:
    if count == 0:
        # Prove a negative: give any stray mirror time to arrive.
        time.sleep(0.6)
        return sink_snapshot()
    deadline = time.time() + timeout
    while time.time() < deadline:
        entries = sink_snapshot()
        if len(entries) >= count:
            break
        time.sleep(0.02)
    time.sleep(0.25)
    return sink_snapshot()


def start_server(server_cls, port: int) -> socketserver.ThreadingTCPServer:
    server = server_cls(("0.0.0.0", port), _Base)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def build_request(
    method, path, body, framing, headers, declared_length=None, host="mirror.test"
) -> bytes:
    lines = [f"{method} {path} HTTP/1.1", f"Host: {host}", "Connection: close"]
    if framing == "cl":
        length = len(body) if declared_length is None else declared_length
        lines.append(f"Content-Length: {length}")
    elif framing == "chunked":
        lines.append("Transfer-Encoding: chunked")
    for name, value in (headers or {}).items():
        lines.append(f"{name}: {value}")
    head = ("\r\n".join(lines) + "\r\n\r\n").encode()
    if framing == "chunked":
        payload = (
            f"{len(body):x}\r\n".encode() + body + b"\r\n0\r\n\r\n" if body else b"0\r\n\r\n"
        )
    else:
        payload = body
    return head + payload


def read_response(sock) -> dict:
    buf = b""
    try:
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
    except OSError:
        return {"status": None, "body": b""}
    if b"\r\n\r\n" not in buf:
        return {"status": None, "body": b""}
    head, rest = buf.split(b"\r\n\r\n", 1)
    status = int(head.split(b"\r\n")[0].split(b" ")[1])
    length = 0
    for line in head.split(b"\r\n")[1:]:
        name, _, value = line.decode("latin1").partition(":")
        if name.strip().lower() == "content-length":
            length = int(value.strip())
    body = rest
    try:
        while len(body) < length:
            chunk = sock.recv(65536)
            if not chunk:
                break
            body += chunk
    except OSError:
        pass
    return {"status": status, "body": body}


def send_case(port: int, case: dict) -> dict:
    started = time.time()
    result = _send_case(port, case)
    result["elapsed"] = time.time() - started
    return result


def _send_case(port: int, case: dict) -> dict:
    request = build_request(
        case["method"],
        case["path"],
        case["body"],
        case["framing"],
        case.get("headers"),
        case.get("declared_length"),
    )
    sock = socket.create_connection(("127.0.0.1", port), timeout=20)
    try:
        if case.get("abort"):
            sock.sendall(request[: len(request) - len(case["body"])] + case["body"][:10])
            sock.shutdown(socket.SHUT_WR)
            return {"status": None, "body": b""}
        if case.get("slow"):
            head_len = len(request) - len(case["body"])
            sock.sendall(request[:head_len])
            step = max(1, len(case["body"]) // 5)
            for i in range(0, len(case["body"]), step):
                sock.sendall(case["body"][i : i + step])
                time.sleep(0.1)
            return read_response(sock)
        try:
            sock.sendall(request)
        except OSError:
            # Early rejection (e.g. 413 mid-body) closes the socket while the
            # client still sends — the response is already in flight, read it.
            pass
        return read_response(sock)
    except OSError:
        return {"status": None, "body": b""}
    finally:
        try:
            sock.close()
        except OSError:
            pass


def nginx_config(phase: str) -> str:
    mirrors = "mirror /sink1;" if phase != "multi" else "mirror /sink1; mirror /sink2;"
    off = "mirror_request_body off;" if phase == "off" else ""
    # With mirror_request_body off, nginx still forwards the parent Content-Length
    # to the mirror location; the docs require stripping it there or the mirror
    # upstream waits for a body that never arrives.
    strip = (
        "proxy_pass_request_body off; proxy_set_header Content-Length \"\";"
        if phase == "off"
        else ""
    )
    # Native Linux runs nginx on the host network (host firewalls drop
    # container->host traffic on the docker0 gateway), so upstreams are
    # loopback; Docker Desktop needs the host-gateway alias instead.
    backend = "host.docker.internal" if sys.platform == "darwin" else "127.0.0.1"
    return f"""worker_processes 1; pid /tmp/ng18-mbd.pid; error_log /dev/stderr warn;
events {{ worker_connections 256; }}
http {{ client_max_body_size 32m; access_log off; server {{ listen {NGINX_PORT};
  location = /main {{
    {mirrors} {off}
    proxy_pass http://{backend}:{ORIGIN_PORT};
  }}
  location = /sink1 {{ {strip} proxy_pass http://{backend}:{SINK_PORT}; }}
  location = /sink2 {{ {strip} proxy_pass http://{backend}:{SINK_PORT}; }}
}} }}
"""


def exy_config(phase: str) -> str:
    destinations = '["/sink1"]' if phase != "multi" else '["/sink1", "/sink2"]'
    request_body = "false" if phase == "off" else "true"
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{EXY_PORT}"
routes = ["main", "sink1", "sink2"]
[[route]]
name = "main"
match = {{ path = "/main" }}
upstream = "origin"
[[route]]
name = "sink1"
match = {{ path = "/sink1" }}
upstream = "sink"
[[route]]
name = "sink2"
match = {{ path = "/sink2" }}
upstream = "sink"
[[upstream]]
name = "origin"
target = "http://127.0.0.1:{ORIGIN_PORT}"
[[upstream]]
name = "sink"
target = "http://127.0.0.1:{SINK_PORT}"
[modules.mirror.routes."main"]
destinations = {destinations}
request_body = {request_body}
"""


def wait_port(port: int, timeout: float = 30.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def start_phase(phase: str) -> subprocess.Popen:
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    (WORK / "nginx.conf").write_text(nginx_config(phase))
    (WORK / "exy.toml").write_text(exy_config(phase))
    subprocess.run(
        ["docker", "rm", "-f", CONTAINER],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    if sys.platform == "darwin":
        net_args = [
            "--add-host=host.docker.internal:host-gateway",
            "-p", f"{NGINX_PORT}:{NGINX_PORT}",
        ]
    else:
        net_args = ["--network", "host"]
    subprocess.run(
        [
            "docker", "run", "-d", "--rm", "--name", CONTAINER,
            *net_args,
            "-v", f"{WORK / 'nginx.conf'}:/etc/nginx/nginx.conf:ro",
            "nginx:1.27-alpine",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    wait_port(NGINX_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={**os.environ, "EXYONQ_CONTROL_SOCKET": f"/tmp/ng18-mbd-{phase}.sock"},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy


def stop_phase(exy: subprocess.Popen, keep: bool) -> None:
    exy.terminate()
    try:
        exy.wait(timeout=5)
    except subprocess.TimeoutExpired:
        exy.kill()
    if not keep:
        subprocess.run(
            ["docker", "rm", "-f", CONTAINER],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


def summarize(entries: list[dict]) -> list[dict]:
    return sorted(
        (
            {
                "method": e["method"],
                "target": e["target"],
                "body_sha": hashlib.sha256(e["body"]).hexdigest(),
                "body_len": len(e["body"]),
                "content_type": e["headers"].get("content-type"),
            }
            for e in entries
        ),
        key=lambda e: (e["target"], e["body_sha"]),
    )


def cases(phase: str) -> list[dict]:
    binary = bytes(range(256)) * 256  # 64 KiB, includes NULs
    large = b"L" * (900 * 1024)
    if phase == "off":
        return [
            {"name": "post_off", "method": "POST", "path": "/main",
             "body": b"hello-body", "framing": "cl", "expect": 1,
             "headers": {"Content-Type": "text/plain"}},
            {"name": "chunked_off", "method": "POST", "path": "/main",
             "body": b"chunked-body-123", "framing": "chunked", "expect": 1},
            {"name": "get_off", "method": "GET", "path": "/main",
             "body": b"", "framing": "none", "expect": 1},
        ]
    base = [
        {"name": "post_small", "method": "POST", "path": "/main",
         "body": b"hello-body", "framing": "cl", "expect": 1,
         "headers": {"Content-Type": "text/plain"}},
        {"name": "post_empty", "method": "POST", "path": "/main",
         "body": b"", "framing": "cl", "expect": 1},
        {"name": "post_chunked", "method": "POST", "path": "/main",
         "body": b"chunked-body-123", "framing": "chunked", "expect": 1},
        {"name": "post_binary", "method": "POST", "path": "/main",
         "body": binary, "framing": "cl", "expect": 1,
         "headers": {"Content-Type": "application/octet-stream"}},
        {"name": "post_large", "method": "POST", "path": "/main",
         "body": large, "framing": "cl", "expect": 1},
        {"name": "get", "method": "GET", "path": "/main",
         "body": b"", "framing": "none", "expect": 1},
        {"name": "head", "method": "HEAD", "path": "/main",
         "body": b"", "framing": "none", "expect": 1},
        {"name": "query", "method": "POST", "path": "/main?a=1&b=two",
         "body": b"q", "framing": "cl", "expect": 1},
        {"name": "slow_body", "method": "POST", "path": "/main",
         "body": b"slow-body-0123456789", "framing": "cl", "expect": 1, "slow": True},
        {"name": "mirror_error_ignored", "method": "POST", "path": "/main",
         "body": b"ignored", "framing": "cl", "expect": 1, "sink_status": 500},
        {"name": "mirror_slow_sink", "method": "POST", "path": "/main",
         "body": b"slow-sink", "framing": "cl", "expect": 1, "sink_delay": 2.0},
        {"name": "client_abort", "method": "POST", "path": "/main",
         "body": b"X" * 100, "framing": "cl", "expect": 0, "abort": True},
        {"name": "under_limit", "method": "POST", "path": "/main",
         "body": b"U" * (32 * 1024 * 1024 - 1), "framing": "cl", "expect": 1},
        {"name": "at_limit", "method": "POST", "path": "/main",
         "body": b"M" * (32 * 1024 * 1024), "framing": "cl", "expect": 1},
        {"name": "over_limit", "method": "POST", "path": "/main",
         "body": b"", "framing": "cl", "expect": 0,
         "declared_length": 32 * 1024 * 1024 + 1},
        {"name": "over_limit_chunked", "method": "POST", "path": "/main",
         "body": b"C" * (32 * 1024 * 1024 + 1), "framing": "chunked", "expect": 0},
    ]
    if phase == "multi":
        base = [c for c in base if c["name"] in ("post_small", "post_chunked", "get")]
        for c in base:
            c["expect"] = 2
    return base


def run_phase(phase: str, keep: bool) -> int:
    global SINK_STATUS, SINK_DELAY
    failures = 0
    exy = start_phase(phase)
    try:
        for case in cases(phase):
            SINK_STATUS = case.get("sink_status", 204)
            SINK_DELAY = case.get("sink_delay", 0.0)
            sink_clear()
            main_n = send_case(NGINX_PORT, case)
            sink_n = wait_sink(case["expect"])
            sink_clear()
            main_e = send_case(EXY_PORT, case)
            sink_e = wait_sink(case["expect"])
            if case.get("abort"):
                main_ok = main_n["status"] is None and main_e["status"] is None
            elif case["name"].startswith("over_limit"):
                # Exact client-facing status parity: nginx client_max_body_size
                # → 413; ExyonQ must emit the same class for the same client
                # rejection. Error bodies legitimately differ between servers.
                main_ok = main_n["status"] == main_e["status"]
            else:
                main_ok = (
                    main_n["status"] == main_e["status"]
                    and main_n["body"] == main_e["body"]
                )
            sink_ok = (
                len(sink_n) == case["expect"]
                and len(sink_e) == case["expect"]
                and summarize(sink_n) == summarize(sink_e)
            )
            if case.get("sink_delay"):
                # Background subrequest: a slow mirror must not delay the main response.
                main_ok = main_ok and main_n["elapsed"] < 1.5 and main_e["elapsed"] < 1.5
            ok = main_ok and sink_ok
            failures += 0 if ok else 1
            print(
                f"[{phase}] {case['name']}: {'PASS' if ok else 'FAIL'} "
                f"main nginx={main_n['status']}/{len(main_n['body'])}B/"
                f"{main_n['elapsed']:.2f}s "
                f"exyonq={main_e['status']}/{len(main_e['body'])}B/"
                f"{main_e['elapsed']:.2f}s "
                f"mirrors nginx={len(sink_n)} exyonq={len(sink_e)}"
            )
            if not ok:
                print(f"    nginx sink: {summarize(sink_n)}")
                print(f"    exyonq sink: {summarize(sink_e)}")
    finally:
        SINK_STATUS = 204
        SINK_DELAY = 0.0
        stop_phase(exy, keep)
    return failures


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--phase", choices=["on", "off", "multi"], action="append")
    args = parser.parse_args()
    origin = start_server(OriginServer, ORIGIN_PORT)
    sink = start_server(SinkServer, SINK_PORT)
    failures = 0
    try:
        for phase in args.phase or ["on", "off", "multi"]:
            failures += run_phase(phase, args.keep)
    finally:
        origin.shutdown()
        sink.shutdown()
        subprocess.run(
            ["docker", "rm", "-f", CONTAINER],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    print(f"failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
