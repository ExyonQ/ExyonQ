#!/usr/bin/env python3
"""CAPABILITY_023 = compression-brotli — real product E2E.

Proves Content-Encoding: br via shared Accept-Encoding negotiation (Cap022 path).
Independent Brotli oracle (Python brotli / system brotli — not Rust brotli crate).
Cap019 Range + Cap020 304 protectors. ZERO_FAKE.
"""
from __future__ import annotations

import concurrent.futures
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

TEXT = b"cap023-brotli-static-payload-v1\n" * 40
OCTET = bytes([0x41 + (i % 26) for i in range(512)])
JSON_BODY = b'{"cap023":true,"pad":"' + (b"x" * 200) + b'"}'
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


def _libbrotli_decompress(raw: bytes) -> bytes:
    """Google C libbrotlidec via ctypes — independent of Rust `brotli` crate."""
    import ctypes

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
        if rc == 1:  # BROTLI_DECODER_RESULT_SUCCESS
            return bytes(out[: decoded_size.value])
        if rc == 3:  # NEEDS_MORE_OUTPUT
            out_cap *= 2
            continue
        raise OSError(f"BrotliDecoderDecompress failed rc={rc}")
    raise OSError("BrotliDecoderDecompress exhausted output growth")


def select_brotli_oracle() -> tuple[str, bool, callable]:
    """Return (name, independent_from_rust_brotli, decompress_fn)."""
    # Prefer system Google C library (present on Netcup/Oracle qualification hosts).
    try:
        import ctypes

        ctypes.CDLL("libbrotlidec.so.1")
        return ("libbrotlidec.so.1", True, _libbrotli_decompress)
    except OSError:
        pass
    try:
        import brotli as pybrotli  # google brotli Python bindings

        def dec(raw: bytes) -> bytes:
            return pybrotli.decompress(raw)

        return ("python-brotli", True, dec)
    except Exception:
        pass
    cli = shutil.which("brotli")
    if cli:

        def dec(raw: bytes) -> bytes:
            p = subprocess.run(
                [cli, "-d", "-c"],
                input=raw,
                capture_output=True,
                check=True,
            )
            return p.stdout

        return ("system-brotli-cli", True, dec)
    raise RuntimeError(
        "no independent Brotli oracle (libbrotlidec / python brotli / brotli CLI)"
    )


ORACLE_NAME, ORACLE_INDEPENDENT, brotli_decompress = select_brotli_oracle()


def brotli_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return brotli_decompress(raw) == expected
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


class UpstreamHandler(BaseHTTPRequestHandler):
    mode = "plain"  # plain | already_gzip
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
        "FEATURE_ID": "compression-brotli",
        "CAPABILITY": "CAPABILITY_023",
        "CAPABILITY_NAME": "compression-brotli",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Content-Encoding: br via shared AE negotiation",
        "BROTLI_CONTENT_ENCODING": "br",
        "BROTLI_ORACLE_IMPLEMENTATION": ORACLE_NAME,
        "BROTLI_ORACLE_INDEPENDENT_FROM_SERVER_ENCODER": "YES"
        if ORACLE_INDEPENDENT
        else "NO",
        "SERVER_PREFERENCE_ON_EQUAL_Q": "br > gzip > deflate",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "CAPABILITY_024_STARTED": "NO",
        "CAP022_REOPEN_REQUIRED": "NO",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap023-", dir=str(EV)))
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
        log = EV / "exyonq-cap023-static.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen), "static listen"
        base = f"http://127.0.0.1:{listen}/assets"

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: br"])
        decomp = brotli_decompress(raw) if brotli_ok(raw, TEXT) else b""
        cl_hdr = h.get("content-length")
        checks["static_brotli"] = {
            "ok": st == 200
            and h.get("content-encoding") == "br"
            and "accept-encoding" in h.get("vary", "").lower()
            and brotli_ok(raw, TEXT)
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
            f"{base}/text.txt", headers=["Accept-Encoding: gzip, deflate, br"]
        )
        checks["equal_q_tie_br"] = {
            "ok": st == 200 and h.get("content-encoding") == "br" and brotli_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: gzip;q=1, br;q=0.5"],
        )
        checks["q_prefers_gzip"] = {
            "ok": st == 200 and h.get("content-encoding") == "gzip" and gzip_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: deflate;q=1, br;q=0.5"],
        )
        checks["q_prefers_deflate"] = {
            "ok": st == 200
            and h.get("content-encoding") == "deflate"
            and zlib_ok(raw, TEXT),
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: br;q=0"])
        checks["br_q_zero_identity"] = {
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
            headers=["Accept-Encoding: br;q=0, gzip;q=0, deflate;q=0, identity;q=0"],
        )
        checks["all_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }

        # Cap071: wildcard / equal-q server preference is zstd > br > gzip > deflate.
        # Fail-closed: require independent zstd oracle (do not PASS on CE token alone).
        st, h, raw = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: *"])
        wild_ok = False
        wild_detail = None
        if st == 200 and h.get("content-encoding") == "zstd":
            cli = shutil.which("zstd")
            if not cli:
                wild_detail = "ENVIRONMENT_BLOCKER: zstd CLI required for wildcard oracle"
            else:
                try:
                    wild_ok = (
                        subprocess.run(
                            [cli, "-d", "-c"],
                            input=raw,
                            capture_output=True,
                            check=True,
                        ).stdout
                        == TEXT
                    )
                except (subprocess.CalledProcessError, OSError) as exc:
                    wild_detail = repr(exc)
        checks["wildcard_prefers_zstd"] = {
            "ok": wild_ok,
            "ce": h.get("content-encoding"),
            "detail": wild_detail,
        }

        st, h, raw = curl_raw(f"{base}/bin.dat", headers=["Accept-Encoding: br"])
        checks["octet_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == OCTET,
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(f"{base}/tiny.txt", headers=["Accept-Encoding: br"])
        checks["below_min_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == b"tiny",
            "ce": h.get("content-encoding"),
        }

        st, h, raw = curl_raw(
            f"{base}/tiny.txt",
            headers=["Accept-Encoding: br, identity;q=0"],
        )
        checks["below_min_identity_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }

        # Cap019: Range + br → 206 unencoded
        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: br", "Range: bytes=0-9"],
        )
        checks["range_206_no_br"] = {
            "ok": st == 206
            and h.get("content-encoding") is None
            and raw == TEXT[:10]
            and h.get("content-range", "").startswith("bytes 0-9/"),
            "status": st,
            "ce": h.get("content-encoding"),
            "cr": h.get("content-range"),
            "body_len": len(raw),
        }

        # Cap019: unsatisfiable → 416
        st, h, raw = curl_raw(
            f"{base}/text.txt",
            headers=["Accept-Encoding: br", "Range: bytes=999999-9999999"],
        )
        checks["range_416_no_br"] = {
            "ok": st == 416 and h.get("content-encoding") is None,
            "status": st,
            "ce": h.get("content-encoding"),
        }

        st0, h0, _ = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: identity"])
        etag = h0.get("etag")
        if etag:
            st, h, raw = curl_raw(
                f"{base}/text.txt",
                headers=["Accept-Encoding: br", f"If-None-Match: {etag}"],
            )
            checks["conditional_304_no_br"] = {
                "ok": st == 304 and h.get("content-encoding") is None and raw == b"",
                "status": st,
                "ce": h.get("content-encoding"),
                "body_len": len(raw),
            }
        else:
            checks["conditional_304_no_br"] = {
                "ok": False,
                "detail": "no ETag on identity GET",
            }

        st, h, raw = curl_raw(
            f"{base}/text.txt", method="HEAD", headers=["Accept-Encoding: br"]
        )
        checks["head_no_body"] = {
            "ok": st == 200
            and raw == b""
            and h.get("content-encoding") in (None, "br"),
            "status": st,
            "body_len": len(raw),
            "ce": h.get("content-encoding"),
        }

        def one(i: int) -> bool:
            kind = i % 4
            if kind == 0:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: br"])
                return s == 200 and hh.get("content-encoding") == "br" and brotli_ok(b, TEXT)
            if kind == 1:
                s, hh, b = curl_raw(f"{base}/text.txt", headers=["Accept-Encoding: gzip"])
                return s == 200 and hh.get("content-encoding") == "gzip" and gzip_ok(b, TEXT)
            if kind == 2:
                s, hh, b = curl_raw(
                    f"{base}/text.txt", headers=["Accept-Encoding: deflate"]
                )
                return (
                    s == 200
                    and hh.get("content-encoding") == "deflate"
                    and zlib_ok(b, TEXT)
                )
            s, hh, b = curl_raw(
                f"{base}/text.txt", headers=["Accept-Encoding: identity"]
            )
            return s == 200 and hh.get("content-encoding") is None and b == TEXT

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
            conc = list(ex.map(one, range(32)))
        checks["concurrency_mixed"] = {
            "ok": all(conc),
            "pass": sum(conc),
            "n": len(conc),
        }

        keepalive_ok = True
        for ae, pred in [
            ("br", lambda r, hh: hh.get("content-encoding") == "br" and brotli_ok(r, TEXT)),
            ("identity", lambda r, hh: hh.get("content-encoding") is None and r == TEXT),
            ("gzip", lambda r, hh: hh.get("content-encoding") == "gzip" and gzip_ok(r, TEXT)),
            (
                "deflate",
                lambda r, hh: hh.get("content-encoding") == "deflate" and zlib_ok(r, TEXT),
            ),
            ("br", lambda r, hh: hh.get("content-encoding") == "br" and brotli_ok(r, TEXT)),
        ]:
            s, hh, b = curl_raw(f"{base}/text.txt", headers=[f"Accept-Encoding: {ae}"])
            if s != 200 or not pred(b, hh):
                keepalive_ok = False
                break
        checks["sequential_mixed_encodings"] = {"ok": keepalive_ok}

        stop_proc(p)
        procs.pop()

        # Proxy brotli
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
            stdout=(EV / "exyonq-cap023-proxy.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        st, h, raw = curl_raw(
            f"http://127.0.0.1:{listen}/api/j",
            headers=["Accept-Encoding: br"],
        )
        checks["proxy_brotli"] = {
            "ok": st == 200
            and h.get("content-encoding") == "br"
            and brotli_ok(raw, JSON_BODY)
            and UpstreamHandler.hits >= 1,
            "status": st,
            "ce": h.get("content-encoding"),
            "upstream_hits": UpstreamHandler.hits,
        }

        # Already-encoded upstream: must not double-wrap as br
        UpstreamHandler.mode = "already_gzip"
        st, h, raw = curl_raw(
            f"http://127.0.0.1:{listen}/api/pre",
            headers=["Accept-Encoding: br"],
        )
        checks["proxy_already_encoded_no_double"] = {
            "ok": st == 200
            and h.get("content-encoding") == "gzip"
            and gzip_ok(raw, JSON_BODY),
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
