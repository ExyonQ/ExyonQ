#!/usr/bin/env python3
"""NGINX static-mvp import, then serve the emitted TOML.

The document root and body bytes are chosen here. `proxy_pass` must be
rejected and must not leave a servable config.
"""
from __future__ import annotations

import hashlib
import json
import os
import socket
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))
COMPAT = Path(os.environ.get("EXYONQ_COMPAT_BIN", str(WS / "target" / "release" / "exyonq-compat")))
BODY = b"nginx-import-oracle-body-v1"


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_listen(port: int, timeout: float = 20.0) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def http_get(port: int) -> tuple[int, bytes]:
    req = "GET /index.html HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    with socket.create_connection(("127.0.0.1", port), timeout=5) as sock:
        sock.sendall(req.encode())
        buf = b""
        while True:
            chunk = sock.recv(65536)
            if not chunk:
                break
            buf += chunk
    head, _, body = buf.partition(b"\r\n\r\n")
    status = 0
    parts = head.split(b" ", 2)
    if len(parts) >= 2 and parts[1].isdigit():
        status = int(parts[1])
    return status, body


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    (root / "index.html").write_bytes(BODY)
    port = pick_port()
    nginx = EV / "nginx.conf"
    nginx.write_text(
        f"""server {{
    listen 127.0.0.1:{port};
    root {root};
    index index.html;
}}
"""
    )
    rejected = EV / "nginx-proxy.conf"
    rejected.write_text(
        f"""server {{
    listen 127.0.0.1:{port};
    location / {{
        proxy_pass http://127.0.0.1:9;
    }}
}}
"""
    )
    good_out = EV / "imported.toml"
    bad_out = EV / "rejected.toml"
    compat_out = EV / "compat.toml"
    compat_report = EV / "compat-report.json"
    compat = subprocess.run(
        [
            str(COMPAT),
            "migrate",
            "--from",
            "nginx",
            "--input",
            str(nginx),
            "--out",
            str(compat_out),
            "--report",
            str(compat_report),
        ],
        capture_output=True,
        text=True,
    )
    compat_text = compat_out.read_text(errors="replace") if compat_out.is_file() else ""
    compat_ok = compat.returncode == 0 and str(root) in compat_text
    bad = subprocess.run(
        [
            str(CTL),
            "config",
            "migrate-nginx",
            "--input",
            str(rejected),
            "--profile",
            "static-mvp",
            "--write",
            "--output",
            str(bad_out),
        ],
        capture_output=True,
        text=True,
    )
    reject_ok = bad.returncode != 0 and (
        not bad_out.exists() or bad_out.read_text(errors="replace").strip() == ""
    )
    good = subprocess.run(
        [
            str(CTL),
            "config",
            "migrate-nginx",
            "--input",
            str(nginx),
            "--profile",
            "static-mvp",
            "--write",
            "--output",
            str(good_out),
        ],
        capture_output=True,
        text=True,
    )
    result: dict = {
        "FEATURE_ID": "nginx-static-mvp-import",
        "ORACLE_ORIGIN": "NGINX_TEXT_AND_FILE_BYTES",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "EXYONQCTL_SHA256": sha256_file(CTL) if CTL.is_file() else "MISSING",
        "reject_exit": bad.returncode,
        "import_exit": good.returncode,
        "compat_exit": compat.returncode,
        "compat_ok": compat_ok,
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    if good.returncode != 0 or not good_out.is_file():
        result.update(
            {
                "FINAL_RESULT": "FAIL_REAL_E2E",
                "DETAIL": "static-mvp import failed",
                "stderr": (good.stderr or "")[-2000:],
                "reject_ok": reject_ok,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 1
    log = EV / "exyonq.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(good_out)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    try:
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "FAIL_REAL_E2E",
                    "DETAIL": "imported config did not listen",
                    "LOG_TAIL": log.read_text(errors="replace")[-2000:],
                    "reject_ok": reject_ok,
                    "imported_toml": good_out.read_text(errors="replace")[:2000],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 1
        status, body = http_get(port)
        body_ok = status == 200 and body == BODY
        ok = body_ok and reject_ok and compat_ok
        result.update(
            {
                "status": status,
                "body_match": body_ok,
                "reject_ok": reject_ok,
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
