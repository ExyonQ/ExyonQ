#!/usr/bin/env python3
"""Q-04 / ADR-057 — X-Cache miss then hit on FPC static; bypass reason; cache explain."""
from __future__ import annotations

import json
import os
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
HEAD = os.environ.get("HEAD", "UNKNOWN")
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "debug" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL", str(WS / "target" / "debug" / "exyonqctl")))


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_pred(pred, timeout: float = 45.0) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.05)
    return False


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), 0.25):
            return True
    except OSError:
        return False


def http_get(port: int, path: str, cookie: str | None = None) -> dict:
    hdrs = [f"GET {path} HTTP/1.1", "Host: q04.example", "Connection: close"]
    if cookie:
        hdrs.append(f"Cookie: {cookie}")
    hdrs.append("")
    hdrs.append("")
    req = "\r\n".join(hdrs).encode()
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=2.0)
        s.settimeout(8.0)
        s.sendall(req)
        chunks: list[bytes] = []
        while True:
            try:
                b = s.recv(65536)
            except TimeoutError:
                break
            if not b:
                break
            chunks.append(b)
        s.close()
    except OSError as e:
        return {"error": str(e), "status": None, "headers": {}}
    raw = b"".join(chunks).decode("latin-1", errors="replace")
    status = None
    headers: dict[str, str] = {}
    if raw.startswith("HTTP/"):
        lines = raw.split("\r\n")
        try:
            status = int(lines[0].split(" ", 2)[1])
        except (IndexError, ValueError):
            status = None
        for ln in lines[1:]:
            if not ln:
                break
            if ":" in ln:
                k, v = ln.split(":", 1)
                headers[k.strip().lower()] = v.strip()
    return {"status": status, "headers": headers, "raw": raw[:1500], "error": None}


def main() -> int:
    if not BIN.is_file() or not CTL.is_file():
        print(f"FAIL: missing {BIN} or {CTL}", file=sys.stderr)
        return 2
    EV.mkdir(parents=True, exist_ok=True)
    listen = pick_port()
    td = Path(tempfile.mkdtemp(prefix="q04-xc-", dir=str(EV)))
    www = td / "www"
    www.mkdir()
    (www / "index.html").write_text("q04-fpc-body\n", encoding="utf-8")
    cfg = f"""
config_version = 1

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "{www}"

[full_page_cache]
enabled = true
namespace = 4
max_entries = 64
max_total_bytes = 1048576
max_object_bytes = 1048576
default_ttl_seconds = 60
max_ttl_seconds = 300

[modules.metrics]
enabled = true
path = "/metrics"
"""
    cfg_path = td / "exyonq.toml"
    cfg_path.write_text(cfg, encoding="utf-8")
    sock = Path(f"/tmp/exyonq-q04-{os.getpid()}-{listen}.sock")
    if sock.exists():
        sock.unlink()
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_LOG_LEVEL"] = "warn"
    log_path = td / "exyonq.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(cfg_path)],
        cwd=str(WS),
        env=env,
        stdout=log_path.open("w", encoding="utf-8"),
        stderr=subprocess.STDOUT,
    )
    checks: dict = {}
    try:
        if not wait_pred(lambda: listening(listen), 40.0):
            checks["listen"] = {"ok": False}
            verdict = "FAIL"
        else:
            r1 = http_get(listen, "/")
            r2 = http_get(listen, "/")
            rb = http_get(listen, "/", cookie="wordpress_logged_in_x=1")
            checks["miss"] = {
                "ok": r1.get("status") == 200
                and r1.get("headers", {}).get("x-cache") == "miss",
                "status": r1.get("status"),
                "x_cache": r1.get("headers", {}).get("x-cache"),
            }
            checks["hit"] = {
                "ok": r2.get("status") == 200
                and r2.get("headers", {}).get("x-cache") == "hit",
                "status": r2.get("status"),
                "x_cache": r2.get("headers", {}).get("x-cache"),
            }
            checks["bypass"] = {
                "ok": rb.get("headers", {}).get("x-cache") == "bypass"
                and rb.get("headers", {}).get("x-cache-reason")
                in ("cookie", "cookie_denied", "cookie_present"),
                "x_cache": rb.get("headers", {}).get("x-cache"),
                "reason": rb.get("headers", {}).get("x-cache-reason"),
            }
            expl = subprocess.run(
                [
                    str(CTL),
                    "cache",
                    "explain",
                    "--config",
                    str(cfg_path),
                    "http://q04.example/wp-admin/",
                ],
                cwd=str(WS),
                capture_output=True,
                text=True,
            )
            expl_out = (expl.stdout or "") + (expl.stderr or "")
            checks["explain"] = {
                "ok": expl.returncode == 0
                and "decision = bypass" in expl_out
                and "wp_admin" in expl_out,
                "excerpt": expl_out[:800],
            }
            stats = subprocess.run(
                [
                    str(CTL),
                    "cache",
                    "stats",
                    "--url",
                    f"http://127.0.0.1:{listen}/metrics",
                ],
                cwd=str(WS),
                capture_output=True,
                text=True,
            )
            stats_out = (stats.stdout or "") + (stats.stderr or "")
            checks["stats"] = {
                "ok": stats.returncode == 0
                and "exyonq_fpc_hit_total" in stats_out
                and "exyonq_fpc_miss_total" in stats_out,
                "excerpt": stats_out[:800],
            }
            verdict = (
                "PASS" if all(c.get("ok") for c in checks.values()) else "FAIL"
            )
    finally:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=2)
        if sock.exists():
            try:
                sock.unlink()
            except OSError:
                pass

    result = {
        "slice": "Q-04",
        "adr": "ADR-057",
        "head": HEAD,
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
        "verdict": verdict,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"verdict": verdict, "out": str(OUT)}))
    return 0 if verdict == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
