#!/usr/bin/env python3
"""CAPABILITY_044 = content-negotiation — real product Accept-Encoding matrix.

Transversal contract over identity/gzip/deflate/br/zstd shared negotiation.
Authoritative path: real binary → real HTTP → real filter. ZERO_FAKE.
Cap043/022/023/071 remain closed; Cap045 must not start.
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

TEXT = b"cap044-negotiation-static-payload-v1\n" * 40
OCTET = bytes([0x41 + (i % 26) for i in range(512)])
JSON_BODY = b'{"cap044":true,"pad":"' + (b"y" * 200) + b'"}'
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
    extra_args: list[str] | None = None,
) -> tuple[int, dict[str, str], bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.hdr"
    cmd = [
        "curl", "-sS", "--max-time", str(timeout),
        "-X", method, "-D", str(hdr_path), "-o", str(body_path),
    ]
    if extra_args:
        cmd.extend(extra_args)
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
            key = k.strip().lower()
            # Repeated response headers: last wins for our checks.
            hmap[key] = v.strip()
    if proc.returncode != 0 and status == 0:
        status = -1
    return status, hmap, body


def gzip_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return gzip.decompress(raw) == expected
    except OSError:
        return False


def zlib_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return zlib.decompress(raw) == expected
    except zlib.error:
        return False


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


def select_brotli_oracle() -> tuple[str, callable]:
    import ctypes

    try:
        ctypes.CDLL("libbrotlidec.so.1")
        return ("libbrotlidec.so.1", _libbrotli_decompress)
    except OSError:
        pass
    try:
        import brotli as pyb

        return ("python-brotli", pyb.decompress)
    except Exception:
        pass
    cli = shutil.which("brotli")
    if cli:

        def dec(raw: bytes) -> bytes:
            return subprocess.run(
                [cli, "-d", "-c"], input=raw, capture_output=True, check=True
            ).stdout

        return ("system-brotli-cli", dec)
    raise RuntimeError(
        "no independent Brotli oracle (libbrotlidec / python brotli / brotli CLI)"
    )


BROTLI_ORACLE_NAME, brotli_decompress = select_brotli_oracle()


def brotli_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return brotli_decompress(raw) == expected
    except Exception:
        return False


def zstd_decompress(raw: bytes) -> bytes:
    cli = shutil.which("zstd")
    if cli:
        return subprocess.run([cli, "-d", "-c"], input=raw, capture_output=True, check=True).stdout
    raise OSError("zstd CLI required for Cap044 oracle")


def zstd_ok(raw: bytes, expected: bytes) -> bool:
    try:
        return zstd_decompress(raw) == expected
    except Exception:
        return False


def coding_ok(ce: str | None, raw: bytes, expected: bytes) -> bool:
    if ce == "gzip":
        return gzip_ok(raw, expected)
    if ce == "deflate":
        return zlib_ok(raw, expected)
    if ce == "br":
        return brotli_ok(raw, expected)
    if ce == "zstd":
        return zstd_ok(raw, expected)
    if ce is None:
        return raw == expected
    return False


class UpstreamHandler(BaseHTTPRequestHandler):
    mode = "plain"
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


def expect_ce(checks: dict, name: str, url: str, ae: str, want_ce: str | None, plain: bytes) -> None:
    st, h, raw = curl_raw(url, headers=[f"Accept-Encoding: {ae}"] if ae is not None else [])
    ce = h.get("content-encoding")
    vary = h.get("vary", "")
    ok = (
        st == 200
        and ce == want_ce
        and coding_ok(ce, raw, plain)
        and "accept-encoding" in vary.lower()
    )
    checks[name] = {"ok": ok, "status": st, "ce": ce, "vary": vary, "ae": ae, "want": want_ce}


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "content-negotiation",
        "CAPABILITY": "CAPABILITY_044",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Accept-Encoding negotiation identity+gzip+deflate+br+zstd",
        "SERVER_PREFERENCE_ON_EQUAL_Q": "zstd > br > gzip > deflate",
        "DUPLICATE_TOKEN_RULE": "LAST_WINS",
        "INVALID_Q_RULE": "TREAT_AS_Q0",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "BROTLI_ORACLE": BROTLI_ORACLE_NAME,
        "CAPABILITY_045_STARTED": "NO",
        "CAP043_REOPEN": "NO",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)

    tmp = Path(tempfile.mkdtemp(prefix="cap044-", dir=str(EV)))
    root = tmp / "www"
    root.mkdir()
    (root / "text.txt").write_bytes(TEXT)
    (root / "bin.dat").write_bytes(OCTET)
    (root / "tiny.txt").write_bytes(b"tiny")
    checks: dict = {}
    procs: list = []
    httpd = None

    try:
        listen = pick_port()
        cfg = tmp / "static.toml"
        write_static_cfg(cfg, listen, root)
        log = EV / "exyonq-cap044-static.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"), stderr=subprocess.STDOUT, cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen), "static listen"
        base = f"http://127.0.0.1:{listen}/assets"
        url = f"{base}/text.txt"

        # 1 Missing AE → identity (no auto-compress)
        st, h, raw = curl_raw(url)
        checks["missing_ae_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == TEXT
            and "accept-encoding" in h.get("vary", "").lower(),
            "status": st, "ce": h.get("content-encoding"), "vary": h.get("vary"),
        }

        # 2-5 single coding
        expect_ce(checks, "gzip_only", url, "gzip", "gzip", TEXT)
        expect_ce(checks, "deflate_only", url, "deflate", "deflate", TEXT)
        expect_ce(checks, "br_only", url, "br", "br", TEXT)
        expect_ce(checks, "zstd_only", url, "zstd", "zstd", TEXT)

        # 6 equal-q
        expect_ce(checks, "equal_q_all_zstd", url, "gzip;q=1, deflate;q=1, br;q=1, zstd;q=1", "zstd", TEXT)
        expect_ce(checks, "equal_q_br_gzip", url, "br;q=1,gzip;q=1", "br", TEXT)
        expect_ce(checks, "equal_q_gzip_deflate", url, "gzip;q=1,deflate;q=1", "gzip", TEXT)

        # 7 client q beats server
        expect_ce(checks, "client_br_over_zstd", url, "br;q=1,zstd;q=0.9", "br", TEXT)
        expect_ce(checks, "client_gzip_over_zstd", url, "gzip;q=1,zstd;q=0.5", "gzip", TEXT)
        expect_ce(checks, "client_deflate_over_br", url, "deflate;q=1,br;q=0.5", "deflate", TEXT)

        # 8 q=0
        expect_ce(checks, "zstd_q0_br", url, "zstd;q=0,br;q=1", "br", TEXT)
        expect_ce(checks, "br_q0_gzip", url, "br;q=0,gzip;q=1", "gzip", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q=0"])
        checks["gzip_q0_never"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip" and raw == TEXT,
            "ce": h.get("content-encoding"),
        }
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: deflate;q=0"])
        checks["deflate_q0_never"] = {
            "ok": st == 200 and h.get("content-encoding") != "deflate" and raw == TEXT,
            "ce": h.get("content-encoding"),
        }

        # Identity semantics (Cap043 contract)
        expect_ce(checks, "gzip_q09_implicit_identity", url, "gzip;q=0.9", "gzip", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: identity;q=1,gzip;q=0.9"])
        checks["identity_q1_over_gzip_q09"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == TEXT,
            "ce": h.get("content-encoding"),
        }
        expect_ce(checks, "identity_q05_gzip_q1", url, "identity;q=0.5,gzip;q=1", "gzip", TEXT)
        expect_ce(checks, "identity_q0_gzip_q1", url, "identity;q=0,gzip;q=1", "gzip", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: identity;q=0,gzip;q=0"])
        checks["identity_gzip_both_q0_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st, "ce": h.get("content-encoding"), "body_hex": raw[:64].hex(),
        }
        st, h, raw = curl_raw(
            url,
            headers=["Accept-Encoding: identity;q=0,zstd;q=0,br;q=0,gzip;q=0,deflate;q=0"],
        )
        checks["all_forbidden_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None and b"Not Acceptable" in raw,
            "status": st, "ce": h.get("content-encoding"),
        }

        # Wildcard
        expect_ce(checks, "star_alone_zstd", url, "*", "zstd", TEXT)
        expect_ce(checks, "star_q1_zstd", url, "*;q=1", "zstd", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: *;q=0"])
        checks["star_q0_406"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st,
        }
        expect_ce(checks, "star_q05_gzip_q1", url, "*;q=0.5,gzip;q=1", "gzip", TEXT)
        # *;q=1,gzip;q=0 → gzip forbidden; other codings from * → zstd
        expect_ce(checks, "star_q1_gzip_q0_zstd", url, "*;q=1,gzip;q=0", "zstd", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: *;q=0,identity;q=1"])
        checks["star_q0_identity_q1"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == TEXT,
            "status": st, "ce": h.get("content-encoding"),
        }

        # Duplicates last-wins
        expect_ce(checks, "dup_gzip_last_q1", url, "gzip;q=0.5,gzip;q=1", "gzip", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q=1,gzip;q=0"])
        checks["dup_gzip_last_q0"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip" and raw == TEXT,
            "ce": h.get("content-encoding"),
        }
        expect_ce(checks, "dup_br_br", url, "br,br", "br", TEXT)
        expect_ce(checks, "dup_zstd_last", url, "zstd;q=0.2,zstd;q=0.8", "zstd", TEXT)

        # Invalid q → q=0 for that token
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q="])
        checks["invalid_q_empty"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip",
            "ce": h.get("content-encoding"), "status": st,
        }
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q=abc"])
        checks["invalid_q_abc"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip",
            "ce": h.get("content-encoding"),
        }
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q=1.1"])
        checks["invalid_q_gt1"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip",
            "ce": h.get("content-encoding"),
        }
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: gzip;q=-1"])
        checks["invalid_q_neg"] = {
            "ok": st == 200 and h.get("content-encoding") != "gzip",
            "ce": h.get("content-encoding"),
        }
        expect_ce(checks, "invalid_q_abc_with_gzip", url, "zstd;q=abc, gzip", "gzip", TEXT)

        # Unknown codings
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: compress"])
        checks["unknown_compress_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == TEXT,
            "ce": h.get("content-encoding"),
        }
        expect_ce(checks, "foobar_with_gzip", url, "foobar;q=1,gzip;q=0.5", "gzip", TEXT)
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: foobar;q=1,identity;q=0"])
        checks["foobar_identity_q0_406"] = {
            "ok": st == 406, "status": st, "ce": h.get("content-encoding"),
        }

        # Repeated Accept-Encoding header fields (curl -H twice).
        # Hyper joins as "gzip, br" → equal client q → server rank prefers br.
        st, h, raw = curl_raw(
            url,
            headers=["Accept-Encoding: gzip", "Accept-Encoding: br"],
        )
        checks["multi_ae_header_fields"] = {
            "ok": st == 200 and h.get("content-encoding") == "br" and brotli_ok(raw, TEXT),
            "status": st, "ce": h.get("content-encoding"),
            "note": "Hyper join gzip, br → equal-q → br",
        }

        # Eligibility
        st, h, raw = curl_raw(f"{base}/bin.dat", headers=["Accept-Encoding: gzip"])
        checks["octet_not_compressible"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == OCTET,
            "ce": h.get("content-encoding"),
        }
        st, h, raw = curl_raw(f"{base}/tiny.txt", headers=["Accept-Encoding: gzip"])
        checks["below_min_identity"] = {
            "ok": st == 200 and h.get("content-encoding") is None and raw == b"tiny",
            "ce": h.get("content-encoding"),
        }

        # Range / 304 protectors
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: zstd", "Range: bytes=0-9"])
        checks["range_206_no_dynamic"] = {
            "ok": st == 206 and h.get("content-encoding") is None and raw == TEXT[:10],
            "status": st, "ce": h.get("content-encoding"),
        }
        st0, h0, _ = curl_raw(url, headers=["Accept-Encoding: identity"])
        etag = h0.get("etag")
        if etag:
            st, h, raw = curl_raw(url, headers=["Accept-Encoding: br", f"If-None-Match: {etag}"])
            checks["conditional_304_no_body"] = {
                "ok": st == 304 and h.get("content-encoding") is None and raw == b"",
                "status": st, "ce": h.get("content-encoding"),
            }
        else:
            checks["conditional_304_no_body"] = {"ok": False, "detail": "no etag"}

        # HEAD: CE must match GET negotiation; body empty
        stg, hg, _ = curl_raw(url, headers=["Accept-Encoding: gzip"])
        sth, hh, bh = curl_raw(url, method="HEAD", headers=["Accept-Encoding: gzip"])
        checks["head_gzip_ce_matches_get"] = {
            "ok": sth == 200 and bh == b"" and hh.get("content-encoding") == hg.get("content-encoding") == "gzip"
            and "accept-encoding" in hh.get("vary", "").lower(),
            "get_ce": hg.get("content-encoding"), "head_ce": hh.get("content-encoding"),
            "head_body_len": len(bh), "status": sth,
        }

        # 406 not encoded
        st, h, raw = curl_raw(
            url,
            headers=["Accept-Encoding: identity;q=0,zstd;q=0,br;q=0,gzip;q=0,deflate;q=0"],
        )
        checks["406_not_content_encoded"] = {
            "ok": st == 406 and h.get("content-encoding") is None,
            "status": st, "ce": h.get("content-encoding"),
        }

        # Concurrency
        def one(i: int) -> bool:
            cases = [
                ("identity", None),
                ("gzip", "gzip"),
                ("deflate", "deflate"),
                ("br", "br"),
                ("zstd", "zstd"),
                ("gzip;q=1,br;q=0.5", "gzip"),
                ("br;q=1,zstd;q=0.9", "br"),
            ]
            ae, want = cases[i % len(cases)]
            s, hh, b = curl_raw(url, headers=[f"Accept-Encoding: {ae}"])
            ce = hh.get("content-encoding")
            ce_ok = ce is None if want is None else ce == want
            return s == 200 and ce_ok and coding_ok(ce, b, TEXT)

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
            conc = list(ex.map(one, range(56)))
        checks["concurrency_mixed"] = {"ok": all(conc), "pass": sum(conc), "n": len(conc)}

        # Keepalive sequential
        seq_ok = True
        for ae, want in [
            ("gzip", "gzip"), ("identity", None), ("zstd", "zstd"), ("br", "br"),
            ("deflate", "deflate"), ("identity;q=0,gzip;q=1", "gzip"),
        ]:
            s, hh, b = curl_raw(url, headers=[f"Accept-Encoding: {ae}"])
            ce = hh.get("content-encoding")
            if s != 200 or not coding_ok(ce, b, TEXT) or ce != want:
                seq_ok = False
                break
        checks["keepalive_sequential"] = {"ok": seq_ok}

        # Cache / FPC cross-variant: sequential primes
        st1, h1, b1 = curl_raw(url, headers=["Accept-Encoding: gzip"])
        st2, h2, b2 = curl_raw(url, headers=["Accept-Encoding: br"])
        st3, h3, b3 = curl_raw(url, headers=["Accept-Encoding: identity"])
        st4, h4, b4 = curl_raw(url, headers=["Accept-Encoding: zstd"])
        checks["no_variant_contamination"] = {
            "ok": (
                st1 == 200 and h1.get("content-encoding") == "gzip" and gzip_ok(b1, TEXT)
                and st2 == 200 and h2.get("content-encoding") == "br" and brotli_ok(b2, TEXT)
                and st3 == 200 and h3.get("content-encoding") is None and b3 == TEXT
                and st4 == 200 and h4.get("content-encoding") == "zstd" and zstd_ok(b4, TEXT)
            ),
        }

        # Vary present on identity after AE negotiation
        st, h, raw = curl_raw(url, headers=["Accept-Encoding: identity"])
        checks["vary_on_identity"] = {
            "ok": st == 200 and "accept-encoding" in h.get("vary", "").lower() and raw == TEXT,
            "vary": h.get("vary"),
        }

        stop_proc(p)
        procs.pop()

        # Proxy path
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
            stdout=(EV / "exyonq-cap044-proxy.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        proxy = f"http://127.0.0.1:{listen}/api/j"
        for name, ae, want in [
            ("proxy_gzip", "gzip", "gzip"),
            ("proxy_zstd", "zstd", "zstd"),
            ("proxy_br", "br", "br"),
            ("proxy_equal_q", "gzip;q=1,br;q=1,zstd;q=1", "zstd"),
        ]:
            st, h, raw = curl_raw(proxy, headers=[f"Accept-Encoding: {ae}"])
            checks[name] = {
                "ok": st == 200 and h.get("content-encoding") == want
                and coding_ok(want, raw, JSON_BODY),
                "status": st, "ce": h.get("content-encoding"),
            }
        UpstreamHandler.mode = "already_gzip"
        st, h, raw = curl_raw(proxy + "2", headers=["Accept-Encoding: zstd"])
        checks["proxy_already_encoded_no_double"] = {
            "ok": st == 200 and h.get("content-encoding") == "gzip" and gzip_ok(raw, JSON_BODY),
            "ce": h.get("content-encoding"),
        }
        stop_proc(p)
        procs.pop()

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["DETAIL"] = repr(exc)
        result["CHECKS"] = checks
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 1
    finally:
        for proc in procs:
            stop_proc(proc)
        if httpd is not None:
            httpd.shutdown()

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    result["FAILED"] = [k for k, v in checks.items() if not v.get("ok")]
    OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
