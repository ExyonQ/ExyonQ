#!/usr/bin/env python3
"""CAPABILITY_003 = tls-termination — real product E2E (single capability).

Canonical matrix row FEATURE_ID=tls-termination:
  USER_VISIBLE_CONTRACT = cert+key; HTTPS GET; ALPN (reload excluded from this row)

Authoritative path: release binary → production TLS config with ephemeral cert+key
→ real TCP/TLS handshake → HTTPS GET → body SHA256 → ALPN observation.

EXPLICIT_NON_SCOPE for this capability:
  - TLS reload (matrix excludes reload from this row)
  - HTTP/2 multiplex stream campaign (Capability 002)
  - static-files Capability 004 closure (minimal route only to exercise HTTPS GET)
  - HTTP/3

No committed private keys. No synthetic TLS. No Cap 004 started.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import sys
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
GEN_TLS = WS / "scripts" / "test-tls" / "generate-ephemeral-tls.sh"

BODY = b"cap003-tls-termination-body-v1"
EXPECTED_BODY_SHA256 = hashlib.sha256(BODY).hexdigest()


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


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


def ensure_ephemeral_tls(ev: Path) -> tuple[Path, Path, Path]:
    if not GEN_TLS.is_file():
        raise FileNotFoundError(f"missing {GEN_TLS}")
    proc = subprocess.run(
        ["bash", str(GEN_TLS)],
        cwd=str(WS),
        capture_output=True,
        text=True,
        check=True,
    )
    env: dict[str, str] = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if line.startswith("export "):
            line = line[len("export ") :]
        if "=" not in line:
            continue
        k, v = line.split("=", 1)
        env[k] = v.strip().strip("'").strip('"')
    tls_dir = Path(env.get("EXYONQ_TLS_DIR", ""))
    cert = Path(env.get("EXYONQ_TLS_CERT", ""))
    key = Path(env.get("EXYONQ_TLS_KEY", ""))
    if not cert.is_file() or not key.is_file():
        raise RuntimeError(f"ephemeral TLS incomplete: {proc.stdout!r} {proc.stderr!r}")
    (ev / "tls_paths.txt").write_text(
        f"EXYONQ_TLS_DIR={tls_dir}\nCERT={cert}\nKEY={key}\nKEY_COMMITTED=NO\n"
    )
    return tls_dir, cert, key


def openssl_alpn(port: int, offer: str) -> dict:
    cmd = [
        "openssl",
        "s_client",
        "-connect",
        f"127.0.0.1:{port}",
        "-alpn",
        offer,
        "-servername",
        "localhost",
    ]
    try:
        proc = subprocess.run(cmd, input=b"", capture_output=True, timeout=10)
    except (FileNotFoundError, subprocess.TimeoutExpired) as e:
        return {"ok": False, "alpn": None, "detail": str(e)}
    text = (proc.stdout + proc.stderr).decode("latin-1", errors="replace")
    alpn = None
    for line in text.splitlines():
        if "ALPN protocol:" in line:
            alpn = line.split(":", 1)[1].strip()
            break
    # Also capture TLS version line
    tls_ver = None
    for line in text.splitlines():
        if line.startswith("New, TLS") or line.startswith("Protocol  :"):
            tls_ver = line.strip()
            break
    return {"ok": alpn is not None and alpn != "", "alpn": alpn, "tls_line": tls_ver}


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "tls-termination",
                    "CAPABILITY": "CAPABILITY_003",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "CLASSIFICATION": "ENVIRONMENT_DEFECT",
                    "DETAIL": f"missing binary {BINARY}",
                    "ARCH_LABEL": ARCH_LABEL,
                    "HOST_LABEL": HOST_LABEL,
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    tls_dir: Path | None = None
    try:
        tls_dir, cert, key = ensure_ephemeral_tls(EV)
    except Exception as e:  # noqa: BLE001
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "tls-termination",
                    "CAPABILITY": "CAPABILITY_003",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "CLASSIFICATION": "HARNESS_DEFECT",
                    "DETAIL": f"ephemeral TLS failed: {e}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 3

    www = EV / "www-tls"
    www.mkdir(parents=True, exist_ok=True)
    (www / "index.html").write_bytes(BODY)

    port = pick_port()
    cfg = EV / "cfg-tls.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]
tls = {{ cert = "{cert}", key = "{key}" }}

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{www}"
index = "index.html"
"""
    )
    log = EV / "exyonq-tls.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    result: dict = {
        "FEATURE_ID": "tls-termination",
        "CAPABILITY": "CAPABILITY_003",
        "CAPABILITY_NAME": "tls-termination",
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
        "EXPECTED_BODY_SHA256": EXPECTED_BODY_SHA256,
        "PRODUCT_CONTRACT": "cert+key; HTTPS GET; ALPN (reload excluded from this row)",
        "EXPLICIT_NON_SCOPE": [
            "tls-reload",
            "http-2-multiplex (Capability 002)",
            "static-files Capability 004 closure",
            "http-3",
        ],
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "TLS_EPHEMERAL": "YES",
            "PRIVATE_KEY_COMMITTED": "NO",
            "RELOAD_TESTED": "NO",
            "CAPABILITY_004_STARTED": "NO",
        },
    }
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "HARNESS_DEFECT": "YES",
                    "DETAIL": "listen timeout",
                    "LOG_TAIL": log.read_text(errors="replace")[-3000:],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        url = f"https://127.0.0.1:{port}/site/"

        # --- Positive: HTTPS GET over TLS (HTTP/1.1) — proves termination, not Cap002 mux ---
        body_path = EV / "https-get.body"
        get = subprocess.run(
            [
                "curl",
                "-skf",
                "--max-time",
                "20",
                "--http1.1",
                "-o",
                str(body_path),
                "-w",
                "%{http_version}:%{http_code}:%{ssl_verify_result}",
                url,
            ],
            capture_output=True,
            text=True,
        )
        meta = (get.stdout or "").strip()
        parts = meta.split(":")
        http_ver = parts[0] if parts else ""
        http_code = parts[1] if len(parts) > 1 else ""
        body = body_path.read_bytes() if body_path.is_file() else b""
        body_sha = sha256_bytes(body) if body else None
        https_get_ok = (
            get.returncode == 0
            and http_code == "200"
            and http_ver == "1.1"
            and body == BODY
            and body_sha == EXPECTED_BODY_SHA256
        )

        # --- ALPN: h2 preferred offer ---
        alpn_h2 = openssl_alpn(port, "h2,http/1.1")
        alpn_h2_ok = alpn_h2.get("alpn") == "h2"

        # --- ALPN: http/1.1 only offer ---
        alpn_11 = openssl_alpn(port, "http/1.1")
        alpn_11_ok = alpn_11.get("alpn") == "http/1.1"

        # --- Negative: plain HTTP to TLS port must NOT succeed as cleartext product path ---
        plain = subprocess.run(
            [
                "curl",
                "-sS",
                "--max-time",
                "5",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                f"http://127.0.0.1:{port}/site/",
            ],
            capture_output=True,
            text=True,
        )
        plain_code = (plain.stdout or "").strip()
        # Expect curl failure or non-200 — TLS port must not serve cleartext 200
        plain_ok = plain.returncode != 0 or plain_code not in ("200",)

        # --- Failure: missing file over HTTPS still TLS-terminated (404) ---
        miss = subprocess.run(
            [
                "curl",
                "-sk",
                "--max-time",
                "15",
                "--http1.1",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                f"https://127.0.0.1:{port}/site/missing-cap003.txt",
            ],
            capture_output=True,
            text=True,
        )
        miss_ok = (miss.stdout or "").strip() == "404"

        # --- Boundary: second HTTPS GET (connection lifecycle / reuse client) ---
        get2 = subprocess.run(
            [
                "curl",
                "-skf",
                "--max-time",
                "15",
                "--http1.1",
                "-o",
                str(EV / "https-get2.body"),
                url,
            ],
            capture_output=True,
        )
        body2 = (EV / "https-get2.body").read_bytes() if (EV / "https-get2.body").is_file() else b""
        lifecycle_ok = get2.returncode == 0 and body2 == BODY

        # --- Failure path: server refuses to start with missing cert (separate short process) ---
        bad_cfg = EV / "cfg-tls-bad.toml"
        bad_cfg.write_text(
            f"""config_version = 1
[[server]]
listen = "127.0.0.1:{pick_port()}"
routes = ["site"]
tls = {{ cert = "{EV / 'no-such-cert.pem'}", key = "{EV / 'no-such-key.pem'}" }}
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{www}"
index = "index.html"
"""
        )
        bad = subprocess.run(
            [str(BINARY), "serve", "--config", str(bad_cfg)],
            cwd=str(WS),
            capture_output=True,
            text=True,
            timeout=8,
        )
        # Must not remain running serving HTTPS successfully — expect non-zero exit quickly
        bad_start_ok = bad.returncode != 0

        positive = https_get_ok and alpn_h2_ok and alpn_11_ok and lifecycle_ok
        negative = plain_ok and miss_ok
        failure = bad_start_ok
        overall = positive and negative and failure

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CLASSIFICATION": "NONE" if overall else "PRODUCT_DEFECT",
                "TLS_HTTPS_GET_STATUS": "PASS" if https_get_ok else "FAIL",
                "TLS_ALPN_H2_STATUS": "PASS" if alpn_h2_ok else "FAIL",
                "TLS_ALPN_HTTP11_STATUS": "PASS" if alpn_11_ok else "FAIL",
                "TLS_PLAINTEXT_REJECT_STATUS": "PASS" if plain_ok else "FAIL",
                "TLS_MISSING_FILE_404_STATUS": "PASS" if miss_ok else "FAIL",
                "TLS_LIFECYCLE_STATUS": "PASS" if lifecycle_ok else "FAIL",
                "TLS_BAD_CERT_START_STATUS": "PASS" if bad_start_ok else "FAIL",
                "CAP003_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP003_NEGATIVE_STATUS": "PASS" if negative else "FAIL",
                "CAP003_FAILURE_STATUS": "PASS" if failure else "FAIL",
                "CAP003_BOUNDARY_STATUS": "PASS" if (alpn_h2_ok and alpn_11_ok) else "FAIL",
                "CAP003_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if lifecycle_ok else "FAIL",
                "BODY_SHA256_STATUS": "PASS"
                if body_sha == EXPECTED_BODY_SHA256
                else "FAIL",
                "BODY_SHA256": body_sha,
                "EXPECTED_BODY_SHA256": EXPECTED_BODY_SHA256,
                "OBSERVED_HTTP_VERSION_OVER_TLS": http_ver,
                "RELOAD_EXERCISED": "NO",
                "checks": {
                    "https_get_meta": meta,
                    "alpn_h2": alpn_h2,
                    "alpn_http11": alpn_11,
                    "plain_http_code": plain_code,
                    "plain_rc": plain.returncode,
                    "missing_404": (miss.stdout or "").strip(),
                    "bad_cert_rc": bad.returncode,
                    "bad_cert_stderr_tail": (bad.stderr or bad.stdout or "")[-800:],
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
        if tls_dir is not None and tls_dir.is_dir():
            subprocess.run(
                ["bash", str(GEN_TLS), "--cleanup", str(tls_dir)],
                cwd=str(WS),
                capture_output=True,
            )


if __name__ == "__main__":
    sys.exit(main())
