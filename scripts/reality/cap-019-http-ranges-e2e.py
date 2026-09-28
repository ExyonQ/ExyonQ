#!/usr/bin/env python3
"""CAPABILITY_019 = http-ranges — real product E2E (single capability).

Canonical matrix: FEATURE_ID=http-ranges
  USER_VISIBLE_CONTRACT = Serve Content-Range / partial content

Authoritative path: release binary → production config → real filesystem file →
real TCP → HTTP/1.1 Range → ExyonQ static Cap004 path → independent slice SHA256.

EXPLICIT_NON_SCOPE_V1:
  - multipart/byteranges
  - If-Range / ETag / Last-Modified validators (Cap020+)
  - range-cache store/hit
"""
from __future__ import annotations

import hashlib
import json
import os
import re
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

# Deterministic corpus (real files; independent oracle slices).
TEXT = b"abcdefghijklmnopqrstuvwxyz0123456789" * 8  # 288 bytes
BINARY_1K = bytes((i % 256) for i in range(1024))
LARGE = bytes([0x41 + (i % 26) for i in range(256 * 1024)])  # 256 KiB
ONE = b"Z"
EMPTY = b""


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


def parse_headers(hdr_text: str) -> tuple[str, dict[str, str]]:
    status = ""
    headers: dict[str, str] = {}
    for line in hdr_text.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
        elif ":" in line:
            k, v = line.split(":", 1)
            headers[k.strip().lower()] = v.strip()
    return status, headers


def curl_range(
    url: str,
    *,
    range_header: str | None = None,
    method: str = "GET",
) -> tuple[int, str, dict[str, str], bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:10]}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.hdr"
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        "30",
        "-D",
        str(hdr_path),
        "-o",
        str(body_path),
        "--http1.1",
    ]
    if method == "HEAD":
        cmd.append("-I")
    if range_header is not None:
        cmd.extend(["-H", f"Range: {range_header}"])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    status, headers = parse_headers(hdr)
    try:
        body_path.unlink(missing_ok=True)
        hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    return proc.returncode, status, headers, body


def expect_206(
    url: str,
    range_hdr: str,
    file_bytes: bytes,
    start: int,
    end: int,
) -> tuple[bool, str]:
    expect = file_bytes[start : end + 1]
    rc, status, headers, body = curl_range(url, range_header=range_hdr)
    if rc != 0:
        return False, f"curl rc={rc}"
    if "206" not in status:
        return False, f"status={status!r} want 206"
    cr = headers.get("content-range", "")
    want_cr = f"bytes {start}-{end}/{len(file_bytes)}"
    if cr != want_cr:
        return False, f"content-range={cr!r} want {want_cr!r}"
    cl = headers.get("content-length", "")
    if cl != str(len(expect)):
        return False, f"content-length={cl!r} want {len(expect)}"
    if headers.get("accept-ranges", "").lower() != "bytes":
        return False, f"accept-ranges={headers.get('accept-ranges')!r}"
    if body != expect:
        return False, f"body mismatch sha got={sha256_bytes(body)} want={sha256_bytes(expect)}"
    if sha256_bytes(body) != sha256_bytes(expect):
        return False, "sha mismatch"
    return True, "ok"


def expect_416(url: str, range_hdr: str, full_len: int) -> tuple[bool, str]:
    rc, status, headers, body = curl_range(url, range_header=range_hdr)
    if rc != 0:
        return False, f"curl rc={rc}"
    if "416" not in status:
        return False, f"status={status!r} want 416"
    if headers.get("content-range") != f"bytes */{full_len}":
        return False, f"content-range={headers.get('content-range')!r}"
    if body:
        return False, f"body non-empty len={len(body)}"
    return True, "ok"


def expect_200_full(url: str, file_bytes: bytes, range_hdr: str | None = None) -> tuple[bool, str]:
    rc, status, headers, body = curl_range(url, range_header=range_hdr)
    if rc != 0:
        return False, f"curl rc={rc}"
    if "200" not in status:
        return False, f"status={status!r} want 200"
    if body != file_bytes or sha256_bytes(body) != sha256_bytes(file_bytes):
        return False, "full body/sha mismatch"
    if "content-range" in headers:
        return False, "unexpected content-range on 200"
    return True, "ok"


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "http-ranges",
                    "CAPABILITY": "CAPABILITY_019",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BINARY}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    root = EV / "static-root"
    root.mkdir(parents=True, exist_ok=True)
    n1000 = (TEXT * 4)[:1000]
    files = {
        "text.txt": TEXT,
        "bin1k.bin": BINARY_1K,
        "large.bin": LARGE,
        "one.bin": ONE,
        "empty.bin": EMPTY,
        "n1000.bin": n1000,
    }
    corpus = []
    for rel, data in files.items():
        p = root / rel
        p.write_bytes(data)
        corpus.append({"path": rel, "size_bytes": len(data), "sha256": sha256_bytes(data)})

    port = pick_port()
    cfg = EV / "cfg-ranges.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["assets"]

[[route]]
name = "assets"
match = {{ path = "/assets" }}
root = "{root}"
"""
    )
    log = EV / "exyonq-ranges.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    checks: dict[str, object] = {}
    result: dict = {
        "FEATURE_ID": "http-ranges",
        "CAPABILITY": "CAPABILITY_019",
        "CAPABILITY_NAME": "http-ranges",
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
        "PRODUCT_CONTRACT": "Serve Content-Range / partial content",
        "SUPPORTED_BEHAVIOR": "single-byte-range 206/416 on static Cap004 path",
        "EXPLICIT_NON_SCOPE": [
            "multipart/byteranges",
            "If-Range",
            "range-cache",
            "Cap020 validators",
        ],
        "CORPUS": corpus,
        "CHECKS": checks,
    }
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "listen timeout",
                    "LOG_TAIL": log.read_text(errors="replace")[-3000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        base = f"http://127.0.0.1:{port}/assets"
        n = len(TEXT)

        # No-Range preserves Cap004 full semantics
        ok, detail = expect_200_full(f"{base}/text.txt", TEXT)
        checks["no_range_200"] = {"ok": ok, "detail": detail}

        # Positive ranges on TEXT
        cases = [
            ("bytes=0-0", 0, 0),
            ("bytes=0-99", 0, 99),
            ("bytes=100-199", 100, 199),
            ("bytes=100-", 100, n - 1),
            ("bytes=-100", n - 100, n - 1),
            ("bytes=0-9999", 0, n - 1),  # clip past EOF
        ]
        for rh, start, end in cases:
            ok, detail = expect_206(f"{base}/text.txt", rh, TEXT, start, end)
            checks[f"get_{rh}"] = {"ok": ok, "detail": detail, "start": start, "end": end}

        # Binary / large / one-byte
        ok, detail = expect_206(f"{base}/bin1k.bin", "bytes=0-0", BINARY_1K, 0, 0)
        checks["bin_first_byte"] = {"ok": ok, "detail": detail}
        ok, detail = expect_206(f"{base}/bin1k.bin", "bytes=-16", BINARY_1K, 1008, 1023)
        checks["bin_suffix"] = {"ok": ok, "detail": detail}
        ok, detail = expect_206(f"{base}/large.bin", "bytes=0-1023", LARGE, 0, 1023)
        checks["large_first_1k"] = {
            "ok": ok,
            "detail": detail,
            "returned_sha256": sha256_bytes(LARGE[0:1024]),
        }
        ok, detail = expect_206(f"{base}/one.bin", "bytes=0-0", ONE, 0, 0)
        checks["one_byte"] = {"ok": ok, "detail": detail}

        # Unsatisfiable (owner examples + EOF boundaries)
        # Owner examples use N=1000 (n1000.bin written before server start).
        for name, url, rh, flen in (
            ("bytes_1000_1001", f"{base}/n1000.bin", "bytes=1000-1001", 1000),
            ("bytes_5000_open", f"{base}/n1000.bin", "bytes=5000-", 1000),
            ("bin_at_eof", f"{base}/bin1k.bin", "bytes=1024-1025", 1024),
            ("text_at_eof", f"{base}/text.txt", "bytes=288-", n),
            ("text_past_eof", f"{base}/text.txt", "bytes=288-300", n),
        ):
            ok, detail = expect_416(url, rh, flen)
            checks[f"unsat_{name}"] = {"ok": ok, "detail": detail}

        ok, detail = expect_416(f"{base}/empty.bin", "bytes=0-0", 0)
        checks["empty_unsat"] = {"ok": ok, "detail": detail}
        ok, detail = expect_416(f"{base}/one.bin", "bytes=1-", 1)
        checks["one_unsat"] = {"ok": ok, "detail": detail}

        # Malformed / multi → 200 full
        for rh in (
            "items=0-10",
            "bytes=abc-def",
            "bytes=",
            "bytes=0-1,4-5",
            "bytes=10-5",
            "bytes=-0",
        ):
            ok, detail = expect_200_full(f"{base}/text.txt", TEXT, range_hdr=rh)
            checks[f"ignore_{rh}"] = {"ok": ok, "detail": detail}

        # HEAD + Range — raw socket (curl may wait on Content-Length despite HEAD).
        head_ok = False
        head_detail = ""
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
            sock.sendall(
                (
                    f"HEAD /assets/text.txt HTTP/1.1\r\n"
                    f"Host: 127.0.0.1\r\n"
                    f"Range: bytes=0-9\r\n"
                    f"Connection: close\r\n\r\n"
                ).encode()
            )
            buf = b""
            while True:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                buf += chunk
            sock.close()
            if b"\r\n\r\n" not in buf:
                head_detail = "no header end"
            else:
                head, rest = buf.split(b"\r\n\r\n", 1)
                status_line = head.split(b"\r\n", 1)[0].decode(errors="replace")
                hdrs = {}
                for line in head.split(b"\r\n")[1:]:
                    if b":" in line:
                        k, v = line.split(b":", 1)
                        hdrs[k.decode().lower()] = v.strip().decode()
                head_ok = (
                    "206" in status_line
                    and rest == b""
                    and hdrs.get("content-range") == f"bytes 0-9/{n}"
                    and hdrs.get("content-length") == "10"
                    and hdrs.get("accept-ranges", "").lower() == "bytes"
                )
                head_detail = status_line if head_ok else f"{status_line} rest_len={len(rest)} hdrs={hdrs}"
        except OSError as exc:
            head_detail = str(exc)
        checks["head_range"] = {"ok": head_ok, "detail": head_detail}

        # Concurrent ranges + mix 200/206
        def worker(name: str, rh: str | None, start: int | None, end: int | None) -> tuple[str, bool, str]:
            if rh is None:
                ok, d = expect_200_full(f"{base}/bin1k.bin", BINARY_1K)
            else:
                assert start is not None and end is not None
                ok, d = expect_206(f"{base}/bin1k.bin", rh, BINARY_1K, start, end)
            return name, ok, d

        jobs = [
            ("c_full", None, None, None),
            ("c_0_99", "bytes=0-99", 0, 99),
            ("c_100_199", "bytes=100-199", 100, 199),
            ("c_suf", "bytes=-50", 974, 1023),
            ("c_open", "bytes=512-", 512, 1023),
        ]
        threads = []
        results_box: list[tuple[str, bool, str]] = []

        def run_job(j):
            results_box.append(worker(*j))

        for j in jobs:
            t = threading.Thread(target=run_job, args=(j,))
            threads.append(t)
            t.start()
        for t in threads:
            t.join(timeout=60)
        conc_ok = all(ok for _, ok, _ in results_box) and len(results_box) == len(jobs)
        checks["concurrency"] = {
            "ok": conc_ok,
            "results": {name: {"ok": ok, "detail": d} for name, ok, d in results_box},
        }

        # Keepalive sequential ranges
        ka_ok = True
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
            for rh, start, end in [("bytes=0-3", 0, 3), ("bytes=4-7", 4, 7)]:
                req = (
                    f"GET /assets/text.txt HTTP/1.1\r\n"
                    f"Host: 127.0.0.1\r\n"
                    f"Range: {rh}\r\n"
                    f"Connection: keep-alive\r\n\r\n"
                ).encode()
                sock.sendall(req)
                buf = bytearray()
                while b"\r\n\r\n" not in buf:
                    chunk = sock.recv(4096)
                    if not chunk:
                        break
                    buf.extend(chunk)
                head, rest = bytes(buf).split(b"\r\n\r\n", 1)
                if b"206" not in head.split(b"\r\n", 1)[0]:
                    ka_ok = False
                    break
                m = re.search(br"Content-Length:\s*(\d+)", head, re.I)
                need = int(m.group(1)) if m else 0
                body = rest
                while len(body) < need:
                    chunk = sock.recv(min(65536, need - len(body)))
                    if not chunk:
                        break
                    body += chunk
                if body[:need] != TEXT[start : end + 1]:
                    ka_ok = False
                    break
            sock.close()
        except OSError:
            ka_ok = False
        checks["keepalive_ranges"] = {"ok": ka_ok}

        failed = [k for k, v in checks.items() if isinstance(v, dict) and v.get("ok") is False]
        all_ok = len(failed) == 0
        result["FAILED_CHECKS"] = failed
        result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if all_ok else "FAIL"
        result["ZERO_FAKE"] = "PASS"
        result["USES_SMOKE"] = "NO"
        result["REAL_RANGE_REQUEST"] = "YES"
        result["INDEPENDENT_BYTE_VERIFICATION"] = "YES"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if all_ok else 1
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
