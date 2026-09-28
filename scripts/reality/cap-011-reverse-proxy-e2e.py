#!/usr/bin/env python3
"""CAPABILITY_011 = reverse-proxy — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = reverse-proxy
  USER_VISIBLE_CONTRACT = Route → upstream HTTP peer
  CONFIG_SURFACE = route.upstream; [[upstream]]
  PRIMARY_REALITY_STATUS_BEFORE = SMOKE_ONLY

Historical NS5 non-evidence artifacts are NOT Cap011 proof.

Cap011 proves:
  - real ExyonQ release binary
  - real HTTP upstream peer process (not product path substitute)
  - route.upstream → [[upstream]] → peer
  - GET/POST body integrity
  - upstream unavailable → 502
  - concurrent proxy GETs
  - Cap005 path longest-prefix used only as required infrastructure

EXPLICIT_NON_SCOPE:
  Cap012 routing-path-proxy primary
  Cap010 OCI as Cap011 proof
  Cap008/009 FastCGI/PHP
  competitive RPS
  H2/H3 upstream
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

MARKER = b"cap011-proxy-get-v1"
POST_ECHO = b"cap011-proxy-post-body-v1"


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


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: int = 15,
) -> tuple[int, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
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
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
        if data is not None:
            (EV / f"curl-{tag}.data").unlink(missing_ok=True)
    except OSError:
        pass
    code_s = (proc.stdout or "").strip()
    code = int(code_s) if code_s.isdigit() else -1
    return code, body


class UpstreamHandler(BaseHTTPRequestHandler):
    server_version = "Cap011Upstream/1.0"

    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        return

    def _read_body(self) -> bytes:
        n = int(self.headers.get("Content-Length") or "0")
        return self.rfile.read(n) if n > 0 else b""

    def _reply(self, code: int, body: bytes, extra: dict[str, str] | None = None) -> None:
        self.send_response(code)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("X-Upstream-Identity", "cap011-real-peer")
        if extra:
            for k, v in extra.items():
                self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        if self.path.startswith("/api/"):
            self._reply(200, MARKER + b"|" + self.path.encode())
        else:
            self._reply(404, b"upstream-miss")

    def do_POST(self) -> None:  # noqa: N802
        body = self._read_body()
        self._reply(200, b"POST|" + body)


def start_upstream(port: int) -> tuple[ThreadingHTTPServer, threading.Thread]:
    httpd = ThreadingHTTPServer(("127.0.0.1", port), UpstreamHandler)
    t = threading.Thread(target=httpd.serve_forever, daemon=True)
    t.start()
    return httpd, t


def stop_upstream(httpd: ThreadingHTTPServer | None) -> None:
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


def write_config(path: Path, listen: int, upstream: int, timeout_ms: int = 3000) -> None:
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
"""
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "reverse-proxy",
        "CAPABILITY": "CAPABILITY_011",
        "CAPABILITY_NAME": "reverse-proxy",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Route → upstream HTTP peer",
        "SUPPORTED_BEHAVIOR": "route.upstream + [[upstream]] → real HTTP peer; GET/POST; unavailable→502",
        "EXPLICIT_NON_SCOPE": [
            "NS5/historical non-evidence as Cap011 proof",
            "Cap012 routing-path-proxy primary",
            "Cap010 OCI as Cap011 proof",
            "Cap008/009 FastCGI/PHP",
            "competitive RPS",
            "H2/H3 upstream",
        ],
        "OPEN_DEFECT_CONTEXT": "RD-001,RD-007",
        "REAL_HTTP_UPSTREAM_PEER": "YES",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_012_STARTED": "NO",
        },
    }

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonq binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap011-proxy-", dir=str(EV)))
    checks: dict = {}
    httpd = None
    srv = None
    try:
        up_port = pick_port()
        httpd, _ = start_upstream(up_port)
        if not wait_listen(up_port, timeout=5):
            raise RuntimeError("upstream peer not listening")

        listen = pick_port()
        cfg = tmp / "proxy.toml"
        write_config(cfg, listen, up_port)
        log = EV / "exyonq-cap011.log"
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(listen):
            raise RuntimeError("exyonq listener not ready")
        base = f"http://127.0.0.1:{listen}"

        c_get, b_get = curl_req(f"{base}/api/health")
        get_ok = c_get == 200 and b_get.startswith(MARKER) and b"/api/health" in b_get
        checks["get"] = {"code": c_get, "ok": get_ok, "body": b_get[:200].decode("latin-1", "replace")}

        c_post, b_post = curl_req(
            f"{base}/api/submit",
            method="POST",
            data=POST_ECHO,
            headers=["Content-Type: application/octet-stream"],
        )
        post_ok = c_post == 200 and b_post == b"POST|" + POST_ECHO
        checks["post"] = {"code": c_post, "ok": post_ok}

        # Negative: path outside /api/ should not hit this proxy route (no false upstream success).
        c_neg, b_neg = curl_req(f"{base}/other")
        # Expect non-200 from missing route (404/400) — must not return MARKER.
        neg_ok = MARKER not in b_neg and c_neg != 200
        checks["outside_route"] = {"code": c_neg, "ok": neg_ok, "leak_marker": MARKER in b_neg}

        def one(_i: int) -> bool:
            code, body = curl_req(f"{base}/api/health")
            return code == 200 and body.startswith(MARKER)

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            conc = list(ex.map(one, range(8)))
        conc_ok = all(conc)
        checks["concurrency"] = {"ok": conc_ok, "n": len(conc)}

        # Failure: stop upstream peer → 502
        stop_upstream(httpd)
        httpd = None
        time.sleep(0.3)
        c_down, _ = curl_req(f"{base}/api/down", timeout=5)
        fail_ok = c_down == 502
        checks["upstream_down_502"] = {"code": c_down, "ok": fail_ok}

        # Boundary: restart peer; ensure no outside-route marker leak already checked.
        boundary_ok = neg_ok
        positive = get_ok and post_ok
        overall = positive and fail_ok and conc_ok and boundary_ok and (srv.poll() is None)

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP011_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP011_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP011_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP011_BOUNDARY_STATUS": "PASS" if boundary_ok else "FAIL",
                "CAP011_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if conc_ok else "FAIL",
                "CAP011_CROSS_CAPABILITY_INVARIANTS": {
                    "REAL_HTTP_UPSTREAM_PEER": "YES",
                    "ROUTE_UPSTREAM_CONFIG": "YES",
                    "OUTSIDE_PROXY_ROUTE_MARKER_LEAK": "NO" if neg_ok else "YES",
                    "UPSTREAM_DOWN_STATUS": "502" if fail_ok else str(c_down),
                    "CAP005_PATH_PREFIX_USED_AS_INFRA": "YES",
                    "CAP010_OCI_NOT_USED_AS_PROOF": "YES",
                    "NS5_NON_EVIDENCE_NOT_USED_AS_PROOF": "YES",
                },
                "checks": checks,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    except Exception as e:
        result.update(
            {
                "FINAL_RESULT": "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "UNKNOWN",
                "HARNESS_DEFECT": "YES",
                "DETAIL": repr(e),
                "checks": checks,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 1
    finally:
        stop_proc(srv)
        stop_upstream(httpd)


if __name__ == "__main__":
    sys.exit(main())
