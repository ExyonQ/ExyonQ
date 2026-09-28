#!/usr/bin/env python3
"""Cap067 miss/NeedHyper hang REAL E2E (Linux process + real FS + real sockets).

Proves sendfile-eligible GET miss/rejection paths terminate with HTTP outcomes
(not curl timeout / 0 bytes). FD_REUSE_CAUSAL = NO — never-cached paths included.

ZERO_FAKE: real files, real sockets, real exyonq binary. No mock / synthetic oracle.
"""
from __future__ import annotations

import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH = os.environ.get("ARCH_LABEL", "unknown")
HOST = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

RESULTS: list[dict] = []
HOT_SIZE = 64 * 1024


def record(name: str, status: str, **extra) -> None:
    row = {"scenario": name, "status": status, **extra}
    RESULTS.append(row)
    print(f"[{status}] {name} {extra}", flush=True)


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_port(port: int, timeout: float = 60.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def curl(url: str, *, timeout: int = 5, path_as_is: bool = False) -> tuple[int, str, bytes, float]:
    tag = f"{time.time_ns()}"
    body_p = EV / f"c-{tag}.body"
    hdr_p = EV / f"c-{tag}.hdr"
    cmd = [
        "curl",
        "-sS",
        "--http1.1",
        "--max-time",
        str(timeout),
        "-D",
        str(hdr_p),
        "-o",
        str(body_p),
    ]
    if path_as_is:
        cmd.append("--path-as-is")
    cmd.append(url)
    t0 = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    elapsed = time.monotonic() - t0
    body = body_p.read_bytes() if body_p.is_file() else b""
    hdr = hdr_p.read_text(errors="replace") if hdr_p.is_file() else ""
    status = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
    return proc.returncode, status, body, elapsed


def http_code(status_line: str) -> str:
    parts = status_line.split()
    return parts[1] if len(parts) >= 2 else ""


def write_config(path: Path, port: int, root: Path) -> None:
    path.write_text(
        f"""config_version = 1

[logging]
level = "info"
format = "text"

[logging.console]
enabled = true
stream = "stdout"

[logging.access]
enabled = false

[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
index = "index.html"
""",
        encoding="utf-8",
    )


def finish(code: int) -> int:
    payload = {
        "arch": ARCH,
        "host": HOST,
        "head": HEAD,
        "binary": str(BIN),
        "binary_sha256": sha256(BIN.read_bytes()) if BIN.is_file() else "",
        "ts": datetime.now(timezone.utc).isoformat(),
        "results": RESULTS,
        "overall": "PASS" if code == 0 else "FAIL",
    }
    OUT.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return code


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BIN.is_file():
        record("binary", "FAIL", path=str(BIN))
        return finish(1)

    port = pick_port()
    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    (root / "hot.bin").write_bytes(b"HOT-FILE-V1" + bytes([0x41]) * (HOT_SIZE - 11))
    (root / "index.html").write_text("ok\n", encoding="utf-8")
    outside = EV / "outside-secret.txt"
    outside.write_bytes(b"TOP_SECRET_PAYLOAD_DO_NOT_LEAK")
    escape = root / "escape.link"
    if escape.exists() or escape.is_symlink():
        escape.unlink()
    escape.symlink_to(outside)

    sock = EV / f"exyonq-{os.getpid()}.sock"
    if sock.exists():
        sock.unlink()
    cfg = EV / "exyonq.toml"
    write_config(cfg, port, root)
    log = EV / "exyonq.log"
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    # Cap067 auto-on on Linux; explicit for clarity.
    env["EXYONQ_EPOLL_STATIC"] = "1"
    env["EXYONQ_EPOLL_SENDFILE"] = "1"

    with log.open("w") as lf:
        proc = subprocess.Popen(
            [str(BIN), "serve", "-c", str(cfg)],
            stdout=lf,
            stderr=subprocess.STDOUT,
            env=env,
            cwd=str(EV),
        )
    try:
        if not wait_port(port):
            record("listen", "FAIL", port=port)
            return finish(1)
        record("listen", "PASS", port=port)

        base = f"http://127.0.0.1:{port}"

        # A. NEVER_EXISTED_PATH
        rc, st, body, elapsed = curl(f"{base}/missing-never.bin", timeout=5)
        code = http_code(st)
        ok = rc == 0 and code == "404" and b"TOP_SECRET" not in body and elapsed < 4.5
        record(
            "never_existed",
            "PASS" if ok else "FAIL",
            http_status=code or st,
            response_bytes=len(body),
            curl_rc=rc,
            elapsed_s=round(elapsed, 3),
            timeout_occurred=rc != 0 and elapsed >= 4.5,
        )
        if not ok:
            return finish(1)

        # Warm hit then delete (B. DELETE_AFTER_EXISTING_FILE)
        rc0, st0, body0, _ = curl(f"{base}/hot.bin", timeout=5)
        if http_code(st0) != "200" or len(body0) != HOT_SIZE:
            record("warmup_before_delete", "FAIL", http_status=http_code(st0), n=len(body0))
            return finish(1)
        (root / "hot.bin").unlink()
        rc, st, body, elapsed = curl(f"{base}/hot.bin", timeout=5)
        code = http_code(st)
        ok = rc == 0 and code == "404" and elapsed < 4.5
        record(
            "delete_after_existing",
            "PASS" if ok else "FAIL",
            http_status=code or st,
            response_bytes=len(body),
            curl_rc=rc,
            elapsed_s=round(elapsed, 3),
            timeout_occurred=rc != 0 and elapsed >= 4.5,
        )
        if not ok:
            return finish(1)
        # recreate for later hit non-regression
        (root / "hot.bin").write_bytes(b"HOT-FILE-V3" + bytes([0x43]) * (HOT_SIZE - 11))

        # C. PATH_TRAVERSAL
        rc, st, body, elapsed = curl(f"{base}/../outside-secret.txt", timeout=5, path_as_is=True)
        code = http_code(st)
        ok = (
            rc == 0
            and code in ("403", "404")
            and b"TOP_SECRET_PAYLOAD" not in body
            and elapsed < 4.5
        )
        record(
            "path_traversal",
            "PASS" if ok else "FAIL",
            http_status=code or st,
            response_bytes=len(body),
            curl_rc=rc,
            elapsed_s=round(elapsed, 3),
            secret_disclosed=b"TOP_SECRET_PAYLOAD" in body,
            timeout_occurred=rc != 0 and elapsed >= 4.5,
        )
        if not ok:
            return finish(1)

        # D. OUT_OF_ROOT_SYMLINK
        rc, st, body, elapsed = curl(f"{base}/escape.link", timeout=5)
        code = http_code(st)
        ok = (
            rc == 0
            and code in ("403", "404")
            and b"TOP_SECRET_PAYLOAD" not in body
            and elapsed < 4.5
        )
        record(
            "out_of_root_symlink",
            "PASS" if ok else "FAIL",
            http_status=code or st,
            response_bytes=len(body),
            curl_rc=rc,
            elapsed_s=round(elapsed, 3),
            secret_disclosed=b"TOP_SECRET_PAYLOAD" in body,
            timeout_occurred=rc != 0 and elapsed >= 4.5,
        )
        if not ok:
            return finish(1)

        # Keepalive after miss: miss then valid on same TCP if contract allows
        s = socket.create_connection(("127.0.0.1", port), timeout=5)
        s.settimeout(5)
        try:

            def read_one_http(sock: socket.socket, *, head_only: bool = False) -> bytes:
                buf = b""
                while b"\r\n\r\n" not in buf:
                    chunk = sock.recv(8192)
                    if not chunk:
                        return buf
                    buf += chunk
                hdr, rest = buf.split(b"\r\n\r\n", 1)
                if head_only:
                    return hdr + b"\r\n\r\n"
                clen = 0
                for line in hdr.split(b"\r\n"):
                    if line.lower().startswith(b"content-length:"):
                        try:
                            clen = int(line.split(b":", 1)[1].strip())
                        except ValueError:
                            clen = 0
                while len(rest) < clen:
                    chunk = sock.recv(8192)
                    if not chunk:
                        break
                    rest += chunk
                return hdr + b"\r\n\r\n" + rest[:clen]

            s.sendall(b"GET /missing-ka.bin HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n")
            first = read_one_http(s)
            first_ok = first.startswith(b"HTTP/1.1 404 ")
            s.sendall(b"GET /hot.bin HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            second = read_one_http(s)
            second_ok = second.startswith(b"HTTP/1.1 200 ") and b"HOT-FILE-V3" in second
            ok = first_ok and second_ok
            record(
                "keepalive_after_miss",
                "PASS" if ok else "FAIL",
                first_status=first.split(b"\r\n", 1)[0].decode("latin1", errors="replace"),
                second_status=second.split(b"\r\n", 1)[0].decode("latin1", errors="replace")
                if second
                else "",
                first_bytes=len(first),
                second_bytes=len(second),
            )
            if not ok:
                return finish(1)

            # HEAD miss then GET hit on same TCP (LA-CAP067-MISS-001)
            s2 = socket.create_connection(("127.0.0.1", port), timeout=5)
            s2.settimeout(5)
            try:
                s2.sendall(
                    b"HEAD /missing-head.bin HTTP/1.1\r\nHost: x\r\nConnection: keep-alive\r\n\r\n"
                )
                head_resp = read_one_http(s2, head_only=True)
                head_ok = head_resp.startswith(b"HTTP/1.1 404 ") and head_resp.endswith(
                    b"\r\n\r\n"
                )
                # Ensure no body leaked into the next read window: send GET immediately.
                s2.sendall(b"GET /hot.bin HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
                after = read_one_http(s2)
                after_ok = after.startswith(b"HTTP/1.1 200 ") and b"HOT-FILE-V3" in after
                ok_h = head_ok and after_ok
                record(
                    "head_miss_then_get_keepalive",
                    "PASS" if ok_h else "FAIL",
                    head_status=head_resp.split(b"\r\n", 1)[0].decode("latin1", errors="replace"),
                    after_status=after.split(b"\r\n", 1)[0].decode("latin1", errors="replace")
                    if after
                    else "",
                    head_wire_len=len(head_resp),
                )
                if not ok_h:
                    return finish(1)
            finally:
                s2.close()
        finally:
            s.close()

        # Hit-path non-regression smoke (correctness, not perf ratios)
        rc, st, body, elapsed = curl(f"{base}/hot.bin", timeout=5)
        ok = rc == 0 and http_code(st) == "200" and len(body) == HOT_SIZE
        record(
            "fd_reuse_hit_path_smoke",
            "PASS" if ok else "FAIL",
            http_status=http_code(st),
            response_bytes=len(body),
        )
        if not ok:
            return finish(1)

        return finish(0)
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
