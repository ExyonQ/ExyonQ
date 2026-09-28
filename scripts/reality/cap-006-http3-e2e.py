#!/usr/bin/env python3
"""CAPABILITY_006 = http-3-quic — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = http-3-quic
  USER_VISIBLE_CONTRACT = UDP http3_listen; GET/POST over H3; stream completion
  CONFIG_SURFACE = [http3]; server.http3_listen
  OPEN_DEFECT_IDS = RD-008 (stream-clean — prove on current HEAD)

Authoritative path: release binary (default s2n) → ephemeral TLS → UDP http3_listen
→ real curl --http3-only (Docker) → HTTP/3 GET/POST → body SHA256 → clean client exit.

HISTORICAL_POST_BODY_RESET_FIX = APPLICABLE_CONTEXT_ONLY (not final evidence).

EXPLICIT_NON_SCOPE:
  - Cap 007+
  - Quinn-legacy as runtime proof
  - HTTP/2 or HTTP/1.1 fallback as H3 PASS
  - competitive RPS / official bench
  - Cap004/005 beyond H3 serving path
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
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
CLIENT_IMAGE = os.environ.get("H3_CLIENT_IMAGE", "ymuski/curl-http3:latest")

GET_BODY = b"cap006-h3-static-get-v1\n"
POST_ECHO_MARKER = b"cap006-post-body-v1"


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


def wait_log(path: Path, needle: str, timeout: float = 60.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists() and needle in path.read_text(errors="ignore"):
            return True
        time.sleep(0.1)
    return False


def docker_plat() -> str:
    m = os.uname().machine
    if m in ("x86_64", "amd64"):
        return "linux/amd64"
    if m in ("aarch64", "arm64"):
        # curl-http3 image is amd64-only; qemu on arm64 hosts.
        return "linux/amd64"
    raise RuntimeError(f"unsupported arch {m}")


def detect_provider_built(binary: Path) -> str:
    try:
        strings = subprocess.run(
            ["strings", str(binary)],
            capture_output=True,
            text=True,
            timeout=60,
        )
        text = strings.stdout or ""
    except (OSError, subprocess.TimeoutExpired):
        text = ""
    # Prefer product default markers; Quinn legacy must not be claimed as executed.
    if "s2n_quic" in text or "s2n-quic" in text or "Http3ProviderId::S2n" in text or "provider = s2n" in text:
        # Also check for exclusive quinn-only (unlikely with default features)
        if "http3-provider-quinn-legacy" in text and "s2n" not in text.lower():
            return "quinn-legacy-only?"
        return "s2n"
    if "quiche" in text.lower() and "s2n" not in text.lower():
        return "quiche?"
    return "UNKNOWN_FROM_STRINGS"


def h3_curl(args: list[str], *, tmp: Path, timeout: int = 30) -> subprocess.CompletedProcess:
    plat = docker_plat()
    cmd = [
        "docker",
        "run",
        "--rm",
        "--platform",
        plat,
        "--network",
        "host",
        "-v",
        f"{tmp}:/e2e",
        CLIENT_IMAGE,
        "curl",
        "--http3-only",
        "-skS",
        "--max-time",
        str(timeout),
        *args,
    ]
    return subprocess.run(cmd, capture_output=True, text=True)


def start_upstream(port: int, state_dir: Path) -> subprocess.Popen:
    state_dir.mkdir(parents=True, exist_ok=True)
    (state_dir / "mode").write_text("ready\n")
    script = f"""
import hashlib, os, socket, threading
addr = ("127.0.0.1", {port})
sd = {str(state_dir)!r}
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(addr)
s.listen(64)

def serve(c):
    data = b""
    c.settimeout(10)
    try:
        while b"\\r\\n\\r\\n" not in data:
            chunk = c.recv(4096)
            if not chunk:
                break
            data += chunk
        hdr, body = data.split(b"\\r\\n\\r\\n", 1)
        cl = 0
        for line in hdr.decode("latin1", "ignore").split("\\r\\n"):
            if line.lower().startswith("content-length:"):
                cl = int(line.split(":", 1)[1].strip() or 0)
        while len(body) < cl:
            chunk = c.recv(max(cl - len(body), 1))
            if not chunk:
                break
            body += chunk
        body = body[:cl]
        open(os.path.join(sd, "last_body"), "wb").write(body)
        open(os.path.join(sd, "last_sha"), "w").write(hashlib.sha256(body).hexdigest())
        resp = (
            f"HTTP/1.1 200 OK\\r\\nContent-Length: {{len(body)}}\\r\\n"
            f"Content-Type: application/octet-stream\\r\\nConnection: close\\r\\n\\r\\n"
        ).encode() + body
        c.sendall(resp)
    except Exception:
        pass
    finally:
        try:
            c.close()
        except Exception:
            pass

while True:
    c, _ = s.accept()
    threading.Thread(target=serve, args=(c,), daemon=True).start()
"""
    return subprocess.Popen(
        [sys.executable, "-c", script],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "http-3-quic",
        "CAPABILITY": "CAPABILITY_006",
        "CAPABILITY_NAME": "http-3-quic",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "UDP http3_listen; GET/POST over H3; stream completion",
        "SUPPORTED_BEHAVIOR": "s2n-default H3 GET/POST; stream-clean client completion; body integrity",
        "EXPLICIT_NON_SCOPE": [
            "Cap007+",
            "Quinn-legacy as runtime proof",
            "H1/H2 fallback as H3 PASS",
            "competitive RPS",
            "Cap004/005 beyond H3 path",
        ],
        "HISTORICAL_POST_BODY_RESET_FIX": "APPLICABLE_CONTEXT_ONLY",
        "OPEN_DEFECT_CONTEXT": "RD-008 stream-clean — prove on current HEAD",
        "HTTP3_PROVIDER_EXPECTED": "s2n",
        "TRANSPORT_RESET_COUNT_DIRECTLY_OBSERVED": "NO",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_007_STARTED": "NO",
            "H3_CLIENT": CLIENT_IMAGE,
        },
    }

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": f"missing {BINARY}"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    if not shutil.which("docker"):
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "docker required for curl-http3"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    result["HTTP3_PROVIDER_ACTUALLY_BUILT"] = detect_provider_built(BINARY)

    plat = docker_plat()
    pull = subprocess.run(
        ["docker", "pull", "--platform", plat, CLIENT_IMAGE],
        capture_output=True,
        text=True,
    )
    if pull.returncode != 0:
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "docker pull failed",
                "pull_stderr": (pull.stderr or "")[-2000:],
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    tmp = Path(tempfile.mkdtemp(prefix="cap006-h3-", dir=str(EV)))
    www = tmp / "www"
    www.mkdir()
    (www / "index.html").write_bytes(GET_BODY)
    want_get_sha = sha256_bytes(GET_BODY)
    post_payload = POST_ECHO_MARKER + os.urandom(64)
    want_post_sha = sha256_bytes(post_payload)
    (tmp / "post.bin").write_bytes(post_payload)

    gen = subprocess.run(
        [
            "bash",
            str(WS / "scripts/test-tls/generate-ephemeral-tls.sh"),
            "--print-paths",
            "--tmpdir",
            str(tmp),
        ],
        capture_output=True,
        text=True,
    )
    if gen.returncode != 0:
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "tls generation failed",
                "tls_stderr": (gen.stderr + gen.stdout)[-2000:],
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    tls_dir = cert = key = None
    for line in gen.stdout.splitlines():
        if line.startswith("DIR="):
            tls_dir = Path(line.split("=", 1)[1])
        elif line.startswith("CERT="):
            cert = Path(line.split("=", 1)[1])
        elif line.startswith("KEY="):
            key = Path(line.split("=", 1)[1])
    if cert is None or key is None or not cert.is_file() or not key.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "tls paths missing"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    tcp = pick_port()
    udp = pick_port()
    up_port = pick_port()
    up_state = tmp / "up"
    up_proc = start_upstream(up_port, up_state)
    time.sleep(0.3)

    cfg = tmp / "exyonq.toml"
    cfg.write_text(
        f"""config_version = 1
[http3]
enabled = true
provider = "s2n"
request_body_drain_cap_bytes = 1048576
[[server]]
listen = "127.0.0.1:{tcp}"
http3_listen = "0.0.0.0:{udp}"
routes = ["site", "api"]
[server.tls]
cert = "{cert}"
key = "{key}"
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{www}"
index = "index.html"
[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:{up_port}"
timeout_ms = 5000
"""
    )
    result["CONFIG_SHA256"] = sha256_file(cfg)
    log = EV / "exyonq-h3.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    try:
        if not wait_log(log, "HTTP/3 listening", 60):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "HTTP/3 listener not ready",
                    "LOG_TAIL": log.read_text(errors="replace")[-4000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        log_text = log.read_text(errors="replace")
        log_plain = re.sub(r"\x1b\[[0-9;]*m", "", log_text)
        executed = "UNKNOWN"
        if re.search(r'HTTP/3 provider selected.*provider\s*=\s*"s2n"', log_plain, re.DOTALL):
            executed = "s2n"
        elif re.search(r'HTTP/3 listening \(s2n-quic provider\).*provider\s*=\s*"s2n"', log_plain, re.DOTALL):
            executed = "s2n"
        else:
            m = re.search(r'provider\s*=\s*"?([a-z0-9\-]+)"?', log_plain)
            if m:
                executed = m.group(1)
        result["HTTP3_PROVIDER_ACTUALLY_EXECUTED"] = executed
        result["HTTP3_LISTENER_LOG"] = "YES"

        # POSITIVE GET
        url_get = f"https://127.0.0.1:{udp}/site/"
        body_out = tmp / "get-body.bin"
        r = h3_curl(
            [
                "-o",
                "/e2e/get-body.bin",
                "-D",
                "/e2e/get-hdr.txt",
                "-w",
                "%{http_code}|%{http_version}|%{ssl_verify_result}|%{exitcode}",
                url_get,
            ],
            tmp=tmp,
        )
        (EV / "curl-get.stdout").write_text(r.stdout or "")
        (EV / "curl-get.stderr").write_text(r.stderr or "")
        parts = (r.stdout or "").strip().split("|")
        code = parts[0] if parts else "000"
        ver = parts[1].strip() if len(parts) > 1 else ""
        got = body_out.read_bytes() if body_out.is_file() else b""
        get_sha_ok = got == GET_BODY and sha256_bytes(got) == want_get_sha
        h3_neg = ver in ("3", "3.0") and code == "200" and r.returncode == 0
        # TLS1.3: curl-http3 over QUIC is TLS1.3 by protocol; record from verbose if present
        rv = h3_curl(["-v", "-o", "/dev/null", url_get], tmp=tmp, timeout=20)
        tls13 = "TLSv1.3" in (rv.stderr or "") or "TLS 1.3" in (rv.stderr or "") or h3_neg
        (EV / "curl-get-verbose.stderr").write_text(rv.stderr or "")

        # MULTIPLE sequential GETs
        seq_ok = 0
        for _ in range(5):
            rr = h3_curl(["-o", "/dev/null", "-w", "%{http_code}|%{http_version}", url_get], tmp=tmp)
            p = (rr.stdout or "").strip().split("|")
            if rr.returncode == 0 and p[:1] == ["200"] and (len(p) < 2 or p[1] in ("3", "3.0")):
                seq_ok += 1

        # POST body (matrix includes POST)
        url_post = f"https://127.0.0.1:{udp}/api/echo"
        rpost = h3_curl(
            [
                "-X",
                "POST",
                "--data-binary",
                "@/e2e/post.bin",
                "-H",
                "Content-Type: application/octet-stream",
                "-o",
                "/e2e/post-resp.bin",
                "-w",
                "%{http_code}|%{http_version}",
                url_post,
            ],
            tmp=tmp,
        )
        (EV / "curl-post.stdout").write_text(rpost.stdout or "")
        (EV / "curl-post.stderr").write_text(rpost.stderr or "")
        pp = (rpost.stdout or "").strip().split("|")
        post_body = (tmp / "post-resp.bin").read_bytes() if (tmp / "post-resp.bin").is_file() else b""
        post_ok = (
            rpost.returncode == 0
            and pp[:1] == ["200"]
            and (len(pp) < 2 or pp[1] in ("3", "3.0"))
            and post_body == post_payload
            and sha256_bytes(post_body) == want_post_sha
        )

        # NEGATIVE: 404 over H3
        r404 = h3_curl(
            ["-o", "/dev/null", "-w", "%{http_code}|%{http_version}", f"https://127.0.0.1:{udp}/missing-cap006"],
            tmp=tmp,
        )
        p404 = (r404.stdout or "").strip().split("|")
        neg_ok = p404[:1] == ["404"] and (len(p404) < 2 or p404[1] in ("3", "3.0")) and r404.returncode == 0

        # FAILURE: wrong UDP port → no H3 success
        bad = h3_curl(
            ["-o", "/dev/null", "-w", "%{http_code}|%{http_version}", f"https://127.0.0.1:{pick_port()}/site/"],
            tmp=tmp,
            timeout=8,
        )
        bad_parts = (bad.stdout or "").strip().split("|")
        fail_ok = not (
            bad.returncode == 0 and bad_parts[:1] == ["200"] and (len(bad_parts) > 1 and bad_parts[1] in ("3", "3.0"))
        )

        # CONCURRENCY / multiplex-style parallel GETs
        def one(_i: int) -> bool:
            rr = h3_curl(["-o", "/dev/null", "-w", "%{http_code}|%{http_version}", url_get], tmp=tmp)
            p = (rr.stdout or "").strip().split("|")
            return rr.returncode == 0 and p[:1] == ["200"] and (len(p) < 2 or p[1] in ("3", "3.0"))

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            conc = list(ex.map(one, range(8)))
        conc_ok = len(conc) == 8 and all(conc)

        # Stream-clean alternate: multi-request success (RD-008). Do not invent RESET_COUNT=0.
        multi_started = 5 + 8 + (1 if post_ok else 0)
        multi_ok = seq_ok + sum(conc) + (1 if post_ok else 0)
        stream_clean = (
            h3_neg
            and get_sha_ok
            and seq_ok == 5
            and conc_ok
            and post_ok
            and r.returncode == 0
            and rpost.returncode == 0
        )

        provider_ok = result["HTTP3_PROVIDER_ACTUALLY_EXECUTED"] == "s2n"
        positive = h3_neg and get_sha_ok and post_ok and tls13
        multiplex = seq_ok == 5 and conc_ok
        contract_ok = (
            positive
            and neg_ok
            and fail_ok
            and multiplex
            and stream_clean
            and proc.poll() is None
        )
        overall = contract_ok and provider_ok

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if contract_ok else "YES",
                "HARNESS_DEFECT": "NO" if (overall or not contract_ok) else "YES",
                "ENVIRONMENT_BLOCKER": "NO",
                "REAL_QUIC_CONNECTION": "YES" if h3_neg else "NO",
                "REAL_HTTP3_NEGOTIATED": "YES" if h3_neg else "NO",
                "OBSERVED_HTTP_VERSION": f"HTTP/{ver}" if ver else "UNKNOWN",
                "HTTP3_ALPN": "h3" if h3_neg else "NOT_NEGOTIATED",
                "TLS_VERSION": "TLS1.3" if tls13 else "NOT_CONFIRMED",
                "CAP006_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP006_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP006_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP006_MULTIPLEX_STATUS": "PASS" if multiplex else "FAIL",
                "HTTP3_BODY_SHA256_STATUS": "PASS" if (get_sha_ok and post_ok) else "FAIL",
                "HTTP3_CLIENT_COMPLETION_STATUS": "PASS" if stream_clean else "FAIL",
                "HTTP3_POST_STATUS": "PASS" if post_ok else "FAIL",
                "STREAM_CLEAN_STATUS": "PASS" if stream_clean else "FAIL",
                "TRANSPORT_RESET_COUNT_DIRECTLY_OBSERVED": "NO",
                "checks": {
                    "get_h3": h3_neg,
                    "get_sha": get_sha_ok,
                    "get_code": code,
                    "get_version": ver,
                    "tls13": tls13,
                    "post_ok": post_ok,
                    "post_code": pp[0] if pp else "000",
                    "post_version": pp[1] if len(pp) > 1 else "",
                    "neg_404": neg_ok,
                    "fail_bad_udp": fail_ok,
                    "seq_ok": seq_ok,
                    "conc_ok": conc_ok,
                    "provider_executed": result["HTTP3_PROVIDER_ACTUALLY_EXECUTED"],
                    "provider_ok": provider_ok,
                    "multi_started": multi_started,
                    "multi_succeeded": multi_ok,
                },
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except Exception:
                proc.kill()
        up_proc.terminate()
        try:
            up_proc.wait(timeout=5)
        except Exception:
            up_proc.kill()
        if tls_dir is not None:
            subprocess.run(
                [
                    "bash",
                    str(WS / "scripts/test-tls/generate-ephemeral-tls.sh"),
                    "--cleanup",
                    str(tls_dir),
                ],
                check=False,
                capture_output=True,
            )


if __name__ == "__main__":
    sys.exit(main())
