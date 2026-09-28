#!/usr/bin/env python3
"""Redis cache config must not start serving when the endpoint carries userinfo.

Positive HIT/MISS against a live Redis is NOT_EXECUTED here: it needs the
full-page cache, HMAC key and an origin. This oracle only proves fail-closed
startup. A listen on the configured port is a failure.
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


def sha256_file(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=0.3):
            return True
    except OSError:
        return False


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    port = pick_port()
    key = EV / "hmac.key"
    key.write_bytes(b"not-a-live-redis-proof")
    cfg = EV / "cfg.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["api"]
[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "backend"
[[upstream]]
name = "backend"
target = "http://127.0.0.1:9"
[full_page_cache]
enabled = true
namespace = 4
[full_page_cache.distributed_cache]
enabled = true
provider = "redis"
invalidation_enabled = true
generation_enabled = true
[full_page_cache.distributed_cache.redis]
endpoint = "redis://:secret@127.0.0.1:6379/"
[full_page_cache.distributed_cache.security]
active_key_id = "k1"
active_key_file = "{key}"
"""
    )
    log = EV / "exyonq.log"
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    time.sleep(2)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        pass
    up = listening(port)
    alive = proc.poll() is None
    log_text = log.read_text(errors="replace")
    ok = (not up) and (not alive)
    result = {
        "FEATURE_ID": "redis-failclosed-userinfo",
        "ORACLE_ORIGIN": "PROCESS_MUST_NOT_LISTEN",
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else "MISSING",
        "listening": up,
        "still_running": alive,
        "exit_code": proc.poll(),
        "POSITIVE_REDIS_HIT": "NOT_EXECUTED",
        "LOG_TAIL": log_text[-1500:],
        "FINAL_RESULT": "PASS_REAL_E2E" if ok else "FAIL_REAL_E2E",
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
    }
    if alive:
        proc.kill()
        proc.wait(timeout=5)
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
