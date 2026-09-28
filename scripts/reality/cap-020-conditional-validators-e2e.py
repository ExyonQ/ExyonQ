#!/usr/bin/env python3
"""CAPABILITY_020 = conditional-validators — real product E2E.

USER_VISIBLE_CONTRACT = If-None-Match / If-Modified-Since → 304

Authoritative: release binary → real config → real FS file → real TCP →
independent verification of 200/304 + Cap019 Range protectors.

EXPLICIT_NON_SCOPE_V1: If-Match, If-Unmodified-Since, If-Range, 412, proxy conditionals.
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
from email.utils import format_datetime, parsedate_to_datetime
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

TEXT = b"abcdefghijklmnopqrstuvwxyz0123456789" * 8  # 288
BINARY_1K = bytes((i % 256) for i in range(1024))
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


def curl_req(
    url: str,
    *,
    method: str = "GET",
    extra_headers: list[str] | None = None,
) -> tuple[str, dict[str, str], bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
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
    for h in extra_headers or []:
        cmd.extend(["-H", h])
    cmd.append(url)
    subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    status, headers = parse_headers(hdr)
    try:
        body_path.unlink(missing_ok=True)
        hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    return status, headers, body


def raw_head(port: int, path: str, headers: list[str]) -> tuple[str, dict[str, str], bytes]:
    req = f"HEAD {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
    for h in headers:
        req += h + "\r\n"
    req += "Connection: close\r\n\r\n"
    sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
    sock.sendall(req.encode())
    buf = b""
    while True:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buf += chunk
    sock.close()
    if b"\r\n\r\n" not in buf:
        return "", {}, buf
    head, rest = buf.split(b"\r\n\r\n", 1)
    status_line = head.split(b"\r\n", 1)[0].decode(errors="replace")
    hdrs: dict[str, str] = {}
    for line in head.split(b"\r\n")[1:]:
        if b":" in line:
            k, v = line.split(b":", 1)
            hdrs[k.decode().lower()] = v.strip().decode()
    return status_line, hdrs, rest


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"}) + "\n"
        )
        return 2

    doc = EV / "docroot"
    doc.mkdir(parents=True, exist_ok=True)
    (doc / "text.txt").write_bytes(TEXT)
    (doc / "bin1k.bin").write_bytes(BINARY_1K)
    (doc / "one.bin").write_bytes(ONE)
    (doc / "empty.bin").write_bytes(EMPTY)
    (doc / "mut.txt").write_bytes(b"version-A-payload!!!!")

    port = pick_port()
    cfg = EV / "exyonq-cap020.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["assets"]

[[route]]
name = "assets"
match = {{ path = "/assets" }}
root = "{doc}"
"""
    )
    log = EV / "exyonq-cap020.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    checks: dict[str, object] = {}
    result: dict = {
        "FEATURE_ID": "conditional-validators",
        "CAPABILITY": "CAPABILITY_020",
        "CAPABILITY_NAME": "conditional-validators",
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
        "PRODUCT_CONTRACT": "If-None-Match/If-Modified-Since → 304",
        "SUPPORTED_BEHAVIOR": "weak ETag + Last-Modified + INM/IMS → 304 on Cap004 static",
        "EXPLICIT_NON_SCOPE": [
            "If-Match",
            "If-Unmodified-Since",
            "If-Range",
            "412",
            "proxy conditionals",
        ],
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

        # Baseline GET
        st, hdrs, body = curl_req(f"{base}/text.txt")
        ok = "200" in st and body == TEXT and "etag" in hdrs and "last-modified" in hdrs
        checks["get_200_validators"] = {
            "ok": ok,
            "status": st,
            "etag": hdrs.get("etag"),
            "last_modified": hdrs.get("last-modified"),
            "body_sha": sha256_bytes(body),
        }
        etag_a = hdrs.get("etag", "")
        lm_a = hdrs.get("last-modified", "")
        body_sha = sha256_bytes(body)

        # INM match → 304
        st, hdrs, body = curl_req(f"{base}/text.txt", extra_headers=[f"If-None-Match: {etag_a}"])
        checks["inm_match_304"] = {
            "ok": "304" in st and body == b"" and hdrs.get("etag") == etag_a,
            "status": st,
            "body_len": len(body),
        }

        # INM non-match → 200 same body
        st, hdrs, body = curl_req(
            f"{base}/text.txt", extra_headers=['If-None-Match: W/"exq-other"']
        )
        checks["inm_nonmatch_200"] = {
            "ok": "200" in st and sha256_bytes(body) == body_sha,
            "status": st,
        }

        # IMS match → 304
        st, hdrs, body = curl_req(f"{base}/text.txt", extra_headers=[f"If-Modified-Since: {lm_a}"])
        checks["ims_match_304"] = {
            "ok": "304" in st and body == b"",
            "status": st,
            "body_len": len(body),
        }

        # IMS older → 200
        st, hdrs, body = curl_req(
            f"{base}/text.txt",
            extra_headers=["If-Modified-Since: Thu, 01 Jan 1970 00:00:00 GMT"],
        )
        checks["ims_older_200"] = {
            "ok": "200" in st and sha256_bytes(body) == body_sha,
            "status": st,
        }

        # Precedence: INM match + IMS old → 304
        st, hdrs, body = curl_req(
            f"{base}/text.txt",
            extra_headers=[
                f"If-None-Match: {etag_a}",
                "If-Modified-Since: Thu, 01 Jan 1970 00:00:00 GMT",
            ],
        )
        checks["precedence_inm_match"] = {
            "ok": "304" in st and body == b"",
            "status": st,
        }

        # Precedence: INM nonmatch + IMS match → 200
        st, hdrs, body = curl_req(
            f"{base}/text.txt",
            extra_headers=[
                'If-None-Match: W/"nope"',
                f"If-Modified-Since: {lm_a}",
            ],
        )
        checks["precedence_inm_nonmatch"] = {
            "ok": "200" in st and sha256_bytes(body) == body_sha,
            "status": st,
        }

        # HEAD match / nonmatch
        st, hdrs, rest = raw_head(
            port, "/assets/text.txt", [f"If-None-Match: {etag_a}"]
        )
        checks["head_inm_match_304"] = {
            "ok": "304" in st and rest == b"",
            "status": st,
            "body_len": len(rest),
        }
        st, hdrs, rest = raw_head(
            port, "/assets/text.txt", ['If-None-Match: W/"nope"']
        )
        checks["head_inm_nonmatch_200"] = {
            "ok": "200" in st and rest == b"",
            "status": st,
            "body_len": len(rest),
        }

        # Cap019 protectors + Cap020 Range interaction
        st, hdrs, body = curl_req(
            f"{base}/text.txt", extra_headers=["Range: bytes=0-9"]
        )
        checks["cap019_range_206"] = {
            "ok": "206" in st and body == TEXT[0:10] and "etag" in hdrs,
            "status": st,
            "body_sha": sha256_bytes(body),
        }
        st, hdrs, body = curl_req(
            f"{base}/text.txt", extra_headers=["Range: bytes=5000-"]
        )
        checks["cap019_range_416"] = {
            "ok": "416" in st and body == b"",
            "status": st,
        }
        st, hdrs, body = curl_req(
            f"{base}/text.txt",
            extra_headers=[f"If-None-Match: {etag_a}", "Range: bytes=0-9"],
        )
        checks["inm_match_with_range_304"] = {
            "ok": "304" in st and body == b"" and "206" not in st,
            "status": st,
            "body_len": len(body),
        }

        # Malformed → ignore → 200
        st, hdrs, body = curl_req(
            f"{base}/text.txt",
            extra_headers=["If-None-Match: not-a-tag", "If-Modified-Since: not-a-date"],
        )
        checks["malformed_ignore_200"] = {
            "ok": "200" in st and sha256_bytes(body) == body_sha,
            "status": st,
        }

        # Wildcard
        st, hdrs, body = curl_req(f"{base}/text.txt", extra_headers=["If-None-Match: *"])
        checks["inm_star_304"] = {"ok": "304" in st and body == b"", "status": st}

        # Mutation: version A → 304; replace → B; old INM → 200; new → 304
        st, hdrs, body = curl_req(f"{base}/mut.txt")
        etag_mut_a = hdrs.get("etag", "")
        checks["mut_a_200"] = {"ok": "200" in st and body == b"version-A-payload!!!!", "etag": etag_mut_a}
        st, hdrs, body = curl_req(
            f"{base}/mut.txt", extra_headers=[f"If-None-Match: {etag_mut_a}"]
        )
        checks["mut_a_304"] = {"ok": "304" in st, "status": st}

        # Ensure mtime advances beyond 1s FS granularity
        time.sleep(1.1)
        (doc / "mut.txt").write_bytes(b"version-B-CHANGED!!!!!!")
        # touch mtime aggressively
        now = time.time() + 2
        os.utime(doc / "mut.txt", (now, now))

        st, hdrs, body = curl_req(f"{base}/mut.txt")
        etag_mut_b = hdrs.get("etag", "")
        checks["mut_b_200"] = {
            "ok": "200" in st
            and body == b"version-B-CHANGED!!!!!!"
            and etag_mut_b != etag_mut_a
            and etag_mut_b != "",
            "etag_a": etag_mut_a,
            "etag_b": etag_mut_b,
        }
        st, hdrs, body = curl_req(
            f"{base}/mut.txt", extra_headers=[f"If-None-Match: {etag_mut_a}"]
        )
        checks["mut_old_inm_200"] = {
            "ok": "200" in st and body == b"version-B-CHANGED!!!!!!",
            "status": st,
        }
        st, hdrs, body = curl_req(
            f"{base}/mut.txt", extra_headers=[f"If-None-Match: {etag_mut_b}"]
        )
        checks["mut_new_inm_304"] = {"ok": "304" in st and body == b"", "status": st}

        # Concurrency mix
        conc_ok = True
        conc_detail = []

        def worker(name: str, headers: list[str], expect_substr: str):
            nonlocal conc_ok
            st, _, b = curl_req(f"{base}/text.txt", extra_headers=headers)
            good = expect_substr in st
            if expect_substr == "200" and good:
                good = sha256_bytes(b) == body_sha
            if expect_substr == "304" and good:
                good = b == b""
            if not good:
                conc_ok = False
            conc_detail.append({"name": name, "ok": good, "status": st})

        threads = [
            threading.Thread(target=worker, args=("full", [], "200")),
            threading.Thread(
                target=worker, args=("inm", [f"If-None-Match: {etag_a}"], "304")
            ),
            threading.Thread(
                target=worker, args=("range", ["Range: bytes=0-3"], "206")
            ),
            threading.Thread(
                target=worker,
                args=("inm_range", [f"If-None-Match: {etag_a}", "Range: bytes=0-3"], "304"),
            ),
        ]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        checks["concurrency"] = {"ok": conc_ok, "results": conc_detail}

        # Keepalive sequential on one TCP connection
        ka_ok = True
        sock = socket.create_connection(("127.0.0.1", port), timeout=5.0)
        leftover = b""
        expectations = (
            ([], "200", TEXT),
            ([f"If-None-Match: {etag_a}"], "304", b""),
            (["Range: bytes=0-0"], "206", TEXT[0:1]),
        )
        for headers, expect_st, expect_body in expectations:
            req = "GET /assets/text.txt HTTP/1.1\r\nHost: 127.0.0.1\r\n"
            for h in headers:
                req += h + "\r\n"
            req += "Connection: keep-alive\r\n\r\n"
            sock.sendall(req.encode())
            buf = leftover
            while b"\r\n\r\n" not in buf:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                buf += chunk
            if b"\r\n\r\n" not in buf:
                ka_ok = False
                break
            head, rest = buf.split(b"\r\n\r\n", 1)
            status_line = head.split(b"\r\n", 1)[0].decode(errors="replace")
            cl = 0
            for line in head.split(b"\r\n")[1:]:
                if line.lower().startswith(b"content-length:"):
                    cl = int(line.split(b":", 1)[1].strip())
            while len(rest) < cl:
                chunk = sock.recv(4096)
                if not chunk:
                    break
                rest += chunk
            body = rest[:cl]
            leftover = rest[cl:]
            if expect_st not in status_line or body != expect_body:
                ka_ok = False
                break
        sock.close()
        checks["keepalive"] = {"ok": ka_ok}

        failed = [k for k, v in checks.items() if not (isinstance(v, dict) and v.get("ok") is True)]
        if failed:
            result["FINAL_RESULT"] = "FAIL"
            result["FAILED_CHECKS"] = failed
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 1
        result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
