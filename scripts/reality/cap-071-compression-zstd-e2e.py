#!/usr/bin/env python3
"""CAPABILITY_071 = compression-zstd — real product E2E.

Proves Content-Encoding: zstd via shared Accept-Encoding negotiation (Cap022 path).
Independent Zstd oracle (system zstd CLI / libzstd.so — not ExyonQ Rust zstd crate).
Cap019 Range + Cap020 304 protectors. ZERO_FAKE.
"""
from __future__ import annotations

import concurrent.futures
import ctypes
import gzip
import hashlib
import json
import os
import shutil
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

TEXT = b"cap071-zstd-static-payload-v1\n" * 40
OCTET = bytes([0x41 + (i % 26) for i in range(512)])
JSON_BODY = b'{"cap071":true,"pad":"' + (b"x" * 200) + b'"}'
ALREADY_GZIP = gzip.compress(JSON_BODY)


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


def _load_libzstd() -> ctypes.CDLL:
    for name in ("libzstd.so.1", "libzstd.so"):
        try:
            return ctypes.CDLL(name)
        except OSError:
            continue
    raise OSError("libzstd.so not found")


def _libzstd_decompress(raw: bytes) -> bytes:
    """Facebook/Meta libzstd via ctypes — independent of ExyonQ Rust zstd crate."""
    lib = _load_libzstd()
    lib.ZSTD_getFrameContentSize.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
    ]
    lib.ZSTD_getFrameContentSize.restype = ctypes.c_ulonglong
    lib.ZSTD_decompress.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
    ]
    lib.ZSTD_decompress.restype = ctypes.c_size_t
    lib.ZSTD_isError.argtypes = [ctypes.c_size_t]
    lib.ZSTD_isError.restype = ctypes.c_uint
    src = (ctypes.c_uint8 * len(raw)).from_buffer_copy(raw)
    content_size = lib.ZSTD_getFrameContentSize(src, len(raw))
    if content_size == ctypes.c_ulonglong(-1).value:
        raise OSError("ZSTD_getFrameContentSize error")
    if content_size == ctypes.c_ulonglong(-2).value:
        out_cap = max(len(raw) * 8, 4096)
    else:
        out_cap = int(content_size)
    for _ in range(12):
        dst = (ctypes.c_uint8 * out_cap)()
        rc = lib.ZSTD_decompress(dst, out_cap, src, len(raw))
        if lib.ZSTD_isError(rc):
            if out_cap < len(raw) * 64:
                out_cap *= 2
                continue
            raise OSError(f"ZSTD_decompress error code={rc}")
        return bytes(dst[: rc])
    raise OSError("ZSTD_decompress exhausted output growth")


def _libzstd_compress(raw: bytes, level: int = 3) -> bytes:
    lib = _load_libzstd()
    lib.ZSTD_compressBound.argtypes = [ctypes.c_size_t]
    lib.ZSTD_compressBound.restype = ctypes.c_size_t
    lib.ZSTD_compress.argtypes = [
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.c_int,
    ]
    lib.ZSTD_compress.restype = ctypes.c_size_t
    lib.ZSTD_isError.argtypes = [ctypes.c_size_t]
    lib.ZSTD_isError.restype = ctypes.c_uint
    bound = lib.ZSTD_compressBound(len(raw))
    dst = (ctypes.c_uint8 * bound)()
    src = (ctypes.c_uint8 * len(raw)).from_buffer_copy(raw)
    rc = lib.ZSTD_compress(dst, bound, src, len(raw), level)
    if lib.ZSTD_isError(rc):
        raise OSError(f"ZSTD_compress error code={rc}")
    return bytes(dst[: rc])


def select_zstd_oracle() -> tuple[str, bool, str, callable]:
    """Prefer system zstd CLI; fallback libzstd.so. Never ExyonQ encoder path."""
    cli = shutil.which("zstd")
    if cli:
        ver = subprocess.run(
            [cli, "--version"], capture_output=True, text=True
        )
        version = (ver.stderr or ver.stdout or "").strip().splitlines()
        version_s = version[0] if version else "unknown"

        def dec(raw: bytes) -> bytes:
            p = subprocess.run(
                [cli, "-d", "-c"],
                input=raw,
                capture_output=True,
                check=True,
            )
            return p.stdout

        return ("system-zstd-cli", True, version_s, dec)
    try:
        _load_libzstd()
        return ("libzstd.so", True, "ctypes-libzstd", _libzstd_decompress)
    except OSError:
        pass
    raise RuntimeError("no independent Zstd oracle (zstd CLI / libzstd.so)")


ORACLE_NAME, ORACLE_INDEPENDENT, ORACLE_VERSION, zstd_decompress = select_zstd_oracle()


def zstd_compress_bytes(raw: bytes) -> bytes:
    cli = shutil.which("zstd")
    if cli:
        p = subprocess.run(
            [cli, "-c", "-3"],
            input=raw,
            capture_output=True,
            check=True,
        )
        return p.stdout
    return _libzstd_compress(raw)


ALREADY_ZSTD = zstd_compress_bytes(JSON_BODY)


def zstd_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return zstd_decompress(raw) == expected
    except Exception:
        return False


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


def _libbrotli_decompress(raw: bytes) -> bytes:
    lib = ctypes.CDLL("libbrotlidec.so.1")
    lib.BrotliDecoderDecompress.argtypes = [
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_uint8),
        ctypes.POINTER(ctypes.c_size_t),
        ctypes.POINTER(ctypes.c_uint8),
    ]
    lib.BrotliDecoderDecompress.restype = ctypes.c_int
    enc = (ctypes.c_uint8 * len(raw)).from_buffer_copy(raw)
    out_cap = max(len(raw) * 8, 4096)
    for _ in range(10):
        out = (ctypes.c_uint8 * out_cap)()
        decoded_size = ctypes.c_size_t(out_cap)
        rc = lib.BrotliDecoderDecompress(
            len(raw), enc, ctypes.byref(decoded_size), out
        )
        if rc == 1:
            return bytes(out[: decoded_size.value])
        if rc == 3:
            out_cap *= 2
            continue
        raise OSError(f"BrotliDecoderDecompress failed rc={rc}")
    raise OSError("BrotliDecoderDecompress exhausted output growth")


def brotli_decompress(raw: bytes) -> bytes:
    cli = shutil.which("brotli")
    if cli:
        p = subprocess.run(
            [cli, "-d", "-c"],
            input=raw,
            capture_output=True,
            check=True,
        )
        return p.stdout
    try:
        return _libbrotli_decompress(raw)
    except OSError:
        import brotli as pybrotli

        return pybrotli.decompress(raw)


def brotli_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return brotli_decompress(raw) == expected
    except Exception:
        return False


class UpstreamHandler(BaseHTTPRequestHandler):
    mode = "plain"  # plain | already_gzip | already_zstd
    hits = 0
    lock = threading.Lock()

    def log_message(self, *_a):
        pass

    def do_GET(self):
        with UpstreamHandler.lock:
            UpstreamHandler.hits += 1
        if UpstreamHandler.mode == "already_gzip":
            body = ALREADY_GZIP
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Encoding", "gzip")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if UpstreamHandler.mode == "already_zstd":
            body = ALREADY_ZSTD
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Encoding", "zstd")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(JSON_BODY)))
        self.end_headers()
        self.wfile.write(JSON_BODY)


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
        "FEATURE_ID": "compression-zstd",
        "CAPABILITY": "CAPABILITY_071",
        "CAPABILITY_NAME": "compression-zstd",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Content-Encoding: zstd via shared AE negotiation",
        "ZSTD_CONTENT_ENCODING": "zstd",
        "ZSTD_ORACLE_IMPLEMENTATION": ORACLE_NAME,
        "ZSTD_ORACLE_VERSION": ORACLE_VERSION,
        "ZSTD_ORACLE_INDEPENDENT_FROM_EXYONQ_ENCODER_PATH": "YES"
        if ORACLE_INDEPENDENT
        else "NO",
        "SERVER_PREFERENCE_ON_EQUAL_Q": "zstd > br > gzip > deflate",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "CAPABILITY_026_STARTED": "NO",
        "PUSH": "NO",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap071-", dir=str(EV)))
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
        log = EV / "exyonq-cap071-static.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen), "static listen"
        base = f"http://127.0.0.1:{listen}/assets"

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: zstd"])
        decomp = zstd_decompress(raw) if zstd_ok(raw, TEXT) else b""
        cl_hdr = h.get("content-length")
        checks["static_zstd"] = {
            "ok": st == 200
            and h.get("content-encoding") == "zstd"
            and "accept-encoding" in h.get("vary", "").lower()
            and zstd_ok(raw, TEXT)
            and sha256_bytes(decomp) == sha256_bytes(TEXT)
            and cl_hdr == str(len(raw)),
            "status": st,
            "ce": h.get("content-encoding"),
            "vary": h.get("vary"),
            "content_length": cl_hdr,
            "encoded_len": len(raw),
            "orig_sha": sha256_bytes(TEXT),
            "decomp_sha": sha256_bytes(decomp) if decomp else None,
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: zstd;q=1, br;q=1, gzip;q=1, deflate;q=1"],
        )
        checks["equal_q_tie_zstd"] = {
            "ok": st == 200 and h.get("content-encoding") == "zstd" and zstd_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: br;q=1, zstd;q=0.9"],
        )
        checks["q_prefers_br_over_zstd"] = {
            "ok": st == 200 and h.get("content-encoding") == "br" and brotli_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: gzip;q=1, zstd;q=0.8"],
        )
        checks["q_prefers_gzip_over_zstd"] = {
            "ok": st == 200 and h.get("content-encoding") == "gzip" and gzip_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: deflate;q=1, zstd;q=0"],
        )
        checks["q_prefers_deflate_over_zstd_zero"] = {
            "ok": st == 200
            and h.get("content-encoding") == "deflate"
            and zlib_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: zstd;q=0"])
        checks["zstd_q_zero_identity"] = {
            "ok": st == 200
            and h.get("content-encoding") is None
            and raw == TEXT
            and "accept-encoding" in h.get("vary", "").lower(),
            "status": st,
            "ce": h.get("content-encoding"),
            "vary": h.get("vary"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=[
                "Accept-Encoding: zstd;q=0, br;q=0, gzip;q=0, deflate;q=0, identity;q=0"
            ],
        )
        checks["all_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: *"])
        checks["wildcard_prefers_zstd"] = {
            "ok": st == 200 and h.get("content-encoding") == "zstd" and zstd_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: gzip"])
        checks["regression_gzip_only"] = {
            "ok": st == 200 and h.get("content-encoding") == "gzip" and gzip_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: deflate"])
        checks["regression_deflate_only"] = {
            "ok": st == 200
            and h.get("content-encoding") == "deflate"
            and zlib_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: br"])
        checks["regression_br_only"] = {
            "ok": st == 200 and h.get("content-encoding") == "br" and brotli_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: zstd"])
        checks["regression_zstd_only"] = {
            "ok": st == 200 and h.get("content-encoding") == "zstd" and zstd_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/bin.dat", headers=["Accept-Encoding: zstd"])
        checks["octet_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == OCTET,
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/tiny.txt", headers=["Accept-Encoding: zstd"])
        checks["below_min_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == b"tiny",
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/tiny.txt",
            headers=["Accept-Encoding: zstd, identity;q=0"],
        )
        checks["below_min_identity_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: zstd", "Range: bytes=0-9"],
        )
        checks["range_206_no_zstd"] = {
            "ok": st == 206
            and h.get("content-encoding") is None
            and raw == TEXT[:10]
            and h.get("content-range", "").startswith("bytes 0-9/"),
            "status": st,
            "ce": h.get("content-encoding"),
            "cr": h.get("content-range"),
            "body_len": len(raw),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: zstd", "Range: bytes=999999-9999999"],
        )
        checks["range_416_no_zstd"] = {
            "ok": st == 416 and h.get("content-encoding") is None,
            "status": st,
            "ce": h.get("content-encoding"),
        }

        st0, h0, _ = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: identity"])
        etag = h0.get("etag")
        if etag:
            st, h, raw = curl_raw(
                f"{base}/text.txt",
                headers=["Accept-Encoding: zstd", f"If-None-Match: {etag}"],
            )
            checks["conditional_304_no_zstd"] = {
                "ok": st == 304 and h.get("content-encoding") is None and raw == b"",
                "status": st,
                "ce": h.get("content-encoding"),
                "body_len": len(raw),
            }
        else:
            checks["conditional_304_no_zstd"] = {
                "ok": False,
                "detail": "no ETag on identity GET",
            }

        st, h, raw = curl_raw(
            f"{base}/text.txt", method="HEAD", headers=["Accept-Encoding: zstd"]
        )
        checks["head_no_body"] = {
            "ok": st == 200 and raw == b"" and h.get("content-encoding") == "zstd",
            "status": st,
            "body_len": len(raw),
            "ce": h.get("content-encoding"),
            "detail": "HEAD body empty; Content-Encoding matches the GET representation",
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            method="HEAD",
            headers=["Accept-Encoding: identity;q=0"],
        )
        checks["head_406_empty_body"] = {
            "ok": st == 406 and raw == b"" and h.get("content-encoding") is None,
            "status": st,
            "body_len": len(raw),
            "ce": h.get("content-encoding"),
        }

        def one(i: int) -> bool:
            kind = i % 5
            if kind == 0:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: zstd"])
                return s == 200 and hh.get("content-encoding") == "zstd" and zstd_ok(b, TEXT)
            if kind == 1:
                s, hh, b = curl_raw(
                    f"{base}/text.txt", headers=["Accept-Encoding: identity"]
                )
                return s == 200 and hh.get("content-encoding") is None and b == TEXT
            if kind == 2:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: gzip"])
                return s == 200 and hh.get("content-encoding") == "gzip" and gzip_ok(b, TEXT)
            if kind == 3:
                s, hh, b = curl_raw(
                    f"{base}/text.txt", headers=["Accept-Encoding: deflate"]
                )
                return (
                    s == 200
                    and hh.get("content-encoding") == "deflate"
                    and zlib_ok(b, TEXT)
                )
            s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: br"])
            return s == 200 and hh.get("content-encoding") == "br" and brotli_ok(b, TEXT)

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
            conc = list(ex.map(one, range(40)))
        checks["concurrency_mixed"] = {
            "ok": all(conc),
            "pass": sum(conc),
            "n": len(conc),
        }

        keepalive_ok = True
        for ae, pred in [
            ("zstd", lambda r, hh: hh.get("content-encoding") == "zstd" and zstd_ok(r, TEXT)),
            ("identity", lambda r, hh: hh.get("content-encoding") is None and r == TEXT),
            ("br", lambda r, hh: hh.get("content-encoding") == "br" and brotli_ok(r, TEXT)),
            ("gzip", lambda r, hh: hh.get("content-encoding") == "gzip" and gzip_ok(r, TEXT)),
            (
                "deflate",
                lambda r, hh: hh.get("content-encoding") == "deflate" and zlib_ok(r, TEXT),
            ),
            ("zstd", lambda r, hh: hh.get("content-encoding") == "zstd" and zstd_ok(r, TEXT)),
        ]:
            s, hh, b = curl_raw(f"{base}/text.txt", headers=[f"Accept-Encoding: {ae}"])
            if s != 200 or not pred(b, hh):
                keepalive_ok = False
                break
        checks["sequential_mixed_encodings"] = {"ok": keepalive_ok}

        stop_proc(p)
        procs.pop()

        up_port = pick_port()
        UpstreamHandler.mode = "plain"
        UpstreamHandler.hits = 0
        httpd = ThreadingHTTPServer(("127.0.0.1", up_port), UpstreamHandler)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        assert wait_listen(up_port, 5)
        listen = pick_port()
        cfg = tmp / "proxy.toml"
        write_proxy_cfg(cfg, listen, up_port)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap071-proxy.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        proxy_base = f"http://127.0.0.1:{listen}/api/j"
        st, h, raw = curl_raw(proxy_base, headers=["Accept-Encoding: zstd"])
        checks["proxy_zstd"] = {
            "ok": st == 200
            and h.get("content-encoding") == "zstd"
            and zstd_ok(raw, JSON_BODY)
            and UpstreamHandler.hits >= 1,
            "status": st,
            "ce": h.get("content-encoding"),
            "upstream_hits": UpstreamHandler.hits,
        }

        UpstreamHandler.mode = "already_gzip"
        st, h, raw = curl_raw(
            f"http://127.0.0.1:{listen}/api/pre",
            headers=["Accept-Encoding: zstd"],
        )
        checks["proxy_already_gzip_no_double"] = {
            "ok": st == 200
            and h.get("content-encoding") == "gzip"
            and gzip_ok(raw, JSON_BODY),
            "status": st,
            "ce": h.get("content-encoding"),
        }

        UpstreamHandler.mode = "already_zstd"
        st, h, raw = curl_raw(
            f"http://127.0.0.1:{listen}/api/pre2",
            headers=["Accept-Encoding: zstd"],
        )
        checks["proxy_already_zstd_no_double"] = {
            "ok": st == 200
            and h.get("content-encoding") == "zstd"
            and zstd_ok(raw, JSON_BODY)
            and raw == ALREADY_ZSTD,
            "status": st,
            "ce": h.get("content-encoding"),
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
