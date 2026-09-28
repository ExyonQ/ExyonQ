#!/usr/bin/env python3
"""CAPABILITY_022 = compression-deflate — real product E2E.

Proves zlib-wrapped Content-Encoding: deflate via shared Accept-Encoding negotiation.
Independent zlib/gzip oracles. Cap019 Range + Cap020 304 protectors.
ZERO_FAKE: real binary, real FS, real HTTP peer, raw wire bytes.
"""
from __future__ import annotations

import concurrent.futures
import gzip
import hashlib
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import zlib
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

TEXT = (
    b"cap022-deflate-static-payload-v1\n" * 40
)  # well above default/min_bytes=32
OCTET = bytes([0x41 + (i % 26) for i in range(512)])
JSON_BODY = b'{"cap022":true,"pad":"' + (b"x" * 200) + b'"}'


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
            time.sleep(0.05)
    return False


def stop_proc(p: subprocess.Popen | None) -> None:
    if p is None or p.poll() is not None:
        return
    p.send_signal(2)
    try:
        p.wait(timeout=8)
    except subprocess.TimeoutExpired:
        p.kill()


def curl_raw(
    url: str,
    *,
    method: str = "GET",
    headers: list[str] | None = None,
    timeout: int = 20,
) -> tuple[int, dict[str, str], bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.hdr"
    # No --compressed: capture raw Content-Encoding body for independent zlib/gzip oracle.
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-X",
        method,
        "-D",
        str(hdr_path),
        "-o",
        str(body_path),
    ]
    if headers:
        for h in headers:
            cmd.extend(["-H", h])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr_text = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    try:
        body_path.unlink(missing_ok=True)
        hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    status = 0
    hmap: dict[str, str] = {}
    for line in hdr_text.splitlines():
        if line.startswith("HTTP/"):
            parts = line.split()
            if len(parts) >= 2 and parts[1].isdigit():
                status = int(parts[1])
        elif ":" in line:
            k, v = line.split(":", 1)
            hmap[k.strip().lower()] = v.strip()
    if proc.returncode != 0 and status == 0:
        status = -1
    return status, hmap, body


def zlib_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return zlib.decompress(raw) == expected
    except zlib.error:
        return False


def gzip_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return gzip.decompress(raw) == expected
    except OSError:
        return False


class UpstreamHandler(BaseHTTPRequestHandler):
    body = JSON_BODY
    hits = 0
    lock = threading.Lock()

    def log_message(self, *_a):
        pass

    def do_GET(self):
        with UpstreamHandler.lock:
            UpstreamHandler.hits += 1
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(self.body)))
        self.end_headers()
        self.wfile.write(self.body)


def write_static_cfg(path: Path, listen: int, root: Path) -> None:
    path.write_text(
        f"""config_version = 1

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["assets"]

[[route]]
name = "assets"
match = {{ path = "/assets" }}
root = "{root}"

[modules.compression]
enabled = true
min_bytes = 32
"""
    )


def write_proxy_cfg(path: Path, listen: int, upstream: int) -> None:
    path.write_text(
        f"""config_version = 1

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api"]

[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:{upstream}"
timeout_ms = 5000

[modules.compression]
enabled = true
min_bytes = 32
"""
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "compression-deflate",
        "CAPABILITY": "CAPABILITY_022",
        "CAPABILITY_NAME": "compression-deflate",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "zlib Content-Encoding: deflate via shared AE negotiation",
        "DEFLATE_WIRE_FORMAT": "zlib-RFC1950",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "CAPABILITY_023_STARTED": "NO",
        "CAP021_REOPEN_REQUIRED": "NO",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap022-", dir=str(EV)))
    root = tmp / "www"
    root.mkdir()
    (root / "text.txt").write_bytes(TEXT)
    (root / "bin.dat").write_bytes(OCTET)
    (root / "tiny.txt").write_bytes(b"tiny")
    checks: dict = {}
    procs: list[subprocess.Popen | None] = []
    httpd = None

    try:
        listen = pick_port()
        cfg = tmp / "static.toml"
        write_static_cfg(cfg, listen, root)
        log = EV / "exyonq-cap022-static.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen), "static listen"
        base = f"http://127.0.0.1:{listen}/assets"

        # Static deflate
        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: deflate"])
        checks["static_deflate"] = {
            "ok": st == 200
            and h.get("content-encoding") == "deflate"
            and "accept-encoding" in h.get("vary", "").lower()
            and zlib_ok(raw, TEXT)
            and sha256_bytes(zlib.decompress(raw)) == sha256_bytes(TEXT)
            and raw[:1] == b"\x78",
            "status": st,
            "ce": h.get("content-encoding"),
            "vary": h.get("vary"),
            "encoded_len": len(raw),
            "orig_sha": sha256_bytes(TEXT),
            "decomp_sha": sha256_bytes(zlib.decompress(raw)) if zlib_ok(raw, TEXT) else None,
        }

        # gzip still works; tie prefers gzip
        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: gzip, deflate"])
        checks["static_gzip_tie"] = {
            "ok": st == 200 and h.get("content-encoding") == "gzip" and gzip_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: gzip;q=0.5, deflate;q=1"],
        )
        checks["q_prefers_deflate"] = {
            "ok": st == 200 and h.get("content-encoding") == "deflate" and zlib_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: deflate;q=0"])
        checks["q_zero_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == TEXT,
            "status": st,
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: identity;q=0"])
        checks["identity_q0_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }

        st, h, raw = curl_raw(
            f"{base}/bin.dat", headers=["Accept-Encoding: deflate"]
        )
        checks["octet_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == OCTET,
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/tiny.txt", headers=["Accept-Encoding: deflate"])
        checks["below_min_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == b"tiny",
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/tiny.txt",
            headers=["Accept-Encoding: deflate, identity;q=0"],
        )
        checks["below_min_identity_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/bin.dat",
            headers=["Accept-Encoding: identity;q=0"],
        )
        checks["octet_identity_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
            "ce": h.get("content-encoding"),
        }

        # Cap019 protector: Range → 206, no Content-Encoding
        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: deflate", "Range: bytes=0-9"],
        )
        checks["range_206_no_deflate"] = {
            "ok": st == 206
            and h.get("content-encoding") is None
            and raw == TEXT[:10]
            and h.get("content-range", "").startswith("bytes 0-9/"),
            "status": st,
            "ce": h.get("content-encoding"),
            "cr": h.get("content-range"),
            "body_len": len(raw),
        }

        # Cap020 protector: 304 has no deflate body
        # Need validators — Cap020 ETag from static. Use If-None-Match with weak etag if present.
        st0, h0, _ = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: identity"])
        etag = h0.get("etag")
        if etag:
            st, h, raw = curl_raw(
                f"{base}/text.txt",
                headers=["Accept-Encoding: deflate", f"If-None-Match: {etag}"],
            )
            checks["conditional_304_no_deflate"] = {
                "ok": st == 304 and h.get("content-encoding") is None and raw == b"",
                "status": st,
                "ce": h.get("content-encoding"),
                "body_len": len(raw),
            }
        else:
            checks["conditional_304_no_deflate"] = {
                "ok": False,
                "detail": "no ETag on identity GET — Cap020 validators missing",
            }

        # HEAD — body must be empty; CE may advertise the GET representation.
        st, h, raw = curl_raw(
            f"{base}/text.txt", method="HEAD", headers=["Accept-Encoding: deflate"]
        )
        checks["head_no_body"] = {
            "ok": st == 200
            and raw == b""
            and len(raw) == 0
            and h.get("content-encoding") in (None, "deflate"),
            "status": st,
            "body_len": len(raw),
            "ce": h.get("content-encoding"),
        }

        # concurrency mixed encodings
        def one(i: int) -> bool:
            if i % 3 == 0:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: deflate"])
                return s == 200 and hh.get("content-encoding") == "deflate" and zlib_ok(b, TEXT)
            if i % 3 == 1:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: gzip"])
                return s == 200 and hh.get("content-encoding") == "gzip" and gzip_ok(b, TEXT)
            s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: identity"])
            return s == 200 and hh.get("content-encoding") is None and b == TEXT

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
            conc = list(ex.map(one, range(24)))
        checks["concurrency_mixed"] = {"ok": all(conc), "pass": sum(conc), "n": len(conc)}

        # keepalive mixed on one curl process via multiple -H rounds is hard; sequential reuse:
        keepalive_ok = True
        for ae, pred in [
            ("deflate", lambda r, hh: hh.get("content-encoding") == "deflate" and zlib_ok(r, TEXT)),
            ("identity", lambda r, hh: hh.get("content-encoding") is None and r == TEXT),
            ("gzip", lambda r, hh: hh.get("content-encoding") == "gzip" and gzip_ok(r, TEXT)),
            ("deflate", lambda r, hh: hh.get("content-encoding") == "deflate" and zlib_ok(r, TEXT)),
        ]:
            s, hh, b = curl_raw(f"{base}/text.txt", headers=[f"Accept-Encoding: {ae}"])
            if s != 200 or not pred(b, hh):
                keepalive_ok = False
                break
        checks["sequential_mixed_encodings"] = {"ok": keepalive_ok}

        stop_proc(p)
        procs.pop()

        # Proxy deflate
        up_port = pick_port()
        httpd = ThreadingHTTPServer(("127.0.0.1", up_port), UpstreamHandler)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        assert wait_listen(up_port, 5)
        listen = pick_port()
        cfg = tmp / "proxy.toml"
        write_proxy_cfg(cfg, listen, up_port)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap022-proxy.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        st, h, raw = curl_raw(
            f"http://127.0.0.1:{listen}/api/j",
            headers=["Accept-Encoding: deflate"],
        )
        checks["proxy_deflate"] = {
            "ok": st == 200
            and h.get("content-encoding") == "deflate"
            and zlib_ok(raw, JSON_BODY)
            and UpstreamHandler.hits >= 1,
            "status": st,
            "ce": h.get("content-encoding"),
            "upstream_hits": UpstreamHandler.hits,
        }
        stop_proc(p)
        procs.pop()

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["DETAIL"] = repr(exc)
        result["CHECKS"] = checks
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 1
    finally:
        for proc in procs:
            stop_proc(proc)
        if httpd is not None:
            httpd.shutdown()

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
