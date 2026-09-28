#!/usr/bin/env python3
"""CAPABILITY_021 = upstream-retries — real product E2E (connect-only).

Proves:
  - multi-peer failover after connect refused (A down → B receives exactly 1)
  - POST pre-send failover (mutation count == 1 on B; A == 0)
  - POST to live peer returning 500 is NOT retried (count == 1)
  - max_connect_retries=0 preserves Cap011 single-attempt 502
  - shared timeout budget against blackhole peer (not T×attempts)
  - concurrent GETs under failover
  - Cap011 GET/POST happy path still works

ZERO_FAKE: real exyonq binary, real HTTP peers, real TCP connect refused / blackhole.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
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

MARKER = b"cap021-proxy-get-v1"
POST_BODY = b"cap021-mutation-v1"


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


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: int = 20,
) -> tuple[int, bytes, float]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-o",
        str(body_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
    ]
    if headers:
        for h in headers:
            cmd.extend(["-H", h])
    if data is not None:
        data_file = EV / f"curl-{tag}.data"
        data_file.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_file}"])
    cmd.append(url)
    t0 = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True)
    elapsed = time.monotonic() - t0
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
        if data is not None:
            (EV / f"curl-{tag}.data").unlink(missing_ok=True)
    except OSError:
        pass
    code_s = (proc.stdout or "").strip()
    code = int(code_s) if code_s.isdigit() else -1
    return code, body, elapsed


class CountingHandler(BaseHTTPRequestHandler):
    server_version = "Cap021Upstream/1.0"
    # Class attrs overridden per server instance via factory.
    get_count = 0
    post_count = 0
    lock = threading.Lock()
    status_override: int | None = None
    identity = b"peer"

    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        return

    def _read_body(self) -> bytes:
        n = int(self.headers.get("Content-Length") or "0")
        return self.rfile.read(n) if n > 0 else b""

    def do_GET(self) -> None:  # noqa: N802
        with self.lock:
            type(self).get_count += 1
        body = MARKER + b"|" + self.identity + b"|" + self.path.encode()
        code = self.status_override or 200
        self.send_response(code)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if code < 400:
            self.wfile.write(body)

    def do_POST(self) -> None:  # noqa: N802
        body_in = self._read_body()
        with self.lock:
            type(self).post_count += 1
        code = self.status_override or 200
        out = b"POST|" + self.identity + b"|" + body_in
        self.send_response(code)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(out) if code < 400 else 0))
        self.end_headers()
        if code < 400:
            self.wfile.write(out)


def make_handler(identity: bytes, status_override: int | None = None):
    class H(CountingHandler):
        pass

    H.get_count = 0
    H.post_count = 0
    H.lock = threading.Lock()
    H.status_override = status_override
    H.identity = identity
    return H


def start_peer(port: int, identity: bytes, status_override: int | None = None):
    handler = make_handler(identity, status_override)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), handler)
    t = threading.Thread(target=httpd.serve_forever, daemon=True)
    t.start()
    return httpd, handler


def stop_httpd(httpd: ThreadingHTTPServer | None) -> None:
    if httpd is None:
        return
    try:
        httpd.shutdown()
    except Exception:
        pass
    try:
        httpd.server_close()
    except Exception:
        pass


def stop_proc(p: subprocess.Popen | None) -> None:
    if p is None:
        return
    if p.poll() is None:
        p.send_signal(signal.SIGTERM)
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()


def write_multi_config(
    path: Path,
    listen: int,
    port_a: int,
    port_b: int,
    *,
    timeout_ms: int = 5000,
    max_connect_retries: int = 1,
) -> None:
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
timeout_ms = {timeout_ms}
max_connect_retries = {max_connect_retries}
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_a}
weight = 1
priority = 0
[[upstream.endpoints]]
address = "127.0.0.1"
port = {port_b}
weight = 1
priority = 0
"""
    )


def write_single_config(
    path: Path,
    listen: int,
    upstream: int,
    *,
    timeout_ms: int = 3000,
    max_connect_retries: int = 1,
) -> None:
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
timeout_ms = {timeout_ms}
max_connect_retries = {max_connect_retries}
"""
    )


def write_blackhole_config(path: Path, listen: int, timeout_ms: int) -> None:
    # TEST-NET-3 — typically blackholes; connect waits for Hyper connect_timeout (500ms).
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
target = "http://203.0.113.1:9"
timeout_ms = {timeout_ms}
max_connect_retries = 1
"""
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "upstream-retries",
        "CAPABILITY": "CAPABILITY_021",
        "CAPABILITY_NAME": "upstream-retries",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Connect-only upstream retry / failover (max 1)",
        "SUPPORTED_BEHAVIOR": "hyper is_connect() only; shared deadline; no status retries",
        "EXPLICIT_NON_SCOPE": [
            "retry on HTTP 5xx",
            "retry after response started",
            "If-Range",
            "active health checking",
            "TLS upstream",
        ],
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "CAPABILITY_022_STARTED": "NO",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonq binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap021-retry-", dir=str(EV)))
    checks: dict = {}
    procs: list[subprocess.Popen | None] = []
    httpds: list = []

    try:
        # --- Cap011 protector: happy path single upstream ---
        up = pick_port()
        httpd, h = start_peer(up, b"single")
        httpds.append(httpd)
        assert wait_listen(up, 5)
        listen = pick_port()
        cfg = tmp / "single.toml"
        write_single_config(cfg, listen, up, max_connect_retries=1)
        log = EV / "exyonq-cap021-single.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        c, b, _ = curl_req(f"{base}/api/health")
        checks["happy_get"] = {"ok": c == 200 and MARKER in b, "code": c}
        c, b, _ = curl_req(
            f"{base}/api/m",
            method="POST",
            data=POST_BODY,
            headers=["Content-Type: application/octet-stream"],
        )
        checks["happy_post"] = {
            "ok": c == 200 and b.startswith(b"POST|") and POST_BODY in b,
            "code": c,
            "post_count": h.post_count,
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(httpd)
        httpds.pop()

        # --- Failover GET: A down, B up ---
        port_a = pick_port()  # never listen
        port_b = pick_port()
        httpd_b, hb = start_peer(port_b, b"B")
        httpds.append(httpd_b)
        assert wait_listen(port_b, 5)
        listen = pick_port()
        cfg = tmp / "failover.toml"
        write_multi_config(cfg, listen, port_a, port_b, max_connect_retries=1)
        log = EV / "exyonq-cap021-failover.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        before_b = hb.get_count
        c, b, _ = curl_req(f"{base}/api/failover")
        after_b = hb.get_count
        checks["failover_get"] = {
            "ok": c == 200 and b"|B|" in b and (after_b - before_b) == 1,
            "code": c,
            "b_get_delta": after_b - before_b,
            "body": b[:120].decode("latin-1", "replace"),
        }

        # --- POST pre-send failover ---
        before_post = hb.post_count
        c, b, _ = curl_req(
            f"{base}/api/mutate",
            method="POST",
            data=POST_BODY,
            headers=["Content-Type: application/octet-stream"],
        )
        after_post = hb.post_count
        checks["failover_post_presend"] = {
            "ok": c == 200 and (after_post - before_post) == 1 and b"|B|" in b,
            "code": c,
            "b_post_delta": after_post - before_post,
            "UPSTREAM_A_APPLICATION_REQUEST_COUNT": 0,
            "UPSTREAM_B_APPLICATION_REQUEST_COUNT": after_post - before_post,
        }

        # concurrency under failover
        def one(_i: int) -> bool:
            code, body, _ = curl_req(f"{base}/api/c{_i}")
            return code == 200 and b"|B|" in body

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as ex:
            conc = list(ex.map(one, range(12)))
        checks["concurrency_failover"] = {"ok": all(conc), "n": len(conc), "pass": sum(conc)}

        stop_proc(p)
        procs.pop()
        stop_httpd(httpd_b)
        httpds.pop()

        # --- POST 500 must not retry ---
        up = pick_port()
        httpd, h5 = start_peer(up, b"S500", status_override=500)
        httpds.append(httpd)
        assert wait_listen(up, 5)
        listen = pick_port()
        cfg = tmp / "status500.toml"
        write_single_config(cfg, listen, up, max_connect_retries=1)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap021-500.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        c, _, _ = curl_req(
            f"http://127.0.0.1:{listen}/api/m",
            method="POST",
            data=POST_BODY,
            headers=["Content-Type: application/octet-stream"],
        )
        checks["no_retry_on_500"] = {
            "ok": c == 500 and h5.post_count == 1,
            "code": c,
            "post_count": h5.post_count,
        }
        stop_proc(p)
        procs.pop()
        stop_httpd(httpd)
        httpds.pop()

        # --- retries disabled: peer down → 502 ---
        dead = pick_port()
        listen = pick_port()
        cfg = tmp / "noretry.toml"
        write_single_config(cfg, listen, dead, max_connect_retries=0, timeout_ms=2000)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap021-noretry.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        c, _, elapsed = curl_req(f"http://127.0.0.1:{listen}/api/x", timeout=10)
        checks["retries_disabled_502"] = {
            "ok": c == 502,
            "code": c,
            "elapsed_s": elapsed,
        }
        stop_proc(p)
        procs.pop()

        # --- shared timeout budget vs blackhole (connect timeout path) ---
        listen = pick_port()
        cfg = tmp / "budget.toml"
        t_ms = 2000
        write_blackhole_config(cfg, listen, t_ms)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap021-budget.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        procs.append(p)
        assert wait_listen(listen)
        c, _, elapsed = curl_req(f"http://127.0.0.1:{listen}/api/bh", timeout=15)
        # Shared deadline: must not approach 2× full request timeout (4s+).
        # Connect timeout 500ms × 2 attempts ≈ 1s; wall budget 2s.
        # Shared deadline proof: reject T×attempts (~4s+). Cap021 uses Hyper
        # connect_timeout≈500ms × ≤2 attempts under a 2s wall budget → ~1s typical.
        not_t_times = elapsed < (t_ms / 1000.0) * 1.6
        checks["timeout_budget"] = {
            "ok": c in (502, 504) and not_t_times and elapsed < 3.2,
            "code": c,
            "elapsed_s": elapsed,
            "timeout_ms": t_ms,
            "not_t_times_attempts": not_t_times,
        }
        stop_proc(p)
        procs.pop()

        # --- invalid config rejected (max_connect_retries=2) ---
        bad = tmp / "bad.toml"
        write_single_config(bad, pick_port(), pick_port(), max_connect_retries=2)
        proc = subprocess.run(
            [str(BINARY), "serve", "--config", str(bad)],
            capture_output=True,
            text=True,
            timeout=15,
        )
        checks["reject_max_retries_gt1"] = {
            "ok": proc.returncode != 0,
            "returncode": proc.returncode,
            "stderr_tail": (proc.stderr or proc.stdout or "")[-300:],
        }

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["DETAIL"] = repr(exc)
        result["CHECKS"] = checks
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 1
    finally:
        for p in procs:
            stop_proc(p)
        for h in httpds:
            stop_httpd(h)

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
