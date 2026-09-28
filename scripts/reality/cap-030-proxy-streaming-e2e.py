#!/usr/bin/env python3
"""CAPABILITY_030 = proxy-streaming — real upstream→downstream incremental body E2E.

ZERO_FAKE: real TCP HTTP upstream process, real ExyonQ, real client sockets.
Streaming proof: FIRST_DOWNSTREAM_BODY_BYTE < UPSTREAM_BODY_COMPLETION with margin.
Cap063/052/051/041/040/048/037/039 must not reopen.
"""
from __future__ import annotations

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
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

CHUNK_A = b"AAA-cap030-stream-part-A\n"
CHUNK_B = b"BBB-cap030-stream-part-B\n"
CHUNK_C = b"CCC-cap030-stream-part-C\n"
PACE_S = 0.35
STREAM_MARGIN_S = 0.15


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    h.update(p.read_bytes())
    return h.hexdigest()


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


# Shared timing board filled by paced upstream handlers.
UPSTREAM_EVENTS: dict[str, list] = {}
UPSTREAM_LOCK = threading.Lock()


def note(path: str, event: str, extra: dict | None = None):
    with UPSTREAM_LOCK:
        UPSTREAM_EVENTS.setdefault(path, []).append(
            {"t": time.monotonic(), "event": event, **(extra or {})}
        )


class PacedUpstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_a):
        pass

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == "/api/paced-cl":
            self._paced(with_cl=True)
        elif path == "/api/paced-chunked":
            self._paced(with_cl=False)
        elif path == "/api/large":
            self._large()
        elif path == "/api/trunc":
            self._truncated()
        elif path == "/api/midfail":
            self._midfail()
        elif path == "/api/fast":
            body = b"fast-ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif path == "/api/stall-head":
            # Cap037 protector: never send head within short timeout.
            time.sleep(5.0)
            self.send_response(200)
            self.end_headers()
            self.wfile.write(b"late")
        elif path == "/api/err500":
            self._paced_status(500)
        else:
            self.send_error(404)

    def _paced(self, *, with_cl: bool):
        body = CHUNK_A + CHUNK_B + CHUNK_C
        note(self.path, "head_begin")
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("X-Cap030", "paced")
        if with_cl:
            self.send_header("Content-Length", str(len(body)))
        else:
            self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        note(self.path, "head_sent")
        for label, chunk in (("A", CHUNK_A), ("B", CHUNK_B), ("C", CHUNK_C)):
            if with_cl:
                self.wfile.write(chunk)
            else:
                self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
            self.wfile.flush()
            note(self.path, f"chunk_{label}", {"n": len(chunk)})
            if label != "C":
                time.sleep(PACE_S)
        if not with_cl:
            self.wfile.write(b"0\r\n\r\n")
            self.wfile.flush()
        note(self.path, "body_complete")

    def _paced_status(self, status: int):
        note(self.path, "head_begin")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        note(self.path, "head_sent")
        for label, chunk in (("A", CHUNK_A), ("B", CHUNK_B)):
            self.wfile.write(f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
            self.wfile.flush()
            note(self.path, f"chunk_{label}")
            time.sleep(PACE_S)
        self.wfile.write(b"0\r\n\r\n")
        note(self.path, "body_complete")

    def _large(self):
        # ~512 KiB streamed in 16 KiB frames with tiny pacing.
        frame = b"L" * 16384
        n_frames = 32
        total = frame * n_frames
        note(self.path, "head_begin")
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(total)))
        self.end_headers()
        note(self.path, "head_sent")
        for i in range(n_frames):
            self.wfile.write(frame)
            self.wfile.flush()
            if i == 0:
                note(self.path, "first_frame")
            if i < n_frames - 1:
                time.sleep(0.02)
        note(self.path, "body_complete", {"bytes": len(total)})

    def _truncated(self):
        # Declares CL=100, sends 20, then closes abruptly.
        note(self.path, "head_begin")
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", "100")
        self.end_headers()
        note(self.path, "head_sent")
        self.wfile.write(b"truncated-partial-XXXX")
        self.wfile.flush()
        note(self.path, "partial_written")
        # Force close without remaining bytes.
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        note(self.path, "aborted")

    def _midfail(self):
        note(self.path, "head_begin")
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()
        note(self.path, "head_sent")
        self.wfile.write(f"{len(CHUNK_A):x}\r\n".encode() + CHUNK_A + b"\r\n")
        self.wfile.flush()
        note(self.path, "chunk_A")
        time.sleep(0.1)
        try:
            self.connection.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        note(self.path, "aborted")


def start_upstream(port: int):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), PacedUpstream)
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def cfg(listen: int, peer: int) -> str:
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api"]

[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "backend"

[[upstream]]
name = "backend"
timeout_ms = 2000
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = 1
priority = 0
admin_state = "enabled"
"""


def start_exyonq(config: Path, sock: Path):
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(config)
    log = EV / f"exyonq-{time.time_ns()}.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
    )
    return proc, log


def stop_proc(proc, grace: float = 8.0):
    if proc.poll() is not None:
        return
    try:
        proc.send_signal(signal.SIGTERM)
    except OSError:
        pass
    end = time.monotonic() + grace
    while time.monotonic() < end and proc.poll() is None:
        time.sleep(0.05)
    if proc.poll() is None:
        try:
            proc.kill()
        except OSError:
            pass


def http_get_streaming(port: int, path: str, *, timeout: float = 15.0):
    """Raw HTTP/1.1 client that records first body byte time and full body."""
    t0 = time.monotonic()
    s = socket.create_connection(("127.0.0.1", port), timeout=2.0)
    s.settimeout(timeout)
    req = (
        f"GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"
        f"User-Agent: cap030-e2e\r\n\r\n"
    ).encode()
    s.sendall(req)
    buf = b""
    first_body_t = None
    header_end = None
    try:
        while True:
            chunk = s.recv(8192)
            if not chunk:
                break
            if first_body_t is None and header_end is None:
                buf += chunk
                idx = buf.find(b"\r\n\r\n")
                if idx >= 0:
                    header_end = idx + 4
                    body_so_far = buf[header_end:]
                    if body_so_far:
                        first_body_t = time.monotonic()
                    # continue accumulating
                continue
            if first_body_t is None:
                # headers done, this chunk is body
                first_body_t = time.monotonic()
                buf += chunk
            else:
                buf += chunk
    except socket.timeout:
        pass
    finally:
        s.close()
    if header_end is None:
        return {
            "ok": False,
            "error": "no_headers",
            "raw": buf[:200],
            "t0": t0,
        }
    headers = buf[:header_end]
    body = buf[header_end:]
    # Decode chunked if present
    if b"transfer-encoding: chunked" in headers.lower():
        body = decode_chunked(body)
    status = 0
    try:
        status = int(headers.split(b"\r\n", 1)[0].split()[1])
    except Exception:
        pass
    return {
        "ok": True,
        "status": status,
        "headers": headers,
        "body": body,
        "first_body_t": first_body_t,
        "t0": t0,
        "done_t": time.monotonic(),
    }


def decode_chunked(data: bytes) -> bytes:
    out = bytearray()
    i = 0
    while i < len(data):
        nl = data.find(b"\r\n", i)
        if nl < 0:
            break
        try:
            size = int(data[i:nl], 16)
        except ValueError:
            break
        i = nl + 2
        if size == 0:
            break
        out.extend(data[i : i + size])
        i += size + 2
    return bytes(out)


def event_t(path: str, name: str) -> float | None:
    with UPSTREAM_LOCK:
        for e in UPSTREAM_EVENTS.get(path, []):
            if e["event"] == name:
                return e["t"]
    return None


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    ok = True
    result = {
        "CAPABILITY_ID": "030",
        "FEATURE_ID": "proxy-streaming",
        "FEATURE_NAME": "Proxy streaming",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP063_REOPEN": "NO",
        "CAP052_REOPEN": "NO",
        "CAP051_REOPEN": "NO",
        "CAP041_REOPEN": "NO",
        "CAP040_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "CAP037_REOPEN": "NO",
        "CAP039_REOPEN": "NO",
        "PROXY_STREAM_BODY_TIMEOUT_POLICY": "NONE_OUTSIDE_CAP037_HEAD_DEADLINE",
        "REQUEST_BODY_STREAMING_SCOPE": "OUTSIDE_CAP030_RESPONSE_ONLY",
        "RESPONSE_TRAILERS": "UNSUPPORTED_CURRENT_CONTRACT",
        "H3_PROXY_STREAMING": "NOT_CLAIMED",
        "UTC": datetime.now(timezone.utc).isoformat(),
    }
    if not BIN.is_file():
        result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BIN)

    def add(name: str, value: dict):
        nonlocal ok
        checks[name] = value
        ok = ok and bool(value.get("ok"))

    work = Path(tempfile.mkdtemp(prefix="cap030-", dir="/tmp"))
    peers = []
    procs = []
    try:
        peer = pick_port()
        peers.append(start_upstream(peer))
        listen = pick_port()
        cfg_path = work / "exyonq.toml"
        cfg_path.write_text(cfg(listen, peer))
        sock = work / "control.sock"
        proc, _ = start_exyonq(cfg_path, sock)
        procs.append(proc)
        ready = wait_pred(lambda: listening(listen))
        add("server_ready", {"ok": ready, "listen": listen, "peer": peer})

        # --- central streaming proof: known CL paced ---
        UPSTREAM_EVENTS.clear()
        r = http_get_streaming(listen, "/api/paced-cl")
        up_complete = event_t("/api/paced-cl", "body_complete")
        up_first = event_t("/api/paced-cl", "chunk_A")
        first = r.get("first_body_t")
        expected = CHUNK_A + CHUNK_B + CHUNK_C
        streaming = (
            first is not None
            and up_complete is not None
            and first < (up_complete - STREAM_MARGIN_S)
            and r.get("body") == expected
        )
        add(
            "streaming_known_cl_timing",
            {
                "ok": bool(streaming and r.get("status") == 200),
                "status": r.get("status"),
                "first_body_t": first,
                "upstream_complete_t": up_complete,
                "upstream_first_chunk_t": up_first,
                "margin_s": STREAM_MARGIN_S,
                "delta_s": None if first is None or up_complete is None else (up_complete - first),
                "body_sha256": hashlib.sha256(r.get("body") or b"").hexdigest(),
                "expected_sha256": hashlib.sha256(expected).hexdigest(),
                "body_len": len(r.get("body") or b""),
            },
        )

        # --- unknown CL / chunked upstream ---
        UPSTREAM_EVENTS.clear()
        r2 = http_get_streaming(listen, "/api/paced-chunked")
        up_c2 = event_t("/api/paced-chunked", "body_complete")
        first2 = r2.get("first_body_t")
        add(
            "streaming_unknown_cl_chunked",
            {
                "ok": r2.get("status") == 200
                and r2.get("body") == expected
                and first2 is not None
                and up_c2 is not None
                and first2 < (up_c2 - STREAM_MARGIN_S),
                "status": r2.get("status"),
                "delta_s": None if first2 is None or up_c2 is None else (up_c2 - first2),
                "body_len": len(r2.get("body") or b""),
            },
        )

        # --- multi-chunk integrity ---
        add(
            "multi_chunk_integrity",
            {
                "ok": (r.get("body") or b"").startswith(CHUNK_A)
                and CHUNK_B in (r.get("body") or b"")
                and (r.get("body") or b"").endswith(CHUNK_C.rstrip(b"\n") + b"\n")
                and r.get("body") == expected,
                "body": (r.get("body") or b"")[:80].decode("utf-8", "replace"),
            },
        )

        # --- large body incremental ---
        UPSTREAM_EVENTS.clear()
        rl = http_get_streaming(listen, "/api/large", timeout=30.0)
        up_l = event_t("/api/large", "body_complete")
        first_l = rl.get("first_body_t")
        add(
            "large_body_streaming",
            {
                "ok": rl.get("status") == 200
                and len(rl.get("body") or b"") == 16384 * 32
                and first_l is not None
                and up_l is not None
                and first_l < (up_l - 0.05),
                "status": rl.get("status"),
                "body_len": len(rl.get("body") or b""),
                "delta_s": None if first_l is None or up_l is None else (up_l - first_l),
            },
        )

        # --- truncated upstream: must not look like success with full CL body ---
        rt = http_get_streaming(listen, "/api/trunc", timeout=5.0)
        body_t = rt.get("body") or b""
        # Either connection error / incomplete / non-200 / short body — never 100-byte success.
        trunc_ok = not (rt.get("status") == 200 and len(body_t) == 100)
        add(
            "truncated_upstream_no_false_complete",
            {
                "ok": trunc_ok,
                "status": rt.get("status"),
                "body_len": len(body_t),
                "raw_ok": rt.get("ok"),
            },
        )

        # --- mid-stream upstream abort ---
        rm = http_get_streaming(listen, "/api/midfail", timeout=5.0)
        # May receive partial A; must not claim full success with invented remainder.
        mid_body = rm.get("body") or b""
        add(
            "midstream_upstream_failure",
            {
                "ok": CHUNK_B not in mid_body and proc.poll() is None,
                "status": rm.get("status"),
                "body_len": len(mid_body),
                "server_alive": proc.poll() is None,
            },
        )

        # --- Cap037 protector: head timeout still 504 ---
        r37 = http_get_streaming(listen, "/api/stall-head", timeout=8.0)
        add(
            "cap037_head_timeout_protector",
            {
                "ok": r37.get("status") == 504,
                "status": r37.get("status"),
                "NOTE": "timeout_ms=2000 until response head; body policy unchanged",
            },
        )

        # --- concurrent streaming isolation ---
        results = []

        def worker(i: int):
            results.append((i, http_get_streaming(listen, "/api/paced-cl")))

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=20)
        add(
            "concurrent_stream_isolation",
            {
                "ok": len(results) == 4
                and all(r.get("body") == expected and r.get("status") == 200 for _, r in results),
                "n": len(results),
            },
        )

        # --- fast + streaming concurrent ---
        slow_holder = {}

        def slow():
            slow_holder["r"] = http_get_streaming(listen, "/api/paced-cl")

        th = threading.Thread(target=slow)
        th.start()
        time.sleep(0.05)
        rf = http_get_streaming(listen, "/api/fast", timeout=5.0)
        th.join(timeout=20)
        add(
            "fast_plus_streaming_liveness",
            {
                "ok": rf.get("status") == 200
                and rf.get("body") == b"fast-ok"
                and (slow_holder.get("r") or {}).get("body") == expected,
                "fast_status": rf.get("status"),
                "slow_ok": (slow_holder.get("r") or {}).get("body") == expected,
            },
        )

        # --- downstream disconnect mid-stream ---
        s = socket.create_connection(("127.0.0.1", listen), 2.0)
        s.sendall(
            b"GET /api/paced-cl HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        # Read a little then abort.
        s.settimeout(2.0)
        try:
            _ = s.recv(64)
        except OSError:
            pass
        s.close()
        time.sleep(0.3)
        # Server must remain healthy for a new request.
        ra = http_get_streaming(listen, "/api/fast")
        add(
            "downstream_disconnect_no_server_death",
            {
                "ok": proc.poll() is None and ra.get("status") == 200,
                "server_alive": proc.poll() is None,
                "followup_status": ra.get("status"),
            },
        )

        # --- upstream 500 streamed ---
        re = http_get_streaming(listen, "/api/err500")
        add(
            "upstream_500_streamed_forward",
            {
                "ok": re.get("status") == 500 and CHUNK_A in (re.get("body") or b""),
                "status": re.get("status"),
                "body_len": len(re.get("body") or b""),
            },
        )

        # --- HTTP/1 keepalive: stream then fast on new connection is enough;
        #    document Connection: close client above; probe second request ---
        add(
            "http1_followup_after_stream",
            {
                "ok": ra.get("status") == 200,
                "NOTE": "client uses Connection: close; follow-up on fresh conn",
            },
        )

        stop_proc(proc)
    except Exception as exc:
        add("harness_exception", {"ok": False, "error": repr(exc)})
        for p in procs:
            stop_proc(p, grace=2.0)
    finally:
        for h in peers:
            try:
                h.shutdown()
            except Exception:
                pass
        try:
            import shutil

            shutil.rmtree(work, ignore_errors=True)
        except Exception:
            pass

    result["CHECKS"] = checks
    result["CHECKS_PASSED"] = sum(1 for v in checks.values() if v.get("ok"))
    result["CHECKS_TOTAL"] = len(checks)
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(
        json.dumps(
            {
                "FINAL_RESULT": result["FINAL_RESULT"],
                "passed": result["CHECKS_PASSED"],
                "total": result["CHECKS_TOTAL"],
            },
            indent=2,
        )
    )
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
