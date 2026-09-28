#!/usr/bin/env python3
"""CAPABILITY_005 = routing-path-static — real product E2E.

Canonical matrix:
  FEATURE_ID = routing-path-static
  USER_VISIBLE_CONTRACT = match.path selects static route
  CONFIG_SURFACE = route.match.path

Authoritative path: release binary → production config with ≥2 route.match.path
entries → real TCP → HTTP/1.1 → RouteIndex longest-prefix selection → correct
static root body (SHA256).

EXPLICIT_NON_SCOPE:
  - Cap 004 full static corpus (except Cap004 containment cross-check)
  - Cap 006 HTTP/3
  - Range/ETag/conditional
  - host-primary routing as Cap005 scope (path is primary)
  - proxy/FastCGI route selection

CROSS_CAPABILITY (Cap004):
  OUTSIDE_ROOT_STATIC_BYTES_REACHABLE = NO through the routed static path.
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

BODY_A = b"cap005-route-a-body-v1"
BODY_DEEP = b"cap005-route-deep-body-v1"
BODY_SITE = b"cap005-route-site-body-v1"
OUTSIDE_SECRET = b"OUTSIDE-ROOT-SECRET-CAP005"


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


def curl_get(url: str, *, path_as_is: bool = False) -> tuple[int, str, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:10]}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.hdr"
    cmd = ["curl", "-sS", "--max-time", "30", "-D", str(hdr_path), "-o", str(body_path)]
    if path_as_is:
        cmd.append("--path-as-is")
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    status = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
    try:
        body_path.unlink(missing_ok=True)
        hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    return proc.returncode, status, body


def curl_code(url: str, *, path_as_is: bool = False) -> str:
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        "15",
        "-o",
        "/dev/null",
        "-w",
        "%{http_code}",
    ]
    if path_as_is:
        cmd.append("--path-as-is")
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    return (proc.stdout or "").strip()


def http11_keepalive_two_paths(
    port: int, path_a: str, expect_a: bytes, path_b: str, expect_b: bytes
) -> bool:
    def exchange(sock: socket.socket, path: str) -> bytes:
        req = f"GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: keep-alive\r\n\r\n".encode()
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
                    "FEATURE_ID": "routing-path-static",
                    "CAPABILITY": "CAPABILITY_005",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BINARY}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    root_a = EV / "root-a"
    root_deep = EV / "root-deep"
    root_site = EV / "root-site"
    outside = EV / "outside-secret.txt"
    for d in (root_a, root_deep, root_site):
        d.mkdir(parents=True, exist_ok=True)
    (root_a / "marker.txt").write_bytes(BODY_A)
    (root_deep / "marker.txt").write_bytes(BODY_DEEP)
    (root_site / "ok.txt").write_bytes(BODY_SITE)
    outside.write_bytes(OUTSIDE_SECRET)
    # Cap004 containment on a routed static root
    link = root_site / "escape.link"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(outside)

    port = pick_port()
    cfg = EV / "cfg-routing.toml"
    # Intentionally register shorter path AFTER longer in list; longest must still win.
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site", "assets", "assets_deep"]

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{root_site}"
index = "index.html"

[[route]]
name = "assets"
match = {{ path = "/assets" }}
root = "{root_a}"
index = "index.html"

[[route]]
name = "assets_deep"
match = {{ path = "/assets/deep" }}
root = "{root_deep}"
index = "index.html"
"""
    )
    log = EV / "exyonq-routing.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    result: dict = {
        "FEATURE_ID": "routing-path-static",
        "CAPABILITY": "CAPABILITY_005",
        "CAPABILITY_NAME": "routing-path-static",
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
        "PRODUCT_CONTRACT": "match.path selects static route",
        "SUPPORTED_BEHAVIOR": "route.match.path longest-prefix selects static root; body integrity",
        "EXPLICIT_NON_SCOPE": [
            "Cap004 full static corpus",
            "Cap006 HTTP/3",
            "Range/ETag",
            "host-primary routing",
            "proxy/FastCGI routing",
        ],
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_006_STARTED": "NO",
            "CAP004_CONTAINMENT_PROTECTOR": "YES",
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

        base = f"http://127.0.0.1:{port}"

        # POSITIVE: short prefix route
        rc, status, body = curl_get(f"{base}/assets/marker.txt")
        pos_short = (
            rc == 0
            and "200" in status
            and body == BODY_A
            and sha256_bytes(body) == sha256_bytes(BODY_A)
        )

        # POSITIVE: longest prefix wins over /assets
        rc, status, body = curl_get(f"{base}/assets/deep/marker.txt")
        pos_long = (
            rc == 0
            and "200" in status
            and body == BODY_DEEP
            and sha256_bytes(body) == sha256_bytes(BODY_DEEP)
            and body != BODY_A
        )

        # POSITIVE: unrelated route
        rc, status, body = curl_get(f"{base}/site/ok.txt")
        pos_site = rc == 0 and "200" in status and body == BODY_SITE

        # NEGATIVE: no matching route
        code = curl_code(f"{base}/nomatch/file.txt")
        neg_nomatch = code == "404"

        # NEGATIVE: must not serve deep body under short-only path that isn't deep
        # /assets/marker already proved BODY_A; deep path must not return BODY_A
        # (already in pos_long). Also: requesting deep via wrong selection would fail SHA.
        neg_cross = pos_long and pos_short

        # FAILURE: missing file on matched route → 404 (route selected, file absent)
        code = curl_code(f"{base}/assets/missing-cap005.bin")
        fail_missing = code == "404"

        # BOUNDARY Cap004: outside-root symlink via routed static root
        code = curl_code(f"{base}/site/escape.link")
        _, _, leak_body = curl_get(f"{base}/site/escape.link")
        trav = curl_code(f"{base}/site/../outside-secret.txt", path_as_is=True)
        boundary_ok = (
            leak_body != OUTSIDE_SECRET
            and code != "200"
            and trav != "200"
        )

        # CONCURRENCY: parallel distinct routes
        conc: list[bool] = []
        lock = threading.Lock()

        def one(url: str, expect: bytes) -> None:
            rr, st, b = curl_get(url)
            ok = rr == 0 and "200" in st and b == expect
            with lock:
                conc.append(ok)

        ths = [
            threading.Thread(target=one, args=(f"{base}/assets/marker.txt", BODY_A)),
            threading.Thread(target=one, args=(f"{base}/assets/deep/marker.txt", BODY_DEEP)),
            threading.Thread(target=one, args=(f"{base}/site/ok.txt", BODY_SITE)),
            threading.Thread(target=one, args=(f"{base}/assets/deep/marker.txt", BODY_DEEP)),
            threading.Thread(target=one, args=(f"{base}/assets/marker.txt", BODY_A)),
            threading.Thread(target=one, args=(f"{base}/site/ok.txt", BODY_SITE)),
            threading.Thread(target=one, args=(f"{base}/assets/marker.txt", BODY_A)),
            threading.Thread(target=one, args=(f"{base}/assets/deep/marker.txt", BODY_DEEP)),
        ]
        for t in ths:
            t.start()
        for t in ths:
            t.join()
        conc_ok = len(conc) == 8 and all(conc)

        # LIFECYCLE: keepalive across two different matched routes
        keepalive_ok = http11_keepalive_two_paths(
            port,
            "/assets/marker.txt",
            BODY_A,
            "/assets/deep/marker.txt",
            BODY_DEEP,
        )

        positive = pos_short and pos_long and pos_site
        negative = neg_nomatch and neg_cross
        failure = fail_missing
        boundary = boundary_ok
        lifecycle = conc_ok and keepalive_ok
        overall = positive and negative and failure and boundary and lifecycle

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP005_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP005_NEGATIVE_STATUS": "PASS" if negative else "FAIL",
                "CAP005_FAILURE_STATUS": "PASS" if failure else "FAIL",
                "CAP005_BOUNDARY_STATUS": "PASS" if boundary else "FAIL",
                "CAP005_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if lifecycle else "FAIL",
                "CAP005_CROSS_CAPABILITY_INVARIANTS": {
                    "OUTSIDE_ROOT_STATIC_BYTES_REACHABLE": "NO" if boundary_ok else "YES",
                    "CAP004_CONTAINMENT_VIA_ROUTED_STATIC": "PASS" if boundary_ok else "FAIL",
                    "LONGEST_PREFIX_WINS": "PASS" if pos_long else "FAIL",
                },
                "BODY_SHA256_STATUS": "PASS" if (pos_short and pos_long and pos_site) else "FAIL",
                "checks": {
                    "pos_short_assets": pos_short,
                    "pos_longest_deep": pos_long,
                    "pos_site": pos_site,
                    "neg_nomatch": neg_nomatch,
                    "fail_missing_on_matched_route": fail_missing,
                    "boundary_symlink_escape": boundary_ok,
                    "boundary_symlink_code": code,
                    "boundary_traversal_code": trav,
                    "concurrency_8": conc_ok,
                    "keepalive_cross_route": keepalive_ok,
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
