#!/usr/bin/env python3
"""CAPABILITY_012 = routing-path-proxy — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = routing-path-proxy
  USER_VISIBLE_CONTRACT = match.path selects proxy route
  CONFIG_SURFACE = route.match.path
  PRIMARY_REALITY_STATUS_BEFORE = SMOKE_ONLY

Cap011 reverse-proxy = CLOSED_VERIFIED_REAL_PRODUCTION (necessary, not sufficient).

Cap012 proves additional semantics:
  - multiple proxy routes with distinct match.path
  - longest-prefix path selection to the correct [[upstream]]
  - wrong-route marker must not leak
  - per-route upstream unavailable → 502 on that path only

EXPLICIT_NON_SCOPE:
  Cap011 alone as Cap012 proof
  Cap005 static path routing as Cap012 proof
  Cap013 reload
  NS5/historical non-evidence
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

ID_API = b"cap012-up-api-v1"
ID_SVC = b"cap012-up-svc-v1"
ID_NEST = b"cap012-up-nest-v1"


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


def curl_req(url: str, *, timeout: int = 15) -> tuple[int, bytes]:
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
        url,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
    except OSError:
        pass
    code_s = (proc.stdout or "").strip()
    code = int(code_s) if code_s.isdigit() else -1
    return code, body


def make_handler(identity: bytes):
    class H(BaseHTTPRequestHandler):
        server_version = "Cap012Upstream/1.0"

        def log_message(self, fmt: str, *args) -> None:  # noqa: A003
            return

        def do_GET(self) -> None:  # noqa: N802
            body = identity + b"|" + self.path.encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("X-Upstream-Identity", identity.decode())
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self) -> None:  # noqa: N802
            n = int(self.headers.get("Content-Length") or "0")
            raw = self.rfile.read(n) if n > 0 else b""
            body = b"POST|" + identity + b"|" + raw
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return H


def start_upstream(port: int, identity: bytes) -> tuple[ThreadingHTTPServer, threading.Thread]:
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_handler(identity))
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


def write_config(
    path: Path,
    listen: int,
    api_port: int,
    svc_port: int,
    nest_port: int,
    *,
    api_timeout_ms: int = 3000,
) -> None:
    # Three proxy routes: /api/, /api/v2/ (longer prefix), /svc/
    path.write_text(
        f"""config_version = 1

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api", "api-v2", "svc"]

[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "up-api"

[[route]]
name = "api-v2"
match = {{ path = "/api/v2/" }}
upstream = "up-nest"

[[route]]
name = "svc"
match = {{ path = "/svc/" }}
upstream = "up-svc"

[[upstream]]
name = "up-api"
target = "http://127.0.0.1:{api_port}"
timeout_ms = {api_timeout_ms}

[[upstream]]
name = "up-nest"
target = "http://127.0.0.1:{nest_port}"
timeout_ms = 3000

[[upstream]]
name = "up-svc"
target = "http://127.0.0.1:{svc_port}"
timeout_ms = 3000
"""
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "routing-path-proxy",
        "CAPABILITY": "CAPABILITY_012",
        "CAPABILITY_NAME": "routing-path-proxy",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "match.path selects proxy route",
        "SUPPORTED_BEHAVIOR": "multi-route match.path longest-prefix → correct [[upstream]]",
        "EXPLICIT_NON_SCOPE": [
            "Cap011 alone as Cap012 proof",
            "Cap005 static path routing as Cap012 proof",
            "Cap013 reload",
            "NS5/historical non-evidence",
            "competitive RPS",
            "H2/H3 upstream",
        ],
        "CAP011_REVERSE_PROXY_BASELINE": "CLOSED_VERIFIED_REAL_PRODUCTION",
        "OPEN_DEFECT_CONTEXT": "RD-001,RD-007",
        "REAL_HTTP_UPSTREAM_PEER": "YES",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_013_STARTED": "NO",
        },
    }

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonq binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap012-rpp-", dir=str(EV)))
    checks: dict = {}
    httpd_api = httpd_svc = httpd_nest = None
    srv = None
    try:
        api_port, svc_port, nest_port = pick_port(), pick_port(), pick_port()
        httpd_api, _ = start_upstream(api_port, ID_API)
        httpd_svc, _ = start_upstream(svc_port, ID_SVC)
        httpd_nest, _ = start_upstream(nest_port, ID_NEST)
        for p in (api_port, svc_port, nest_port):
            if not wait_listen(p, timeout=5):
                raise RuntimeError(f"upstream peer not listening on {p}")

        listen = pick_port()
        cfg = tmp / "routes.toml"
        write_config(cfg, listen, api_port, svc_port, nest_port)
        log = EV / "exyonq-cap012.log"
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(listen):
            raise RuntimeError("exyonq listener not ready")
        base = f"http://127.0.0.1:{listen}"

        c_api, b_api = curl_req(f"{base}/api/health")
        api_ok = c_api == 200 and b_api.startswith(ID_API) and ID_SVC not in b_api and ID_NEST not in b_api
        checks["route_api"] = {"code": c_api, "ok": api_ok}

        c_svc, b_svc = curl_req(f"{base}/svc/status")
        svc_ok = c_svc == 200 and b_svc.startswith(ID_SVC) and ID_API not in b_svc
        checks["route_svc"] = {"code": c_svc, "ok": svc_ok}

        # Longest prefix: nested paths must hit nest, not shorter /api/
        nest_cases = {}
        nest_ok = True
        for path in ("/api/v2/item", "/api/v2/", "/api/v2"):
            code, body = curl_req(f"{base}{path}")
            ok = (
                code == 200
                and body.startswith(ID_NEST)
                and ID_API not in body
                and ID_SVC not in body
            )
            nest_cases[path] = {"code": code, "ok": ok}
            nest_ok = nest_ok and ok
        checks["longest_prefix_api_v2"] = {"ok": nest_ok, "cases": nest_cases}

        # False directory-prefix: /api2 must not select /api/
        c_fp, b_fp = curl_req(f"{base}/api2")
        false_prefix_ok = c_fp != 200 and ID_API not in b_fp and ID_NEST not in b_fp
        checks["false_prefix_api2"] = {"code": c_fp, "ok": false_prefix_ok}

        # POST on selected route
        tag = f"{time.time_ns()}"
        data_file = EV / f"post-{tag}.data"
        data_file.write_bytes(b"body-cap012")
        proc = subprocess.run(
            [
                "curl",
                "-sS",
                "--max-time",
                "10",
                "-o",
                str(EV / f"post-{tag}.body"),
                "-w",
                "%{http_code}",
                "-X",
                "POST",
                "--data-binary",
                f"@{data_file}",
                f"{base}/svc/submit",
            ],
            capture_output=True,
            text=True,
        )
        c_post = int(proc.stdout.strip()) if proc.stdout.strip().isdigit() else -1
        b_post = (EV / f"post-{tag}.body").read_bytes() if (EV / f"post-{tag}.body").is_file() else b""
        post_ok = c_post == 200 and b_post == b"POST|" + ID_SVC + b"|body-cap012"
        checks["post_selected_route"] = {"code": c_post, "ok": post_ok}

        # Negative: outside routes must not return any upstream identity markers
        c_out, b_out = curl_req(f"{base}/other")
        neg_ok = c_out != 200 and ID_API not in b_out and ID_SVC not in b_out and ID_NEST not in b_out
        checks["outside_route"] = {"code": c_out, "ok": neg_ok}

        def one(_i: int) -> bool:
            ca, ba = curl_req(f"{base}/api/health")
            cs, bs = curl_req(f"{base}/svc/status")
            return ca == 200 and ba.startswith(ID_API) and cs == 200 and bs.startswith(ID_SVC)

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            conc = list(ex.map(one, range(6)))
        conc_ok = all(conc)
        checks["concurrency"] = {"ok": conc_ok, "n": len(conc)}

        # Failure: stop api peer → /api/ → 502; /svc/ still works
        stop_upstream(httpd_api)
        httpd_api = None
        time.sleep(0.3)
        c_down, _ = curl_req(f"{base}/api/down", timeout=5)
        c_svc2, b_svc2 = curl_req(f"{base}/svc/still")
        fail_ok = c_down == 502 and c_svc2 == 200 and b_svc2.startswith(ID_SVC)
        checks["per_route_upstream_down"] = {
            "api_code": c_down,
            "svc_code": c_svc2,
            "ok": fail_ok,
        }

        positive = api_ok and svc_ok and nest_ok and post_ok
        boundary_ok = nest_ok and neg_ok and false_prefix_ok
        overall = positive and fail_ok and conc_ok and boundary_ok and (srv.poll() is None)

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP012_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP012_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP012_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP012_BOUNDARY_STATUS": "PASS" if boundary_ok else "FAIL",
                "CAP012_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if conc_ok else "FAIL",
                "CAP012_CROSS_CAPABILITY_INVARIANTS": {
                    "CAP011_REVERSE_PROXY_BASELINE": "CLOSED_VERIFIED_REAL_PRODUCTION",
                    "REAL_HTTP_UPSTREAM_PEER": "YES",
                    "LONGEST_PREFIX_PROXY_ROUTE": "YES" if nest_ok else "NO",
                    "WRONG_UPSTREAM_MARKER_LEAK": "NO" if (api_ok and svc_ok and nest_ok and neg_ok) else "YES",
                    "PER_ROUTE_UPSTREAM_DOWN_ISOLATION": "YES" if fail_ok else "NO",
                    "UPSTREAM_FAILURE_FALSE_SUCCESS": "NO" if fail_ok else "UNKNOWN",
                    "OUTSIDE_PROXY_ROUTE_MARKER_LEAK": "NO" if neg_ok else "YES",
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
        stop_upstream(httpd_api)
        stop_upstream(httpd_svc)
        stop_upstream(httpd_nest)


if __name__ == "__main__":
    sys.exit(main())
