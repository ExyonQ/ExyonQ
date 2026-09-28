#!/usr/bin/env python3
"""HTTP/3 via the quiche provider. Client is curl --http3-only, not the server decoder.

Requires EXYONQ_BIN built with `--features http3-provider-quiche --no-default-features`.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
BODY = b"quiche-oracle-body-v1"
CLIENT_IMAGE = os.environ.get("H3_CLIENT_IMAGE", "ymuski/curl-http3:latest")


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "http3-quiche",
        "ORACLE_ORIGIN": "CURL_HTTP3_AND_FILE_BYTES",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "EXPECTED_SHA256": sha256_bytes(BODY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "POSITIVE_S2N_PATH": "OUT_OF_SCOPE",
    }
    if subprocess.run(["docker", "info"], capture_output=True).returncode != 0:
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "docker required"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    tmp = Path(tempfile.mkdtemp(prefix="cap083-", dir=str(EV)))
    www = tmp / "www"
    www.mkdir()
    (www / "body.bin").write_bytes(BODY)
    gen = subprocess.run(
        ["bash", str(WS / "scripts/test-tls/generate-ephemeral-tls.sh"), "--print-paths", "--tmpdir", str(tmp)],
        capture_output=True,
        text=True,
    )
    if gen.returncode != 0:
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "tls generation failed", "stderr": gen.stderr[-1000:]})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    cert = key = None
    for line in gen.stdout.splitlines():
        if line.startswith("CERT="):
            cert = line.split("=", 1)[1]
        elif line.startswith("KEY="):
            key = line.split("=", 1)[1]
    tcp, udp = pick_port(), pick_port()
    cfg = tmp / "exyonq.toml"
    cfg.write_text(
        f"""config_version = 1
[http3]
enabled = true
provider = "quiche"
[[server]]
listen = "127.0.0.1:{tcp}"
http3_listen = "0.0.0.0:{udp}"
routes = ["site"]
[server.tls]
cert = "{cert}"
key = "{key}"
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{www}"
"""
    )
    log = EV / "exyonq-quiche.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    try:
        time.sleep(1.5)
        url = f"https://127.0.0.1:{udp}/site/body.bin"
        curl = subprocess.run(
            [
                "docker", "run", "--rm", "--network", "host",
                CLIENT_IMAGE, "curl", "--http3-only", "-skS", "--max-time", "20", url,
            ],
            capture_output=True,
        )
        log_text = log.read_text(errors="replace")
        provider_quiche = "quiche" in log_text.lower()
        provider_s2n_only = "s2n" in log_text.lower() and not provider_quiche
        body_ok = curl.returncode == 0 and curl.stdout == BODY
        ok = body_ok and provider_quiche and not provider_s2n_only
        result.update(
            {
                "curl_exit": curl.returncode,
                "body_match": body_ok,
                "provider_quiche_in_log": provider_quiche,
                "provider_s2n_only": provider_s2n_only,
                "curl_stderr": curl.stderr.decode("utf-8", "replace")[-1500:],
                "FINAL_RESULT": "PASS_REAL_E2E" if ok else "FAIL_REAL_E2E",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if ok else 1
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    raise SystemExit(main())
