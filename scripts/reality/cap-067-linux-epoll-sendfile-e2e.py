#!/usr/bin/env python3
"""CAPABILITY_067 = linux-epoll-sendfile — REAL product E2E (Linux only).

Option A: automatic cleartext H1 sendfile for eligible static files.
Proves TARGET_PATH_EXECUTED via exyonq_epoll_sendfile_complete_total delta
(not HTTP success alone). Cap061 filename specialization must remain absent.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH = os.environ.get("ARCH_LABEL", "unknown")
HOST = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

RESULTS: list[dict] = []


def record(name: str, status: str, **extra) -> None:
    row = {"scenario": name, "status": status, **extra}
    RESULTS.append(row)
    print(f"[{status}] {name} {extra}", flush=True)


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_port(port: int, timeout: float = 60.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def curl(url: str, *, method: str = "GET", headers: list[str] | None = None, timeout: int = 30) -> tuple[int, str, bytes, str]:
    tag = f"{time.time_ns()}"
    body_p = EV / f"c-{tag}.body"
    hdr_p = EV / f"c-{tag}.hdr"
    # HEAD: use --head (CURLOPT_NOBODY). Do not use -o — some curl builds dump
    # response headers into -o under --head, which falsely fails an empty-body oracle.
    cmd = ["curl", "-sS", "--max-time", str(timeout), "-D", str(hdr_p)]
    if method.upper() == "HEAD":
        cmd.append("--head")
    else:
        cmd.extend(["-o", str(body_p), "-X", method])
    for h in headers or []:
        cmd.extend(["-H", h])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    if method.upper() == "HEAD":
        body = b""
    else:
        body = body_p.read_bytes() if body_p.is_file() else b""
    hdr = hdr_p.read_text(errors="replace") if hdr_p.is_file() else ""
    status = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
    return proc.returncode, status, body, hdr


def metric(text: str, name: str) -> int:
    m = re.search(rf"^{re.escape(name)} (\d+)\s*$", text, re.M)
    return int(m.group(1)) if m else -1


def write_config(path: Path, port: int, root: Path, ctl: Path) -> None:
    _ = ctl  # control socket not required for Cap067 sendfile contract
    path.write_text(
        f"""config_version = 1

[logging]
level = "info"
format = "text"

[logging.console]
enabled = true
stream = "stdout"

[logging.access]
enabled = true

[modules.metrics]
enabled = true

[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
index = "index.html"
""",
        encoding="utf-8",
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if os.uname().sysname != "Linux":
        record("platform", "FAIL", detail="Cap067 requires Linux")
        OUT.write_text(json.dumps({"OVERALL": "FAIL", "results": RESULTS}, indent=2) + "\n")
        return 1
    if not BIN.is_file():
        record("binary", "FAIL", path=str(BIN))
        OUT.write_text(json.dumps({"OVERALL": "FAIL", "results": RESULTS}, indent=2) + "\n")
        return 1

    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    files = {
        "empty.bin": b"",
        "small.bin": b"cap067-small",
        "medium.bin": bytes([0x41]) * (64 * 1024),
        "large.bin": bytes([0x42]) * (1024 * 1024),
        "index.html": b"<html>cap067</html>",
    }
    for name, data in files.items():
        (root / name).write_bytes(data)

    port = pick_port()
    cfg = EV / "exyonq.toml"
    ctl = EV / "control.sock"
    write_config(cfg, port, root, ctl)
    log = open(EV / "exyonq.log", "w", encoding="utf-8")
    env = os.environ.copy()
    # Cap067 auto-on: do NOT require EXYONQ_EPOLL_*=1. Kill-switch must remain unset.
    env.pop("EXYONQ_EPOLL_STATIC", None)
    env.pop("EXYONQ_EPOLL_SENDFILE", None)
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(cfg)],
        stdout=log,
        stderr=subprocess.STDOUT,
        env=env,
        cwd=str(WS),
    )
    try:
        if not wait_port(port):
            log_tail = ""
            try:
                log.flush()
                log_tail = (EV / "exyonq.log").read_text(errors="replace")[-2000:]
            except OSError:
                pass
            record("listen", "FAIL", log_tail=log_tail)
            return finish(1)
        record("listen", "PASS", port=port)

        base = f"http://127.0.0.1:{port}"

        def scrape_complete() -> int:
            rc, _, body, _ = curl(f"{base}/metrics")
            if rc != 0:
                return -1
            return metric(body.decode("utf-8", errors="replace"), "exyonq_epoll_sendfile_complete_total")

        before = scrape_complete()
        if before < 0:
            record("metrics_baseline", "FAIL")
            return finish(1)
        record("metrics_baseline", "PASS", complete=before)

        # 1 eligible full GET → sendfile
        rc, st, body, _ = curl(f"{base}/medium.bin")
        after = scrape_complete()
        ok = rc == 0 and "200" in st and body == files["medium.bin"] and after == before + 1
        record(
            "eligible_get_sendfile",
            "PASS" if ok else "FAIL",
            http_status=st,
            body_sha=sha256(body),
            complete_before=before,
            complete_after=after,
            SENDFILE_FAST_PATH_EXECUTED="YES" if after == before + 1 else "NO",
        )
        before = after

        # 2 keepalive repeated
        for i in range(3):
            rc, st, body, _ = curl(f"{base}/medium.bin")
            if rc != 0 or "200" not in st or body != files["medium.bin"]:
                record("keepalive", "FAIL", i=i, http_status=st)
                break
        else:
            after = scrape_complete()
            record(
                "keepalive",
                "PASS" if after >= before + 3 else "FAIL",
                complete_delta=after - before,
            )
            before = after

        # 3 HEAD (no body; Content-Length still declares resource size)
        before = scrape_complete()
        rc, st, body, hdr = curl(f"{base}/medium.bin", method="HEAD")
        after = scrape_complete()
        head_ok = (
            rc == 0
            and "200" in st
            and body == b""
            and "Content-Length: 65536" in hdr
            and after >= before + 1
        )
        record(
            "head",
            "PASS" if head_ok else "FAIL",
            http_status=st,
            body_len=len(body),
            curl_rc=rc,
            complete_before=before,
            complete_after=after,
            SENDFILE_FAST_PATH_EXECUTED="YES" if after >= before + 1 else "NO",
        )
        before = after

        # 4 range 206
        rc, st, body, hdr = curl(f"{base}/medium.bin", headers=["Range: bytes=0-15"])
        after = scrape_complete()
        record(
            "range_206",
            "PASS"
            if rc == 0 and "206" in st and body == files["medium.bin"][:16] and after == before + 1
            else "FAIL",
            http_status=st,
            body_len=len(body),
        )
        before = after

        # 5 unsatisfiable 416
        rc, st, body, _ = curl(f"{base}/medium.bin", headers=["Range: bytes=999999999-"])
        after = scrape_complete()
        record(
            "range_416",
            "PASS" if rc == 0 and "416" in st and after == before + 1 else "FAIL",
            http_status=st,
        )
        before = after

        # 6 conditional 304
        rc, st, body, hdr = curl(f"{base}/medium.bin")
        etag = ""
        for line in hdr.splitlines():
            if line.lower().startswith("etag:"):
                etag = line.split(":", 1)[1].strip()
        before = scrape_complete()
        rc, st, body, _ = curl(f"{base}/medium.bin", headers=[f"If-None-Match: {etag}"])
        after = scrape_complete()
        record(
            "conditional_304",
            "PASS" if rc == 0 and "304" in st and body == b"" and after == before + 1 else "FAIL",
            http_status=st,
            etag=etag,
        )
        before = after

        # Files below 64 KiB are written inline. Sendfile completion moves only
        # at or above that size. Both must return the file bytes.
        sendfile_min = 64 * 1024
        for name in ("empty.bin", "small.bin", "medium.bin", "large.bin"):
            rc, st, body, _ = curl(f"{base}/{name}")
            after = scrape_complete()
            body_ok = rc == 0 and "200" in st and body == files[name]
            sent = after == before + 1
            expect_sendfile = len(files[name]) >= sendfile_min
            ok = body_ok and sent == expect_sendfile
            record(
                f"size_{name}",
                "PASS" if ok else "FAIL",
                http_status=st,
                expect_len=len(files[name]),
                got_len=len(body),
                SENDFILE_FAST_PATH_EXECUTED="YES" if sent else "NO",
            )
            before = after

        # 15 kill-switch → fallback (no complete delta required; body still correct)
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        log.close()

        port2 = pick_port()
        cfg2 = EV / "exyonq-kill.toml"
        ctl2 = EV / "control-kill.sock"
        write_config(cfg2, port2, root, ctl2)
        log2 = open(EV / "exyonq-kill.log", "w", encoding="utf-8")
        env2 = os.environ.copy()
        env2["EXYONQ_EPOLL_SENDFILE"] = "0"
        proc2 = subprocess.Popen(
            [str(BIN), "serve", "--config", str(cfg2)],
            stdout=log2,
            stderr=subprocess.STDOUT,
            env=env2,
            cwd=str(WS),
        )
        try:
            if not wait_port(port2):
                record("kill_switch_listen", "FAIL")
                return finish(1)
            base2 = f"http://127.0.0.1:{port2}"
            rc, _, body, _ = curl(f"{base2}/metrics")
            b0 = metric(body.decode("utf-8", errors="replace"), "exyonq_epoll_sendfile_complete_total")
            rc, st, body, _ = curl(f"{base2}/medium.bin")
            rc2, _, mbody, _ = curl(f"{base2}/metrics")
            b1 = metric(mbody.decode("utf-8", errors="replace"), "exyonq_epoll_sendfile_complete_total")
            ok = (
                rc == 0
                and "200" in st
                and body == files["medium.bin"]
                and b0 >= 0
                and b1 == b0
            )
            record(
                "kill_switch_fallback",
                "PASS" if ok else "FAIL",
                http_status=st,
                complete_before=b0,
                complete_after=b1,
                NORMAL_STATIC_FALLBACK_EXECUTED="YES" if b1 == b0 and body == files["medium.bin"] else "NO",
            )
        finally:
            proc2.send_signal(signal.SIGTERM)
            try:
                proc2.wait(timeout=15)
            except subprocess.TimeoutExpired:
                proc2.kill()
                proc2.wait(timeout=5)
            log2.close()

        # Integrity: no filename specialization in product eligibility source
        src = (WS / "crates/exyonq-mod-static/src/wire_eligibility.rs").read_text(encoding="utf-8")
        product = src.split("#[cfg(test)]", 1)[0]
        code_only = "\n".join(
            line for line in product.splitlines() if not line.lstrip().startswith("//")
        )
        banned = ["64k.bin", "1m.bin", "1k.bin", "routeNNN", "is_bench"]
        hits = [b for b in banned if b in code_only]
        record(
            "zero_filename_specialization",
            "PASS" if not hits else "FAIL",
            hits=hits,
        )

        fails = sum(1 for r in RESULTS if r["status"] == "FAIL")
        return finish(1 if fails else 0)
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()


def finish(code: int) -> int:
    overall = "PASS" if code == 0 else "FAIL"
    payload = {
        "CAPABILITY_ID": "067",
        "FEATURE_ID": "linux-epoll-sendfile",
        "ARCH": ARCH,
        "HOST": HOST,
        "HEAD": HEAD,
        "OVERALL": overall,
        "STARTED_UTC": datetime.now(timezone.utc).isoformat(),
        "results": RESULTS,
        "SENDFILE_TARGET_PATH_EXECUTED": any(
            r.get("SENDFILE_FAST_PATH_EXECUTED") == "YES" for r in RESULTS
        ),
        "BENCHMARK_FILENAME_SPECIALIZATION": 0
        if all(r["status"] == "PASS" for r in RESULTS if r["scenario"] == "zero_filename_specialization")
        else 1,
    }
    OUT.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"OVERALL": overall, "OUT": str(OUT)}))
    return code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as e:
        record("uncaught", "FAIL", error=str(e))
        OUT.write_text(json.dumps({"OVERALL": "FAIL", "results": RESULTS, "error": str(e)}, indent=2) + "\n")
        raise
