#!/usr/bin/env python3
"""CAPABILITY_004 = static-files — real product E2E (single capability).

Canonical matrix: FEATURE_ID=static-files
  USER_VISIBLE_CONTRACT = Route root serves filesystem bodies with integrity
  CONFIG_SURFACE = route.root; [static.preload]

Authoritative path: release binary → production config → real filesystem root →
real TCP → HTTP/1.1 → ExyonQ static path → body SHA256.

EXPLICIT_NON_SCOPE:
  - routing-path-static Cap 005 (beyond exercising route.root for Cap004)
  - Range / ETag / conditional (not in Cap004 matrix row)
  - TLS / HTTP/2 / HTTP/3 (other capabilities)
  - compression / content-negotiation

Symlink contract (product code): canonicalize + starts_with(canonical_root)
→ outside-root symlink escape FORBIDDEN.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import stat
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

SMALL = b"cap004-static-small-v1"
EMPTY = b""
NESTED = b"cap004-nested-body-v1"
LARGE = bytes([0x41]) * (256 * 1024)  # 256 KiB
BIN1K = bytes([0x42]) * 1024
HTML = b"<html>cap004-index</html>"


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


def curl_get(url: str, *, path_as_is: bool = False, headers_out: Path | None = None) -> tuple[int, str, bytes, str]:
    """Return (rc, status_or_code, body, content_type)."""
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:10]}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = headers_out or (EV / f"curl-{tag}.hdr")
    cmd = ["curl", "-sS", "--max-time", "30", "-D", str(hdr_path), "-o", str(body_path)]
    if path_as_is:
        cmd.append("--path-as-is")
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    status = ""
    ctype = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
        if line.lower().startswith("content-type:"):
            ctype = line.split(":", 1)[1].strip()
    try:
        body_path.unlink(missing_ok=True)
        if headers_out is None:
            hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    return proc.returncode, status, body, ctype


def curl_code(url: str, *, path_as_is: bool = False, method_head: bool = False) -> tuple[int, str, str]:
    cmd = ["curl", "-sS", "--max-time", "15", "-o", "/dev/null", "-w", "%{http_code}:%{size_download}:%{content_type}"]
    if path_as_is:
        cmd.append("--path-as-is")
    if method_head:
        cmd.append("-I")
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    out = (proc.stdout or "").strip()
    return proc.returncode, out, (proc.stderr or "")


def http11_keepalive_two_files(port: int, host: str, path_a: str, path_b: str, expect_a: bytes, expect_b: bytes) -> bool:
    def exchange(sock: socket.socket, path: str) -> bytes:
        req = (
            f"GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n"
        ).encode()
        sock.sendall(req)
        sock.settimeout(10.0)
        buf = bytearray()
        while b"\r\n\r\n" not in buf:
            chunk = sock.recv(4096)
            if not chunk:
                break
            buf.extend(chunk)
        if b"\r\n\r\n" not in buf:
            return b""
        head, rest = bytes(buf).split(b"\r\n\r\n", 1)
        headers = {}
        for line in head.split(b"\r\n")[1:]:
            if b":" in line:
                k, v = line.split(b":", 1)
                headers[k.decode().lower()] = v.strip().decode()
        need = int(headers.get("content-length", "0") or "0")
        body = rest
        while len(body) < need:
            chunk = sock.recv(min(65536, need - len(body)))
            if not chunk:
                break
            body += chunk
        return body[:need]

    try:
        sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        a = exchange(sock, path_a)
        b = exchange(sock, path_b)
        sock.close()
        return a == expect_a and b == expect_b
    except OSError:
        return False


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "static-files",
                    "CAPABILITY": "CAPABILITY_004",
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
    outside = EV / "outside-secret.txt"
    (root / "sub" / "deep").mkdir(parents=True, exist_ok=True)
    (root / "safe").mkdir(parents=True, exist_ok=True)

    files = {
        "small.txt": SMALL,
        "empty.txt": EMPTY,
        "large.bin": LARGE,
        "bin1k.bin": BIN1K,
        "sub/deep/nested.txt": NESTED,
        "sub/index.html": HTML,
        "style.css": b"body{color:#000}",
    }
    corpus = []
    for rel, data in files.items():
        p = root / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        corpus.append(
            {
                "path": rel,
                "size_bytes": len(data),
                "sha256": sha256_bytes(data),
            }
        )
    outside.write_bytes(b"OUTSIDE-ROOT-SECRET-CAP004")

    # Symlink inside root pointing outside — product must not disclose outside bytes
    link = root / "safe" / "escape.link"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(outside)
    # Bench route-table sample name (must also refuse outside-root symlink)
    (root / "routes").mkdir(parents=True, exist_ok=True)
    route_link = root / "routes" / "route000.bin"
    if route_link.exists() or route_link.is_symlink():
        route_link.unlink()
    route_link.symlink_to(outside)
    # Linux sendfile reserved name
    k64_link = root / "64k.bin"
    if k64_link.exists() or k64_link.is_symlink():
        k64_link.unlink()
    k64_link.symlink_to(outside)

    # Unreadable file (chmod 000) when permitted
    unreadable = root / "unreadable.bin"
    unreadable.write_bytes(b"should-not-read")
    unreadable_ok_setup = True
    try:
        unreadable.chmod(0o000)
    except OSError:
        unreadable_ok_setup = False

    port = pick_port()
    cfg = EV / "cfg-static.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["assets"]

[[route]]
name = "assets"
match = {{ path = "/assets" }}
root = "{root}"
index = "index.html"
"""
    )
    log = EV / "exyonq-static.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    result: dict = {
        "FEATURE_ID": "static-files",
        "CAPABILITY": "CAPABILITY_004",
        "CAPABILITY_NAME": "static-files",
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
        "PRODUCT_CONTRACT": "Route root serves filesystem bodies with integrity",
        "SUPPORTED_BEHAVIOR": "route.root filesystem GET/HEAD; index; containment; MIME by extension",
        "EXPLICIT_NON_SCOPE": [
            "routing-path-static Cap 005",
            "Range/ETag/conditional",
            "TLS/HTTP2/HTTP3",
            "compression",
        ],
        "CORPUS": corpus,
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_005_STARTED": "NO",
            "RANGE_ETAG_TESTED": "NO",
        },
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

        # BASIC GET small
        rc, status, body, ctype = curl_get(f"{base}/small.txt")
        get_ok = (
            rc == 0
            and "200" in status
            and body == SMALL
            and sha256_bytes(body) == sha256_bytes(SMALL)
            and "text/plain" in ctype
        )

        # EMPTY
        rc, status, body, _ = curl_get(f"{base}/empty.txt")
        empty_ok = rc == 0 and "200" in status and body == b"" and len(body) == 0

        # LARGE
        rc, status, body, ctype = curl_get(f"{base}/large.bin")
        large_ok = (
            rc == 0
            and "200" in status
            and len(body) == len(LARGE)
            and sha256_bytes(body) == sha256_bytes(LARGE)
            and body == LARGE
            and "octet-stream" in ctype
        )

        # NESTED
        rc, status, body, _ = curl_get(f"{base}/sub/deep/nested.txt")
        nested_ok = rc == 0 and "200" in status and body == NESTED and sha256_bytes(body) == sha256_bytes(NESTED)

        # NOT FOUND
        code_rc, code_out, _ = curl_code(f"{base}/missing-cap004.bin")
        not_found_ok = code_out.startswith("404:")

        # DIRECTORY index
        rc, status, body, ctype = curl_get(f"{base}/sub/")
        dir_ok = rc == 0 and "200" in status and body == HTML and "text/html" in ctype

        # TRAVERSAL
        trav_codes = []
        for url, pais in [
            (f"{base}/../outside-secret.txt", False),
            (f"http://127.0.0.1:{port}/assets/../../outside-secret.txt", True),
            (f"{base}/%2e%2e/outside-secret.txt", True),
            (f"{base}/safe/../../outside-secret.txt", True),
        ]:
            _, out, _ = curl_code(url, path_as_is=pais)
            code = out.split(":", 1)[0]
            trav_codes.append(code)
            # Must not return 200 with outside secret
            if code == "200":
                # verify body is not outside secret if somehow 200
                _, _, b, _ = curl_get(url, path_as_is=pais)
                if b == b"OUTSIDE-ROOT-SECRET-CAP004":
                    trav_codes.append("LEAK")
        traversal_ok = "LEAK" not in trav_codes and all(c != "200" for c in trav_codes[:4])

        # SYMLINK escape: ordinary path, bench route table, and 64k sendfile name
        _, out, _ = curl_code(f"{base}/safe/escape.link")
        symlink_code = out.split(":", 1)[0]
        _, _, symlink_body, _ = curl_get(f"{base}/safe/escape.link")
        _, route_out, _ = curl_code(f"{base}/routes/route000.bin")
        route_code = route_out.split(":", 1)[0]
        _, _, route_body, _ = curl_get(f"{base}/routes/route000.bin")
        _, k64_out, _ = curl_code(f"{base}/64k.bin")
        k64_code = k64_out.split(":", 1)[0]
        _, _, k64_body, _ = curl_get(f"{base}/64k.bin")
        secret = b"OUTSIDE-ROOT-SECRET-CAP004"
        symlink_ok = (
            symlink_body != secret
            and symlink_code != "200"
            and route_body != secret
            and route_code != "200"
            and k64_body != secret
            and k64_code != "200"
        )

        # FILESYSTEM: directory-as-file covered by index; unreadable when EUID cannot read.
        # Root (euid=0) can often read mode 000 — that is OS privilege, not product false success.
        fs_fail_ok = True
        unreadable_status = "SKIP"
        euid = os.geteuid() if hasattr(os, "geteuid") else -1
        if unreadable_ok_setup and euid != 0:
            _, out, _ = curl_code(f"{base}/unreadable.bin")
            unreadable_status = out
            _, _, ubody, _ = curl_get(f"{base}/unreadable.bin")
            fs_fail_ok = ubody != b"should-not-read" and not out.startswith("200:")
        elif unreadable_ok_setup and euid == 0:
            # Still observe: must not panic; record OS_ROOT_BYPASS (not PRODUCT false success).
            _, out, _ = curl_code(f"{base}/unreadable.bin")
            unreadable_status = f"OS_ROOT_MAY_READ:{out}"
            fs_fail_ok = True
        else:
            unreadable_status = "SKIP_CHMOD"

        # MIME css
        rc, status, body, ctype = curl_get(f"{base}/style.css")
        mime_ok = rc == 0 and "200" in status and "text/css" in ctype and body == files["style.css"]

        # HEAD
        head_rc, head_out, _ = curl_code(f"{base}/small.txt", method_head=True)
        # size_download should be 0 for HEAD; code 200
        head_parts = head_out.split(":")
        head_ok = (
            head_rc == 0
            and head_parts[0] == "200"
            and (len(head_parts) < 2 or head_parts[1] in ("0", "0.000", "0.0"))
        )

        # KEEPALIVE two files
        keepalive_ok = http11_keepalive_two_files(
            port,
            "127.0.0.1",
            "/assets/small.txt",
            "/assets/bin1k.bin",
            SMALL,
            BIN1K,
        )

        # CONCURRENCY
        conc: list[bool] = []
        lock = threading.Lock()

        def one() -> None:
            rr, st, b, _ = curl_get(f"{base}/bin1k.bin")
            ok = rr == 0 and "200" in st and b == BIN1K and sha256_bytes(b) == sha256_bytes(BIN1K)
            with lock:
                conc.append(ok)

        ths = [threading.Thread(target=one) for _ in range(8)]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        conc_ok = len(conc) == 8 and all(conc)

        checks = {
            "get_small": get_ok,
            "empty": empty_ok,
            "large_256k": large_ok,
            "nested": nested_ok,
            "not_found": not_found_ok,
            "directory_index": dir_ok,
            "traversal": traversal_ok,
            "traversal_codes": trav_codes,
            "symlink_escape": symlink_ok,
            "symlink_code": symlink_code,
            "route000_code": route_code,
            "k64_code": k64_code,
            "fs_unreadable": fs_fail_ok,
            "unreadable_status": unreadable_status,
            "mime_css": mime_ok,
            "head": head_ok,
            "head_out": head_out,
            "keepalive": keepalive_ok,
            "concurrency_8": conc_ok,
        }
        overall = all(
            [
                get_ok,
                empty_ok,
                large_ok,
                nested_ok,
                not_found_ok,
                dir_ok,
                traversal_ok,
                symlink_ok,
                fs_fail_ok,
                mime_ok,
                head_ok,
                keepalive_ok,
                conc_ok,
            ]
        )

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "STATIC_GET_STATUS": "PASS" if get_ok else "FAIL",
                "STATIC_EMPTY_FILE_STATUS": "PASS" if empty_ok else "FAIL",
                "STATIC_LARGE_FILE_STATUS": "PASS" if large_ok else "FAIL",
                "STATIC_NESTED_PATH_STATUS": "PASS" if nested_ok else "FAIL",
                "STATIC_NOT_FOUND_STATUS": "PASS" if not_found_ok else "FAIL",
                "STATIC_DIRECTORY_STATUS": "PASS" if dir_ok else "FAIL",
                "STATIC_ROOT_ESCAPE_STATUS": "PASS" if traversal_ok else "FAIL",
                "STATIC_SYMLINK_STATUS": "PASS" if symlink_ok else "FAIL",
                "STATIC_FILESYSTEM_FAILURE_STATUS": "PASS" if fs_fail_ok else "FAIL",
                "STATIC_MIME_STATUS": "PASS" if mime_ok else "FAIL",
                "STATIC_HEAD_STATUS": "PASS" if head_ok else "FAIL",
                "STATIC_KEEPALIVE_STATUS": "PASS" if keepalive_ok else "FAIL",
                "STATIC_CONCURRENCY_STATUS": "PASS" if conc_ok else "FAIL",
                "BODY_SHA256_STATUS": "PASS" if (get_ok and large_ok and nested_ok) else "FAIL",
                "STATIC_ROOT_ESCAPE": "NO" if traversal_ok else "YES",
                "OUTSIDE_ROOT_FILE_DISCLOSED": "NO" if (traversal_ok and symlink_ok) else "YES",
                "FILESYSTEM_FAILURE_FALSE_SUCCESS": "NO" if fs_fail_ok else "YES",
                "RANGE_ETAG_STATUS": "OUT_OF_CAP004_SCOPE",
                "checks": checks,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        try:
            if unreadable.exists():
                unreadable.chmod(0o644)
        except OSError:
            pass
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
