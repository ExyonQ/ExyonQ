#!/usr/bin/env python3
"""CAPABILITY_014 = cli-spike — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = cli-spike
  FEATURE_NAME = exyonq spike
  USER_VISIBLE_CONTRACT = Fase 0 reverse-proxy spike CLI
  CONFIG_SURFACE = CLI flags listen/upstream
  IMPLEMENTATION_PATH = cli Commands::Spike; exyonq-mod-proxy/spike.rs
  DUAL_ARCH_RELEVANT = NO (matrix exception; campaign still runs dual-Linux)

Cap014 proves the spike CLI entrypoint (not Cap011 serve):
  - real release binary: exyonq spike --listen --upstream
  - real TCP listen socket owned by that process
  - real external HTTP upstream peer process
  - GET + POST body integrity through the spike
  - upstream unavailable → 502
  - concurrent GETs through the spike
  - Cap014-L3-001: mid-response body collect Err → 502
    (declared Content-Length, real peer process terminated after
     headers+prefix; not connect-unavailable alone)

EXPLICIT_NON_SCOPE:
  Cap011 reverse-proxy (exyonq serve + route.upstream) as Cap014 proof
  Cap012 routing-path-proxy
  Cap013 reload / exyonqctl
  library-only spike_proxy integration test as sole Cap014 proof
  platform-boundary-spike-harness
  competitive RPS
  H2/H3 upstream
  Cap015
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

ID_GET = b"cap014-spike-get-v1"
ID_POST = b"cap014-spike-post-body-v1"


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
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body


class UpstreamHandler(BaseHTTPRequestHandler):
    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        return

    def do_GET(self) -> None:  # noqa: N802
        body = ID_GET + self.path.encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Type", "text/plain")
        self.end_headers()
        self.wfile.write(body)

    def do_POST(self) -> None:  # noqa: N802
        n = int(self.headers.get("Content-Length", "0"))
        payload = self.rfile.read(n) if n else b""
        body = ID_POST + payload
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        self.wfile.write(body)


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if not BINARY.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "cli-spike",
                    "CAPABILITY": "CAPABILITY_014",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BINARY}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    up_port = pick_port()
    listen_port = pick_port()
    upstream = ThreadingHTTPServer(("127.0.0.1", up_port), UpstreamHandler)
    up_thread = threading.Thread(target=upstream.serve_forever, daemon=True)
    up_thread.start()

    log = EV / "exyonq-cap014-spike.log"
    srv = subprocess.Popen(
        [
            str(BINARY),
            "spike",
            "--listen",
            f"127.0.0.1:{listen_port}",
            "--upstream",
            f"http://127.0.0.1:{up_port}",
        ],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
    )
    base = f"http://127.0.0.1:{listen_port}"
    checks: dict = {}
    result: dict = {
        "FEATURE_ID": "cli-spike",
        "CAPABILITY": "CAPABILITY_014",
        "CAPABILITY_NAME": "cli-spike",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Fase 0 reverse-proxy spike CLI",
        "SUPPORTED_BEHAVIOR": (
            "exyonq spike --listen/--upstream → real listen; "
            "GET/POST via real HTTP upstream peer; upstream down → 502; "
            "mid-response body.collect Err → 502 (Cap014-L3-001)"
        ),
        "BODY_COLLECT_PATH_CONDITION": (
            "forward_request materializes when Content-Length is Some and "
            "len <= BENCH_SMALL_UPSTREAM_BODY"
        ),
        "BODY_COLLECT_SIZE_THRESHOLD_IF_ANY": 16384,
        "TEST_BODY_DECLARED_LENGTH": 4096,
        "EXPLICIT_NON_SCOPE": [
            "Cap011 reverse-proxy (exyonq serve + route.upstream) as Cap014 proof",
            "Cap012 routing-path-proxy",
            "Cap013 reload / exyonqctl",
            "library-only spike_proxy integration test as sole Cap014 proof",
            "platform-boundary-spike-harness",
            "competitive RPS",
            "H2/H3 upstream",
            "Cap015",
        ],
        "CONFIG_SURFACE": "CLI flags listen/upstream",
        "ENTRYPOINT": "exyonq spike",
        "TARGET_PATH_EXECUTED": "Commands::Spike → run_spike_proxy",
        "CAP011_NOT_USED_AS_PROOF": "YES",
        "CAP013_NOT_USED_AS_PROOF": "YES",
        "REAL_HTTP_UPSTREAM_PEER": "YES",
        "DUAL_ARCH_MATRIX_RELEVANT": "NO",
        "checks": checks,
    }

    try:
        if not wait_listen(listen_port) or srv.poll() is not None:
            result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
            result["DETAIL"] = "spike listen not ready"
            result["SERVER_LOG_TAIL"] = log.read_text(errors="replace")[-4000:]
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 2

        c_get, b_get = curl_req(f"{base}/health")
        get_ok = c_get == 200 and b_get.startswith(ID_GET) and b"/health" in b_get
        checks["get_forward"] = {"code": c_get, "ok": get_ok}

        post_payload = b"payload-cap014-v1"
        c_post, b_post = curl_req(f"{base}/echo", method="POST", data=post_payload)
        post_ok = c_post == 200 and b_post.startswith(ID_POST) and post_payload in b_post
        checks["post_forward"] = {"code": c_post, "ok": post_ok}

        # Negative: path still goes to upstream (spike has no route table); use marker absence
        # for Cap011 path — Cap014 spike forwards all paths. Outside-product check: wrong binary
        # mode unused. Boundary = response identity is spike-upstream, not Cap011 markers.
        c_root, b_root = curl_req(f"{base}/")
        boundary_ok = c_root == 200 and b_root.startswith(ID_GET) and b"cap011-" not in b_root
        checks["spike_identity_boundary"] = {"code": c_root, "ok": boundary_ok}

        def one(_: int) -> bool:
            code, body = curl_req(f"{base}/health")
            return code == 200 and body.startswith(ID_GET)

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
            conc_ok = all(pool.map(one, range(6)))
        checks["concurrency"] = {"ok": conc_ok, "n": 6}

        # --- Cap014-L3-001: mid-response body collect failure (distinct from connect-down)
        upstream.shutdown()
        upstream.server_close()
        deadline = time.time() + 5.0
        while time.time() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", up_port), timeout=0.2):
                    time.sleep(0.05)
                    continue
            except OSError:
                break

        mid_sync = EV / "cap014-l3-midbody-sync.json"
        mid_meta = EV / "cap014-l3-midbody-meta.json"
        mid_log = EV / "cap014-l3-midbody-upstream.log"
        for p in (mid_sync, mid_meta):
            try:
                p.unlink(missing_ok=True)
            except OSError:
                pass
        mid_up = subprocess.Popen(
            [
                sys.executable,
                str(WS / "scripts/reality/cap-014-l3-midbody-upstream.py"),
                "--listen",
                f"127.0.0.1:{up_port}",
                "--sync-file",
                str(mid_sync),
                "--meta-file",
                str(mid_meta),
            ],
            stdout=mid_log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        midbody_ok = False
        midbody_detail: dict = {}
        try:
            if not wait_listen(up_port, timeout=10.0) or mid_up.poll() is not None:
                midbody_detail = {
                    "ok": False,
                    "DETAIL": "midbody upstream listen failed",
                    "UPSTREAM_LOG_TAIL": mid_log.read_text(errors="replace")[-2000:],
                }
            else:
                # Independent proof: peer is a real listening HTTP process.
                c_ready, b_ready = curl_req(f"http://127.0.0.1:{up_port}/ready", timeout=5)
                peer_ready = c_ready == 200 and b_ready.startswith(b"ready")
                meta = json.loads(mid_meta.read_text()) if mid_meta.is_file() else {}
                midbody_detail["UPSTREAM_IMPLEMENTATION"] = meta.get(
                    "UPSTREAM_IMPLEMENTATION"
                )
                midbody_detail["UPSTREAM_PID"] = mid_up.pid
                midbody_detail["UPSTREAM_LISTEN_ADDRESS"] = f"127.0.0.1:{up_port}"
                midbody_detail["PEER_READY_DIRECT"] = {
                    "code": c_ready,
                    "ok": peer_ready,
                }

                if not peer_ready:
                    midbody_detail["ok"] = False
                    midbody_detail["DETAIL"] = "direct peer /ready failed"
                else:
                    # Client against ExyonQ spike (not against upstream).
                    client_fut = concurrent.futures.ThreadPoolExecutor(max_workers=1)
                    fut = client_fut.submit(
                        curl_req, f"{base}/midbody", timeout=20
                    )
                    # Observable sync: headers+prefix flushed BEFORE termination.
                    sync_deadline = time.time() + 15.0
                    sync = None
                    while time.time() < sync_deadline:
                        if mid_sync.is_file():
                            try:
                                sync = json.loads(mid_sync.read_text())
                                if sync.get("UPSTREAM_RESPONSE_STARTED") == "YES":
                                    break
                            except json.JSONDecodeError:
                                pass
                        if fut.done():
                            break
                        time.sleep(0.02)

                    started = bool(sync) and sync.get("UPSTREAM_RESPONSE_STARTED") == "YES"
                    midbody_detail["UPSTREAM_RESPONSE_STARTED"] = (
                        "YES" if started else "NO"
                    )
                    if sync:
                        midbody_detail["UPSTREAM_RESPONSE_STATUS"] = sync.get(
                            "UPSTREAM_RESPONSE_STATUS"
                        )
                        midbody_detail["UPSTREAM_CONTENT_LENGTH_HEADER"] = sync.get(
                            "UPSTREAM_CONTENT_LENGTH_HEADER"
                        )
                        midbody_detail["UPSTREAM_DECLARED_CONTENT_LENGTH"] = sync.get(
                            "UPSTREAM_DECLARED_CONTENT_LENGTH"
                        )
                        midbody_detail["UPSTREAM_BODY_BYTES_ACTUALLY_DELIVERED"] = sync.get(
                            "UPSTREAM_BODY_BYTES_ACTUALLY_DELIVERED"
                        )

                    if not started:
                        midbody_detail["ok"] = False
                        midbody_detail["DETAIL"] = (
                            "sync missing — would collapse to connect-failure class"
                        )
                        # Cancel in-flight client best-effort
                        try:
                            mid_up.kill()
                        except OSError:
                            pass
                        try:
                            fut.result(timeout=5)
                        except Exception:
                            pass
                        client_fut.shutdown(wait=False)
                    else:
                        declared = int(sync["UPSTREAM_DECLARED_CONTENT_LENGTH"])
                        delivered = int(sync["UPSTREAM_BODY_BYTES_ACTUALLY_DELIVERED"])
                        incomplete = delivered < declared
                        midbody_detail["INCOMPLETE_BODY_PRECONDITION"] = incomplete

                        # Real OS process termination AFTER response started.
                        mid_up.send_signal(signal.SIGKILL)
                        try:
                            mid_up.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            pass
                        midbody_detail["UPSTREAM_REAL_TERMINATION_OCCURRED"] = "YES"
                        midbody_detail["UPSTREAM_CONNECTION_TERMINATED_PREMATURELY"] = (
                            "YES"
                        )

                        c_mid, b_mid = fut.result(timeout=25)
                        client_fut.shutdown(wait=True)

                        false_2xx = 200 <= c_mid < 300
                        empty_success = false_2xx and len(b_mid) == 0
                        status_502 = c_mid == 502
                        # Spike still alive; client talked to ExyonQ.
                        spike_alive = srv.poll() is None
                        log_tail = log.read_text(errors="replace")[-8000:]
                        collect_path = "upstream body collect error" in log_tail

                        midbody_ok = (
                            peer_ready
                            and started
                            and incomplete
                            and status_502
                            and not false_2xx
                            and not empty_success
                            and spike_alive
                            and declared == 4096
                            and declared <= 16384
                        )
                        midbody_detail.update(
                            {
                                "ok": midbody_ok,
                                "CLIENT_CONNECTED_TO_EXYONQ": "YES",
                                "EXYONQ_SPIKE_ENTRYPOINT": "YES",
                                "FINAL_HTTP_STATUS": c_mid,
                                "HTTP_2XX_RECEIVED": "YES" if false_2xx else "NO",
                                "FALSE_EMPTY_SUCCESS_BODY": (
                                    "YES" if empty_success else "NO"
                                ),
                                "EXYONQ_FINAL_STATUS": c_mid,
                                "FALSE_SUCCESS_2XX": "YES" if false_2xx else "NO",
                                "DEFAULT_EMPTY_BODY_SUBSTITUTION": (
                                    "YES" if empty_success else "NO"
                                ),
                                "HYPER_BODY_COLLECTION_FAILURE_PATH_REACHED": (
                                    "YES" if collect_path else "UNKNOWN_LOG_ABSENT"
                                ),
                                "BODY_BYTES_CLIENT": len(b_mid),
                                "CONNECT_FAILURE_502_EVIDENCE_DOES_NOT_PROVE_THIS": "YES",
                            }
                        )
        finally:
            if mid_up.poll() is None:
                mid_up.kill()
                try:
                    mid_up.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
        checks["cap014_l3_001_midbody_collect"] = midbody_detail

        # Failure class already proven historically: connect unavailable → 502
        # Midbody peer is dead after SIGKILL; reuse that as connect-down check.
        c_down, b_down = curl_req(f"{base}/health", timeout=10)
        fail_ok = c_down == 502 and ID_GET not in b_down
        checks["upstream_down_502"] = {"code": c_down, "ok": fail_ok}

        positive = get_ok and post_ok
        negative = boundary_ok  # Cap011 marker leak / wrong product path
        failure = fail_ok and midbody_ok
        boundary = boundary_ok
        conc = conc_ok
        overall = (
            positive
            and negative
            and failure
            and boundary
            and conc
            and midbody_ok
            and (srv.poll() is None)
        )

        result["CAP014_POSITIVE_STATUS"] = "PASS" if positive else "FAIL"
        result["CAP014_NEGATIVE_STATUS"] = "PASS" if negative else "FAIL"
        result["CAP014_FAILURE_STATUS"] = "PASS" if failure else "FAIL"
        result["CAP014_BOUNDARY_STATUS"] = "PASS" if boundary else "FAIL"
        result["CAP014_CONCURRENCY_OR_LIFECYCLE_STATUS"] = "PASS" if conc else "FAIL"
        result["CAP014_L3_001_STATUS"] = (
            "CLOSED_REAL_PRODUCTION_REGRESSION" if midbody_ok else "OPEN"
        )
        result["CAP014_L3_001_REAL_E2E_EXERCISED"] = "YES" if midbody_ok else "NO"
        result["CAP014_CROSS_CAPABILITY_INVARIANTS"] = {
            "ENTRYPOINT_IS_SPIKE_CLI": "YES",
            "CAP011_SERVE_NOT_USED": "YES",
            "CAP013_RELOAD_NOT_USED": "YES",
            "REAL_HTTP_UPSTREAM_PEER": "YES",
            "UPSTREAM_DOWN_STATUS": "502" if fail_ok else "UNEXPECTED",
            "UPSTREAM_FAILURE_FALSE_SUCCESS": "NO" if fail_ok else "YES",
            "MIDBODY_COLLECT_ERR_STATUS": (
                midbody_detail.get("FINAL_HTTP_STATUS") if midbody_ok else "FAIL"
            ),
            "MIDBODY_FALSE_SUCCESS_2XX": midbody_detail.get(
                "FALSE_SUCCESS_2XX", "UNKNOWN"
            ),
            "LIBRARY_ONLY_SPIKE_PROXY_RS_NOT_SOLE_PROOF": "YES",
            "RELOAD_PROCESS_RESTART_USED_AS_RELOAD_PROOF": "NO",
            "REQUIRED_UPSTREAM_BODY_FAILURE_MUST_NOT_BECOME_SUCCESS": (
                "YES" if midbody_ok else "NO"
            ),
        }
        result["PRODUCT_DEFECT"] = "YES" if not overall else "NO"
        result["HARNESS_DEFECT"] = "NO"
        result["FINAL_RESULT"] = "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        if srv.poll() is None:
            srv.send_signal(signal.SIGTERM)
            try:
                srv.wait(timeout=10)
            except subprocess.TimeoutExpired:
                srv.kill()
                srv.wait(timeout=5)
        try:
            upstream.server_close()
        except Exception:
            pass


if __name__ == "__main__":
    sys.exit(main())
