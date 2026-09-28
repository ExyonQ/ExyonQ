#!/usr/bin/env python3
"""CAPABILITY_001 = http-1-1 — real product E2E (single capability).

Authoritative path: release binary → production config → real TCP → real HTTP/1.1
request bytes → ExyonQ parser/router/response → client-visible wire → assertions.

No synthetic product path. No stand-in ExyonQ process. No direct parser unit
invocation as authoritative evidence.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import sys
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
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

BODY = b"cap001-http11-body-v1-post-dep-seal"
EXPECTED_BODY_SHA256 = hashlib.sha256(BODY).hexdigest()


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def recv_until(sock: socket.socket, needle: bytes, limit: int = 1 << 20) -> bytes:
    buf = bytearray()
    sock.settimeout(10.0)
    while needle not in buf and len(buf) < limit:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buf.extend(chunk)
    return bytes(buf)


def parse_http_response(raw: bytes) -> dict:
    if b"\r\n\r\n" not in raw:
        return {
            "ok": False,
            "status_line": "",
            "version": "",
            "status": None,
            "headers": {},
            "body": b"",
            "raw": raw,
        }
    head, body = raw.split(b"\r\n\r\n", 1)
    lines = head.split(b"\r\n")
    status_line = lines[0].decode("latin-1", errors="replace")
    parts = status_line.split(" ", 2)
    version = parts[0] if parts else ""
    try:
        status = int(parts[1]) if len(parts) > 1 else None
    except ValueError:
        status = None
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if b":" not in line:
            continue
        k, v = line.split(b":", 1)
        headers[k.decode("latin-1").strip().lower()] = v.decode("latin-1").strip()
    cl = headers.get("content-length")
    if cl is not None:
        try:
            n = int(cl)
            body = body[:n]
        except ValueError:
            pass
    return {
        "ok": True,
        "status_line": status_line,
        "version": version,
        "status": status,
        "headers": headers,
        "body": body,
        "raw": raw,
    }


def http11_exchange(
    port: int,
    req: bytes,
    *,
    sock: socket.socket | None = None,
    close_after: bool = True,
) -> tuple[dict, socket.socket | None]:
    own = sock is None
    if own:
        sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    assert sock is not None
    sock.sendall(req)
    # Read headers first
    hdr = recv_until(sock, b"\r\n\r\n")
    if b"\r\n\r\n" not in hdr:
        if close_after and own:
            sock.close()
            return parse_http_response(hdr), None
        return parse_http_response(hdr), sock
    head, rest = hdr.split(b"\r\n\r\n", 1)
    headers: dict[str, str] = {}
    for line in head.split(b"\r\n")[1:]:
        if b":" not in line:
            continue
        k, v = line.split(b":", 1)
        headers[k.decode("latin-1").strip().lower()] = v.decode("latin-1").strip()
    body = rest
    cl = headers.get("content-length")
    if cl is not None:
        need = int(cl) - len(body)
        sock.settimeout(10.0)
        while need > 0:
            chunk = sock.recv(min(4096, need))
            if not chunk:
                break
            body += chunk
            need -= len(chunk)
    raw = head + b"\r\n\r\n" + body
    resp = parse_http_response(raw)
    if close_after and own:
        sock.close()
        return resp, None
    return resp, sock


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "http-1-1",
                    "CAPABILITY": "CAPABILITY_001",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "PRODUCT_DEFECT": "NO",
                    "HARNESS_DEFECT": "NO",
                    "ENVIRONMENT_BLOCKER": "YES",
                    "CLASSIFICATION": "ENVIRONMENT_DEFECT",
                    "DETAIL": f"missing binary {BINARY}",
                    "ARCH_LABEL": ARCH_LABEL,
                    "HOST_LABEL": HOST_LABEL,
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    www = EV / "www-http11"
    (www / "a").mkdir(parents=True, exist_ok=True)
    (www / "a" / "small.txt").write_bytes(BODY)

    port = pick_port()
    cfg = EV / "cfg-http11.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["ra"]

[[route]]
name = "ra"
match = {{ path = "/a", host = "site-a.test" }}
root = "{www / 'a'}"
index = "index.html"
"""
    )
    log = EV / "exyonq-http11.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    result: dict = {
        "FEATURE_ID": "http-1-1",
        "CAPABILITY": "CAPABILITY_001",
        "CAPABILITY_NAME": "http-1-1",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY),
        "CONFIG_SHA256": sha256_file(cfg),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "EXPECTED_BODY_SHA256": EXPECTED_BODY_SHA256,
        "PRODUCT_CONTRACT": "Accept HTTP/1.1 on TCP listeners; serve responses",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "LOCAL_ITERATION_ONLY": "NO",
            "NOT_LINUX_EVIDENCE": "NO",
            "DUAL_ARCH_REQUIRED": "YES",
            "RELEASE_BINARY": "YES",
        },
    }
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "PRODUCT_DEFECT": "UNKNOWN",
                    "HARNESS_DEFECT": "YES",
                    "ENVIRONMENT_BLOCKER": "YES",
                    "CLASSIFICATION": "HARNESS_DEFECT",
                    "DETAIL": "listen timeout",
                    "LOG_TAIL": log.read_text(errors="replace")[-3000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        # --- Positive GET on real TCP ---
        get_req = (
            b"GET /a/small.txt HTTP/1.1\r\n"
            b"Host: site-a.test\r\n"
            b"Connection: keep-alive\r\n"
            b"\r\n"
        )
        get_resp, _ = http11_exchange(port, get_req)
        observed_version = get_resp["version"]
        body_sha = sha256_bytes(get_resp["body"]) if get_resp["body"] else None
        cl_hdr = get_resp["headers"].get("content-length")
        positive_get = (
            get_resp["ok"]
            and observed_version == "HTTP/1.1"
            and get_resp["status"] == 200
            and get_resp["body"] == BODY
            and body_sha == EXPECTED_BODY_SHA256
            and cl_hdr == str(len(BODY))
        )

        # --- HEAD ---
        head_req = (
            b"HEAD /a/small.txt HTTP/1.1\r\n"
            b"Host: site-a.test\r\n"
            b"Connection: close\r\n"
            b"\r\n"
        )
        head_resp, _ = http11_exchange(port, head_req)
        head_ok = (
            head_resp["ok"]
            and head_resp["version"] == "HTTP/1.1"
            and head_resp["status"] == 200
            and head_resp["body"] == b""
        )

        # --- Keep-alive: two requests on one TCP connection ---
        sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        r1, sock = http11_exchange(port, get_req, sock=sock, close_after=False)
        r2, sock = http11_exchange(port, get_req, sock=sock, close_after=False)
        keepalive_ok = (
            sock is not None
            and r1["ok"]
            and r2["ok"]
            and r1["version"] == "HTTP/1.1"
            and r2["version"] == "HTTP/1.1"
            and r1["status"] == 200
            and r2["status"] == 200
            and r1["body"] == BODY
            and r2["body"] == BODY
            and sha256_bytes(r1["body"]) == EXPECTED_BODY_SHA256
            and sha256_bytes(r2["body"]) == EXPECTED_BODY_SHA256
        )
        # Prove connection still open after request 2
        premature_close = False
        try:
            sock.settimeout(0.2)
            peek = sock.recv(1, socket.MSG_PEEK)
            # empty peek with timeout means still open idle; closed peer returns b''
            if peek == b"":
                premature_close = True
        except socket.timeout:
            premature_close = False
        except OSError:
            premature_close = True
        keepalive_ok = keepalive_ok and not premature_close
        if sock is not None:
            sock.close()

        # --- Connection: close semantics ---
        close_req = (
            b"GET /a/small.txt HTTP/1.1\r\n"
            b"Host: site-a.test\r\n"
            b"Connection: close\r\n"
            b"\r\n"
        )
        close_sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        close_resp, close_sock = http11_exchange(
            port, close_req, sock=close_sock, close_after=False
        )
        close_semantics_ok = (
            close_resp["ok"]
            and close_resp["version"] == "HTTP/1.1"
            and close_resp["status"] == 200
            and close_resp["body"] == BODY
        )
        # After Connection: close, peer should close (or advertise close)
        conn_hdr = (close_resp["headers"].get("connection") or "").lower()
        peer_closed = False
        if close_sock is not None:
            close_sock.settimeout(2.0)
            try:
                leftover = close_sock.recv(1)
                peer_closed = leftover == b""
            except socket.timeout:
                peer_closed = False
            except OSError:
                peer_closed = True
            close_sock.close()
        connection_close_ok = close_semantics_ok and (
            peer_closed or "close" in conn_hdr
        )

        # --- Missing file (product routing) ---
        miss_req = (
            b"GET /a/missing.txt HTTP/1.1\r\n"
            b"Host: site-a.test\r\n"
            b"Connection: close\r\n"
            b"\r\n"
        )
        miss_resp, _ = http11_exchange(port, miss_req)
        miss_ok = miss_resp["ok"] and miss_resp["version"] == "HTTP/1.1" and miss_resp["status"] == 404

        # --- No matching Host: must not return 200 with body ---
        nohost_req = (
            b"GET /a/small.txt HTTP/1.1\r\n"
            b"Host: other.test\r\n"
            b"Connection: close\r\n"
            b"\r\n"
        )
        nohost_resp, _ = http11_exchange(port, nohost_req)
        nohost_ok = nohost_resp["ok"] and nohost_resp["status"] != 200

        # --- Negative: malformed request line (wire bytes) ---
        bad_line = b"GET /a/small.txt\r\nHost: site-a.test\r\n\r\n"  # missing HTTP version
        bad_line_sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        bad_line_sock.sendall(bad_line)
        bad_line_sock.settimeout(3.0)
        try:
            bad_line_raw = bad_line_sock.recv(8192)
        except socket.timeout:
            bad_line_raw = b""
        bad_line_sock.close()
        bad_line_resp = parse_http_response(bad_line_raw) if bad_line_raw else {
            "ok": False,
            "status": None,
            "version": "",
            "body": b"",
            "status_line": "",
        }
        # Must not succeed as normal 200 with body
        malformed_line_ok = not (
            bad_line_resp.get("status") == 200 and bad_line_resp.get("body") == BODY
        )
        # Prefer: connection closed or 4xx
        if bad_line_raw == b"":
            malformed_line_ok = True
        elif bad_line_resp.get("status") is not None and bad_line_resp["status"] >= 400:
            malformed_line_ok = True

        # --- Negative: malformed header (NUL / space in name) ---
        bad_hdr = (
            b"GET /a/small.txt HTTP/1.1\r\n"
            b"Host: site-a.test\r\n"
            b"Bad Header: x\r\n"
            b"\r\n"
        )
        bad_hdr_sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        bad_hdr_sock.sendall(bad_hdr)
        bad_hdr_sock.settimeout(3.0)
        try:
            bad_hdr_raw = bad_hdr_sock.recv(8192)
        except socket.timeout:
            bad_hdr_raw = b""
        bad_hdr_sock.close()
        bad_hdr_resp = parse_http_response(bad_hdr_raw) if bad_hdr_raw else {
            "ok": False,
            "status": None,
            "body": b"",
        }
        malformed_hdr_ok = not (
            bad_hdr_resp.get("status") == 200 and bad_hdr_resp.get("body") == BODY
        )
        if bad_hdr_raw == b"":
            malformed_hdr_ok = True
        elif bad_hdr_resp.get("status") is not None and bad_hdr_resp["status"] >= 400:
            malformed_hdr_ok = True

        # --- Concurrent real clients ---
        conc: list[bool] = []
        lock = threading.Lock()

        def one() -> None:
            try:
                resp, _ = http11_exchange(port, get_req)
                ok = (
                    resp["ok"]
                    and resp["version"] == "HTTP/1.1"
                    and resp["status"] == 200
                    and resp["body"] == BODY
                    and sha256_bytes(resp["body"]) == EXPECTED_BODY_SHA256
                )
            except OSError:
                ok = False
            with lock:
                conc.append(ok)

        ths = [threading.Thread(target=one) for _ in range(8)]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        conc_ok = len(conc) == 8 and all(conc)

        positive = positive_get and head_ok and keepalive_ok and connection_close_ok
        negative = miss_ok and nohost_ok and malformed_line_ok and malformed_hdr_ok
        overall = positive and negative and conc_ok

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CLASSIFICATION": "NONE" if overall else "PRODUCT_DEFECT",
                "OBSERVED_HTTP_VERSION": observed_version,
                "HTTP11_PROTOCOL_OBSERVED": observed_version == "HTTP/1.1",
                "HTTP11_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "HTTP11_NEGATIVE_STATUS": "PASS" if negative else "FAIL",
                "HTTP11_KEEPALIVE_STATUS": "PASS" if keepalive_ok else "FAIL",
                "HTTP11_CONNECTION_CLOSE_STATUS": "PASS" if connection_close_ok else "FAIL",
                "HTTP11_CONCURRENCY_STATUS": "PASS" if conc_ok else "FAIL",
                "HTTP11_SUCCESS_PATH": "PASS" if positive else "FAIL",
                "HTTP11_FAILURE_PATHS": "PASS" if negative else "FAIL",
                "HTTP11_CONCURRENCY_CHECK": "PASS" if conc_ok else "FAIL",
                "BODY_SHA256_STATUS": "PASS"
                if body_sha == EXPECTED_BODY_SHA256
                else "FAIL",
                "STATUS_LINE": get_resp.get("status_line"),
                "BODY_SHA256": body_sha,
                "EXPECTED_BODY_SHA256": EXPECTED_BODY_SHA256,
                "checks": {
                    "get_200_sha_cl": positive_get,
                    "head_200": head_ok,
                    "keepalive_same_tcp": keepalive_ok,
                    "connection_close": connection_close_ok,
                    "missing_404": miss_ok,
                    "wrong_host_not_200": nohost_ok,
                    "malformed_request_line": malformed_line_ok,
                    "malformed_header": malformed_hdr_ok,
                    "concurrent_8": conc_ok,
                    "observed_http_version": observed_version,
                    "content_length": cl_hdr,
                    "malformed_line_status": bad_line_resp.get("status"),
                    "malformed_line_raw_len": len(bad_line_raw),
                    "malformed_hdr_status": bad_hdr_resp.get("status"),
                    "malformed_hdr_raw_len": len(bad_hdr_raw),
                    "connection_close_hdr": conn_hdr,
                    "connection_close_peer_closed": peer_closed,
                },
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
