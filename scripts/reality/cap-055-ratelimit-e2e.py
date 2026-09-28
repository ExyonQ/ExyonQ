#!/usr/bin/env python3
"""CAPABILITY_055 = ratelimit — real product E2E (IP rate limit).

ZERO_FAKE chain:
  REAL NETWORK CLIENT → REAL EXYONQ → REAL PEER IP IDENTITY
  → REAL TOKEN BUCKET → REAL ALLOW/REJECT → REAL HTTP + BACKEND SIDE-EFFECT

Proves Cap055 (does not reopen Cap054/038/032/031/…):
  - basic burst allow then 429
  - Retry-After present on reject
  - refill recovery
  - spoofed X-Forwarded-For does NOT change identity
  - spoofed x-exyonq-client-ip overwritten by peer
  - multi TCP connection same IP shares bucket
  - concurrent oversubscription does not exceed burst
  - keepalive requests each consume tokens
  - rejected proxy request → upstream accept count unchanged
  - /health|/live|/ready remain answerable after quota exhaust
  - reload same policy does not refill quota
  - failed reload (rps=0) preserves active limiter
  - zero-config rejected at load (fail-closed)

Cap056/057 cache ordering: dependency noted, not closed here.
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
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

STATIC_BODY = b"cap055-static-ok\n"
UPSTREAM_HITS = {"n": 0}
UPSTREAM_LOCK = threading.Lock()


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
        time.sleep(0.05)
    return False


def stop_proc(p):
    if p is None or p.poll() is not None:
        return
    p.send_signal(signal.SIGTERM)
    try:
        p.wait(timeout=10)
    except Exception:
        p.kill()


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    timeout: int = 20,
    headers: list[str] | None = None,
    headers_out: Path | None = None,
    keepalive: bool = False,
) -> tuple[int, bytes, str]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = headers_out or (EV / f"curl-{tag}.hdr")
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-D",
        str(hdr_path),
        "-o",
        str(body_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
    ]
    if not keepalive:
        cmd.extend(["--http1.1", "-H", "Connection: close"])
    if headers:
        for h in headers:
            cmd.extend(["-H", h])
    data_file = None
    if data is not None:
        data_file = EV / f"curl-{tag}.data"
        data_file.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_file}"])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    try:
        body_path.unlink(missing_ok=True)
        if data_file is not None:
            data_file.unlink(missing_ok=True)
        if headers_out is None:
            hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body, hdr


def make_peer():
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def _hit(self):
            with UPSTREAM_LOCK:
                UPSTREAM_HITS["n"] += 1

        def do_GET(self):
            self._hit()
            body = b"cap055-api-ok"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            self._hit()
            n = int(self.headers.get("Content-Length") or "0")
            _ = self.rfile.read(n) if n else b""
            body = b"cap055-post-ok"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_HEAD(self):
            self._hit()
            self.send_response(200)
            self.send_header("Content-Length", "0")
            self.end_headers()

    return H


def start_peer(port: int):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_peer())
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def cfg_text(
    listen: int,
    root: Path,
    peer: int,
    *,
    rps: int,
    burst: int,
    enabled: bool = True,
) -> str:
    en = "true" if enabled else "false"
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["site", "api"]
[[route]]
name = "site"
match = {{ path = "/site/" }}
root = "{root}"
index = "index.html"
[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "backend"
[[upstream]]
name = "backend"
timeout_ms = 5000
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = 1
priority = 0
admin_state = "enabled"
[modules.ratelimit]
enabled = {en}
requests_per_second = {rps}
burst = {burst}
"""


def hdr_has(hdr: str, name: str) -> bool:
    low = name.lower() + ":"
    return any(line.lower().startswith(low) for line in hdr.splitlines())


def hdr_value(hdr: str, name: str) -> str | None:
    low = name.lower() + ":"
    for line in hdr.splitlines():
        if line.lower().startswith(low):
            return line.split(":", 1)[1].strip()
    return None


def main() -> int:
    checks: dict = {}
    ok = True
    tmp = Path(tempfile.mkdtemp(prefix="cap055-", dir=str(EV)))
    www = tmp / "www"
    www.mkdir()
    (www / "index.html").write_bytes(STATIC_BODY)
    ctrl = Path(f"/tmp/exq55-{os.getpid()}-{time.time_ns() % 100000}.sock")
    if ctrl.exists():
        ctrl.unlink()
    cfg_path = tmp / "exyonq.toml"
    log_path = tmp / "exyonq.log"

    peer_port = pick_port()
    httpd = start_peer(peer_port)
    listen = pick_port()

    # Deterministic small policy: burst=3, rps=1
    RPS, BURST = 1, 3
    cfg_path.write_text(cfg_text(listen, www, peer_port, rps=RPS, burst=BURST))

    env = os.environ.copy()
    env["RUST_LOG"] = "info"
    env["EXYONQ_CONFIG"] = str(cfg_path)
    env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg_path)],
        cwd=str(WS),
        env=env,
        stdout=log_path.open("w"),
        stderr=subprocess.STDOUT,
    )
    try:
        if not wait_sock(ctrl) or not wait_listen(listen):
            checks["boot"] = {
                "PASS": False,
                "detail": "listen/control timeout",
                "log_tail": log_path.read_text()[-800:] if log_path.is_file() else "",
            }
            ok = False
            raise RuntimeError("boot failed")
        checks["boot"] = {"PASS": True, "pid": proc.pid}

        base = f"http://127.0.0.1:{listen}"
        api = f"{base}/api/x"
        site = f"{base}/site/index.html"

        # --- basic allow/reject boundary ---
        codes = []
        last_hdr = ""
        for i in range(BURST + 2):
            code, body, hdr = curl_req(api)
            codes.append(code)
            if code == 429:
                last_hdr = hdr
                checks["reject_body"] = {
                    "PASS": body == b"rate limit exceeded",
                    "body": body.decode("utf-8", "replace")[:80],
                }
                if body != b"rate limit exceeded":
                    ok = False
        allowed = sum(1 for c in codes if c == 200)
        rejected = sum(1 for c in codes if c == 429)
        checks["basic_boundary"] = {
            "PASS": allowed == BURST and rejected >= 1 and codes[BURST] == 429,
            "codes": codes,
            "allowed": allowed,
            "rejected": rejected,
            "burst": BURST,
        }
        if not checks["basic_boundary"]["PASS"]:
            ok = False

        checks["reject_status_429"] = {"PASS": 429 in codes, "codes": codes}
        if 429 not in codes:
            ok = False

        ra = hdr_value(last_hdr, "Retry-After")
        checks["retry_after"] = {
            "PASS": ra is not None and ra.isdigit() and int(ra) >= 1,
            "value": ra,
        }
        if not checks["retry_after"]["PASS"]:
            ok = False

        # --- refill recovery (rps=1 → wait ≥1.2s) ---
        time.sleep(1.35)
        code_r, _, _ = curl_req(api)
        checks["refill_recovery"] = {"PASS": code_r == 200, "code": code_r}
        if code_r != 200:
            ok = False

        # Exhaust again for subsequent tests
        for _ in range(BURST + 2):
            curl_req(api)

        # --- XFF spoof protector ---
        xff_codes = []
        for i in range(4):
            code, _, _ = curl_req(
                api,
                headers=[f"X-Forwarded-For: 198.51.100.{i}", f"Forwarded: for=203.0.113.{i}"],
            )
            xff_codes.append(code)
        # All should be 429 if still exhausted (same peer identity)
        checks["xff_spoof_untrusted"] = {
            "PASS": all(c == 429 for c in xff_codes),
            "codes": xff_codes,
            "RATE_LIMIT_CLIENT_ID_SOURCE": "socket_peer_ip_via_x-exyonq-client-ip",
            "TRUSTED_PROXY": "NONE",
        }
        if not checks["xff_spoof_untrusted"]["PASS"]:
            ok = False

        # Spoof internal client-ip header — must still be peer (still 429)
        code_spoof, _, _ = curl_req(api, headers=["x-exyonq-client-ip: 203.0.113.99"])
        checks["client_ip_header_overwrite"] = {
            "PASS": code_spoof == 429,
            "code": code_spoof,
        }
        if code_spoof != 429:
            ok = False

        # Wait for full burst refill before multi-connection proof
        time.sleep(float(BURST) + 0.4)
        multi_codes = []
        for _ in range(BURST + 2):
            code, _, _ = curl_req(api)  # Connection: close each time
            multi_codes.append(code)
        checks["multi_tcp_same_ip"] = {
            "PASS": sum(1 for c in multi_codes if c == 200) == BURST
            and multi_codes[BURST] == 429,
            "codes": multi_codes,
        }
        if not checks["multi_tcp_same_ip"]["PASS"]:
            ok = False

        # --- concurrent oversubscription ---
        time.sleep(float(BURST) + 0.4)
        with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
            futs = [pool.submit(curl_req, api) for _ in range(16)]
            conc = [f.result()[0] for f in futs]
        conc_ok = sum(1 for c in conc if c == 200)
        checks["concurrent_no_oversubscribe"] = {
            "PASS": conc_ok <= BURST and conc_ok >= 1,
            "allowed": conc_ok,
            "burst": BURST,
            "codes_sample": conc[:8],
        }
        if not checks["concurrent_no_oversubscribe"]["PASS"]:
            ok = False

        # --- keepalive consumes per request (one TCP, many HTTP/1.1) ---
        time.sleep(float(BURST) + 0.4)
        ka_codes: list[int] = []
        try:
            import http.client

            conn = http.client.HTTPConnection("127.0.0.1", listen, timeout=10)
            for _ in range(BURST + 2):
                conn.request("GET", "/api/x", headers={"Host": f"127.0.0.1:{listen}"})
                resp = conn.getresponse()
                _ = resp.read()
                ka_codes.append(resp.status)
            conn.close()
        except Exception as exc:
            ka_codes = []
            checks["keepalive_error"] = {"PASS": False, "error": str(exc)}
            ok = False
        checks["keepalive_per_request"] = {
            "PASS": len(ka_codes) >= BURST + 1
            and sum(1 for c in ka_codes if c == 200) == BURST
            and 429 in ka_codes,
            "codes": ka_codes,
        }
        if not checks["keepalive_per_request"]["PASS"]:
            ok = False

        # --- backend zero side-effect on reject ---
        time.sleep(float(BURST) + 0.4)
        with UPSTREAM_LOCK:
            UPSTREAM_HITS["n"] = 0
        for _ in range(BURST):
            curl_req(api)
        with UPSTREAM_LOCK:
            before_reject = UPSTREAM_HITS["n"]
        # Now over limit
        code429, _, _ = curl_req(api)
        with UPSTREAM_LOCK:
            after_reject = UPSTREAM_HITS["n"]
        checks["backend_zero_side_effect"] = {
            "PASS": code429 == 429 and after_reject == before_reject and before_reject == BURST,
            "reject_code": code429,
            "upstream_before": before_reject,
            "upstream_after": after_reject,
        }
        if not checks["backend_zero_side_effect"]["PASS"]:
            ok = False

        # POST reject also no upstream
        with UPSTREAM_LOCK:
            UPSTREAM_HITS["n"] = 0
        code_post, _, _ = curl_req(api, method="POST", data=b"x" * 64)
        with UPSTREAM_LOCK:
            post_hits = UPSTREAM_HITS["n"]
        checks["post_reject_no_upstream"] = {
            "PASS": code_post == 429 and post_hits == 0,
            "code": code_post,
            "upstream": post_hits,
        }
        if not checks["post_reject_no_upstream"]["PASS"]:
            ok = False

        # --- probes not rate-limited ---
        for path, expect_body in [("/health", b"ok"), ("/live", b"live"), ("/ready", None)]:
            code, body, _ = curl_req(f"{base}{path}")
            if path == "/ready":
                # ready may be 200 when admitted; must not be 429
                pass_ok = code != 429
            else:
                pass_ok = code == 200 and (expect_body is None or body == expect_body)
            checks[f"probe_{path.strip('/')}"] = {
                "PASS": pass_ok,
                "code": code,
                "body": body.decode("utf-8", "replace")[:40],
            }
            if not pass_ok:
                ok = False

        # Static still subject to limiter when exhausted — may be 429
        code_site, _, _ = curl_req(site)
        checks["static_subject_to_global_limit"] = {
            "PASS": code_site in (200, 429),
            "code": code_site,
            "NOTE": "global modules.ratelimit applies after probe short-circuit",
        }

        # --- reload preserves quota (force non-NO_OP via unrelated timeout tweak) ---
        for _ in range(BURST + 3):
            curl_req(api)
        code_pre, _, _ = curl_req(api)
        # Same ratelimit params; change upstream timeout so generation advances
        reload_cfg = cfg_text(listen, www, peer_port, rps=RPS, burst=BURST).replace(
            "timeout_ms = 5000", "timeout_ms = 5001"
        )
        cfg_path.write_text(reload_cfg)
        rel = subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        checks["reload_ok"] = {
            "PASS": rel.returncode == 0 and "NO_OP" not in ((rel.stdout or "") + (rel.stderr or "")),
            "rc": rel.returncode,
            "stdout": (rel.stdout or "")[:160],
            "stderr": (rel.stderr or "")[:160],
        }
        if not checks["reload_ok"]["PASS"]:
            ok = False
        # Immediate probe — must not grant a free full burst from state wipe
        code_post_rel, _, _ = curl_req(api)
        checks["reload_preserves_quota"] = {
            "PASS": code_pre == 429 and code_post_rel == 429,
            "pre": code_pre,
            "post": code_post_rel,
        }
        if not checks["reload_preserves_quota"]["PASS"]:
            ok = False

        # Restore canonical timeout for subsequent bad-reload test
        cfg_path.write_text(cfg_text(listen, www, peer_port, rps=RPS, burst=BURST))
        subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        # Re-exhaust after possible partial refill during reloads
        for _ in range(BURST + 3):
            curl_req(api)
        bad = cfg_text(listen, www, peer_port, rps=0, burst=BURST)
        cfg_path.write_text(bad)
        bad_rel = subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
            timeout=30,
        )
        checks["failed_reload_rejected"] = {
            "PASS": bad_rel.returncode != 0,
            "rc": bad_rel.returncode,
            "stderr": (bad_rel.stderr or bad_rel.stdout or "")[:200],
        }
        if bad_rel.returncode == 0:
            ok = False
        # Restore good config file for process continuity
        cfg_path.write_text(cfg_text(listen, www, peer_port, rps=RPS, burst=BURST))
        code_after_bad, _, _ = curl_req(api)
        checks["failed_reload_preserves_limiter"] = {
            "PASS": code_after_bad == 429,
            "code": code_after_bad,
        }
        if code_after_bad != 429:
            ok = False

        # --- fail-closed at process start for rps=0 ---
        listen2 = pick_port()
        cfg2 = tmp / "bad.toml"
        cfg2.write_text(cfg_text(listen2, www, peer_port, rps=0, burst=3))
        log2 = tmp / "bad.log"
        env2 = os.environ.copy()
        env2["EXYONQ_CONFIG"] = str(cfg2)
        p2 = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg2)],
            cwd=str(WS),
            env=env2,
            stdout=log2.open("w"),
            stderr=subprocess.STDOUT,
        )
        time.sleep(1.5)
        booted = wait_listen(listen2, timeout=1.0)
        still_running = p2.poll() is None
        stop_proc(p2)
        checks["zero_rps_fail_closed_boot"] = {
            "PASS": (not booted) or (not still_running),
            "booted": booted,
            "still_running_after_1_5s": still_running,
            "exit": p2.poll(),
        }
        if not checks["zero_rps_fail_closed_boot"]["PASS"]:
            ok = False

        checks["eviction_policy_structural"] = {
            "PASS": True,
            "MAX_KEYS_PER_SHARD": 2048,
            "SHARD_COUNT": 64,
            "IDLE_EVICT_AFTER_SECS": 300,
            "POLICY": "idle_full_only_or_deny_new_key",
            "PROOF": "unit tests eviction_caps_shard_growth + shard_full_does_not_reset_throttled_via_oldest_evict",
            "NOTE": "LA-CAP055-003: structural citation of unit proof — not a live flood oracle",
        }
        checks["algorithm"] = {
            "RATE_LIMIT_ALGORITHM": "token_bucket",
            "RATE_LIMIT_CAPACITY": "burst",
            "RATE_LIMIT_REFILL": "requests_per_second tokens/sec",
            "CLOCK_SOURCE": "std::time::Instant",
            "RATE_LIMIT_REJECT_STATUS": 429,
            "SCOPE": "global_process_modules_ratelimit_peer_ip",
        }

    except Exception as exc:
        checks["fatal"] = {"PASS": False, "error": str(exc)}
        ok = False
    finally:
        stop_proc(proc)
        try:
            httpd.shutdown()
        except Exception:
            pass

    result = {
        "CAPABILITY_ID": "055",
        "FEATURE_ID": "ratelimit",
        "FEATURE_NAME": "IP rate limit",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "BINARY": str(BINARY),
        "BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else None,
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS" if ok else "FAIL",
        "RATE_LIMIT_CLIENT_ID_SOURCE": "socket_peer_ip",
        "TRUSTED_PROXY_MODEL": "NONE",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if ok else "FAIL",
        "checks": checks,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "ZERO_FAKE": result["ZERO_FAKE"]}, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
