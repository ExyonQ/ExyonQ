#!/usr/bin/env python3
"""CAPABILITY_024 = upstream-active-health — real product E2E.

Proves:
  - dual-peer optimistic HEALTHY → both selectable
  - stop Peer A → real probe failures → A UNHEALTHY → traffic only on B
  - restart A → recovery threshold → A selectable again
  - /health returns 503 (process up) → UNHEALTHY; restore → recover
  - hung /health → real probe timeout → UNHEALTHY → recover
  - concurrent client traffic during transitions
  - reload changes health path/thresholds; invalid reload keeps last (Cap013)
  - all peers unhealthy → 503 (not silent unhealthy select)

ZERO_FAKE: real exyonq binary, real HTTP peers, real network/protocol probes.
No synthetic health-state mutation.
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
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

MARKER = b"cap024-proxy-get-v1"


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


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
        time.sleep(0.05)
    return False


def curl_req(url: str, *, timeout: int = 20) -> tuple[int, bytes]:
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


class PeerHandler(BaseHTTPRequestHandler):
    server_version = "Cap024Upstream/1.0"
    get_count = 0
    health_count = 0
    lock = threading.Lock()
    identity = b"peer"
    health_status = 200
    health_hang_s = 0.0

    def log_message(self, fmt: str, *args) -> None:  # noqa: A003
        return

    def do_GET(self) -> None:  # noqa: N802
        path = self.path.split("?", 1)[0]
        if path == "/health":
            with self.lock:
                type(self).health_count += 1
                hang = float(self.health_hang_s)
                code = int(self.health_status)
            if hang > 0:
                time.sleep(hang)
            body = b"ok" if code < 400 else b"fail"
            self.send_response(code)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        # App traffic only under /api/; other probe paths (e.g. /healthz) must not 200.
        if not path.startswith("/api/"):
            body = b"not found"
            self.send_response(404)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        with self.lock:
            type(self).get_count += 1
        body = MARKER + b"|" + self.identity + b"|" + path.encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def make_handler(identity: bytes):
    class H(PeerHandler):
        pass

    H.get_count = 0
    H.health_count = 0
    H.lock = threading.Lock()
    H.identity = identity
    H.health_status = 200
    H.health_hang_s = 0.0
    return H


def start_peer(port: int, identity: bytes):
    handler = make_handler(identity)
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


def health_inline(
    *,
    enabled: bool = True,
    interval_ms: int = 200,
    timeout_ms: int = 100,
    path: str = "/health",
    healthy_threshold: int = 2,
    unhealthy_threshold: int = 2,
) -> str:
    """Inline table on [[upstream]] so it is not attached to endpoints (TOML)."""
    if not enabled:
        return ""
    return (
        f'health_check = {{ enabled = true, interval_ms = {interval_ms}, '
        f'timeout_ms = {timeout_ms}, path = "{path}", '
        f"healthy_threshold = {healthy_threshold}, "
        f"unhealthy_threshold = {unhealthy_threshold} }}"
    )


def write_dual_config(
    path: Path,
    listen: int,
    port_a: int,
    port_b: int,
    *,
    health: str,
    timeout_ms: int = 5000,
) -> None:
    health_line = f"{health}\n" if health else ""
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
max_connect_retries = 1
{health_line}[[upstream.endpoints]]
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
    health: str,
    timeout_ms: int = 3000,
) -> None:
    health_line = f"{health}\n" if health else ""
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
max_connect_retries = 1
{health_line}"""
    )


def ctl_reload(sock: Path, cfg: Path) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "reload", "--config", str(cfg), "--socket", str(sock)],
        capture_output=True,
        text=True,
        env=env,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def wait_until(pred, timeout: float, interval: float = 0.1) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if pred():
            return True
        time.sleep(interval)
    return False


def sample_peers(base: str, n: int = 12) -> dict[str, int]:
    counts = {"A": 0, "B": 0, "other": 0, "err": 0}
    for _ in range(n):
        c, b = curl_req(f"{base}/api/work")
        if c != 200 or MARKER not in b:
            counts["err"] += 1
            continue
        if b"|A|" in b:
            counts["A"] += 1
        elif b"|B|" in b:
            counts["B"] += 1
        else:
            counts["other"] += 1
    return counts


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "upstream-active-health",
        "CAPABILITY": "CAPABILITY_024",
        "CAPABILITY_NAME": "upstream-active-health",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Active HTTP health probes drive peer selection",
        "SUPPORTED_BEHAVIOR": [
            "opt-in health_check on upstream",
            "real GET path on peer authority",
            "2xx healthy; non-2xx / timeout / connect fail = failed observation",
            "consecutive thresholds; optimistic initial HEALTHY",
            "UNHEALTHY excluded from new selection; recover after healthy_threshold",
            "all unhealthy → 503",
        ],
        "EXPLICIT_NON_SCOPE": [
            "TCP-only health mode",
            "TLS upstream health",
            "passive-only health from user traffic",
            "zstd / Cap025",
            "Cap021 policy changes",
        ],
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
        "CAPABILITY_025_STARTED": "NO",
        "ACTIVE_HEALTH_DEFAULT": "DISABLED",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonq binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    tmp = Path(tempfile.mkdtemp(prefix="cap024-health-", dir=str(EV)))
    checks: dict = {}
    procs: list[subprocess.Popen | None] = []
    httpds: list = []

    try:
        hc = health_inline(
            enabled=True,
            interval_ms=200,
            timeout_ms=100,
            path="/health",
            healthy_threshold=2,
            unhealthy_threshold=2,
        )

        # --- Dual peer happy + fail A + recover A ---
        port_a = pick_port()
        port_b = pick_port()
        httpd_a, h_a = start_peer(port_a, b"A")
        httpd_b, h_b = start_peer(port_b, b"B")
        httpds.extend([httpd_a, httpd_b])
        assert wait_listen(port_a) and wait_listen(port_b)
        listen = pick_port()
        cfg = tmp / "dual.toml"
        write_dual_config(cfg, listen, port_a, port_b, health=hc)
        log = EV / "exyonq-cap024-dual.log"
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"

        # Allow probes to warm (optimistic start already selectable).
        time.sleep(0.5)
        both = sample_peers(base, 16)
        checks["dual_both_selectable"] = {
            "ok": both["A"] > 0 and both["B"] > 0 and both["err"] == 0,
            "counts": both,
            "health_a": h_a.health_count,
            "health_b": h_b.health_count,
        }

        # Stop A — real process down.
        before_b = h_b.get_count
        stop_httpd(httpd_a)
        httpds[0] = None
        httpd_a = None

        def a_gone() -> bool:
            c = sample_peers(base, 10)
            return c["A"] == 0 and c["B"] > 0 and c["err"] == 0

        ok_fail = wait_until(a_gone, timeout=8.0)
        after = sample_peers(base, 12)
        checks["peer_a_down_excluded"] = {
            "ok": ok_fail and after["A"] == 0 and after["B"] > 0 and after["err"] == 0,
            "counts": after,
            "b_gets": h_b.get_count - before_b,
        }

        # Restart A — real recovery.
        port_a2 = port_a  # same port so peer key matches config
        httpd_a, h_a = start_peer(port_a2, b"A")
        httpds[0] = httpd_a
        assert wait_listen(port_a2)

        def a_back() -> bool:
            c = sample_peers(base, 12)
            return c["A"] > 0 and c["B"] > 0 and c["err"] == 0

        ok_rec = wait_until(a_back, timeout=10.0)
        recovered = sample_peers(base, 16)
        checks["peer_a_recovered"] = {
            "ok": ok_rec and recovered["A"] > 0 and recovered["B"] > 0,
            "counts": recovered,
        }

        # Concurrent traffic during health transitions (503 on A health).
        h_a.health_status = 503
        # Wait until exclusion first — then prove concurrent traffic never hits A.
        ok_excl = wait_until(lambda: sample_peers(base, 8)["A"] == 0, timeout=8.0)
        errs = 0
        seen_a = 0
        seen_b = 0

        def hammer() -> tuple[int, int, int]:
            local_err = 0
            local_a = 0
            local_b = 0
            for _ in range(20):
                c, b = curl_req(f"{base}/api/conc")
                if c == 200 and b"|A|" in b:
                    local_a += 1
                elif c == 200 and b"|B|" in b:
                    local_b += 1
                elif c != 200:
                    local_err += 1
            return local_err, local_a, local_b

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            futs = [ex.submit(hammer) for _ in range(4)]
            for f in concurrent.futures.as_completed(futs):
                e, a, b = f.result()
                errs += e
                seen_a += a
                seen_b += b
        h_a.health_status = 200
        wait_until(a_back, timeout=10.0)
        checks["concurrency_during_transition"] = {
            "ok": ok_excl and errs == 0 and seen_a == 0 and seen_b > 0 and p.poll() is None,
            "excluded_before_hammer": ok_excl,
            "errs": errs,
            "seen_a": seen_a,
            "seen_b": seen_b,
            "alive": p.poll() is None,
        }

        stop_proc(p)
        procs.pop()
        stop_httpd(httpd_a)
        stop_httpd(httpd_b)
        httpds.clear()

        # --- Health status 503 (process alive) ---
        port_u = pick_port()
        httpd_u, h_u = start_peer(port_u, b"S")
        httpds.append(httpd_u)
        assert wait_listen(port_u)
        listen2 = pick_port()
        cfg2 = tmp / "status.toml"
        write_single_config(cfg2, listen2, port_u, health=hc)
        log2 = EV / "exyonq-cap024-status.log"
        env2 = os.environ.copy()
        env2["EXYONQ_CONFIG"] = str(cfg2)
        p2 = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg2)],
            stdout=log2.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env2,
        )
        procs.append(p2)
        assert wait_listen(listen2)
        base2 = f"http://127.0.0.1:{listen2}"
        time.sleep(0.4)
        c, b = curl_req(f"{base2}/api/x")
        checks["status_baseline"] = {"ok": c == 200 and MARKER in b, "code": c}
        h_u.health_status = 503

        def becomes_503() -> bool:
            code, _ = curl_req(f"{base2}/api/x")
            return code == 503

        ok503 = wait_until(becomes_503, timeout=8.0)
        code_bad, _ = curl_req(f"{base2}/api/x")
        h_u.health_status = 200

        def back_200() -> bool:
            code, body = curl_req(f"{base2}/api/x")
            return code == 200 and MARKER in body

        ok_back = wait_until(back_200, timeout=10.0)
        checks["health_http_503_marks_unhealthy"] = {
            "ok": ok503 and code_bad == 503 and ok_back,
            "code_while_bad": code_bad,
            "health_probes": h_u.health_count,
        }

        # --- Timeout hang on /health ---
        h_u.health_hang_s = 1.0  # timeout_ms=100
        h_u.health_status = 200

        def timeout_unhealthy() -> bool:
            code, _ = curl_req(f"{base2}/api/y")
            return code == 503

        ok_to = wait_until(timeout_unhealthy, timeout=10.0)
        code_to, _ = curl_req(f"{base2}/api/y")
        h_u.health_hang_s = 0.0
        ok_to_back = wait_until(back_200, timeout=10.0)
        checks["health_timeout_marks_unhealthy"] = {
            "ok": ok_to and code_to == 503 and ok_to_back,
            "code_while_hung": code_to,
        }

        stop_proc(p2)
        procs.pop()
        stop_httpd(httpd_u)
        httpds.clear()

        # --- Reload health path + invalid reload (Cap013) ---
        if not CTL.is_file():
            checks["reload"] = {
                "ok": False,
                "detail": "missing exyonqctl",
            }
        else:
            port_r = pick_port()
            httpd_r, h_r = start_peer(port_r, b"R")
            httpds.append(httpd_r)
            assert wait_listen(port_r)
            listen_r = pick_port()
            cfg_r = tmp / "reload.toml"
            # Unix sockaddr path must stay under SUN_LEN (~104); keep short under /tmp.
            ctrl = Path(f"/tmp/exq24-{os.getpid()}.sock")
            if ctrl.exists() or ctrl.is_symlink():
                ctrl.unlink()
            write_single_config(
                cfg_r,
                listen_r,
                port_r,
                health=health_inline(path="/health", interval_ms=200, timeout_ms=100),
            )
            env_r = os.environ.copy()
            env_r["EXYONQ_CONFIG"] = str(cfg_r)
            env_r["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
            log_r = EV / "exyonq-cap024-reload.log"
            pr = subprocess.Popen(
                [str(BINARY), "serve", "--config", str(cfg_r)],
                stdout=log_r.open("w"),
                stderr=subprocess.STDOUT,
                cwd=str(WS),
                env=env_r,
            )
            procs.append(pr)
            assert wait_listen(listen_r), f"listen {listen_r}"
            assert wait_sock(ctrl), f"control sock missing {ctrl}"
            base_r = f"http://127.0.0.1:{listen_r}"
            time.sleep(0.4)
            c0, _ = curl_req(f"{base_r}/api/r")
            # Switch path to /healthz (peer only has /health) → probes fail → 503
            write_single_config(
                cfg_r,
                listen_r,
                port_r,
                health=health_inline(path="/healthz", interval_ms=200, timeout_ms=100),
            )
            rc_ok, out_ok = ctl_reload(ctrl, cfg_r)
            ok_path = wait_until(
                lambda: curl_req(f"{base_r}/api/r")[0] == 503, timeout=10.0
            )
            # Invalid: timeout > interval
            write_single_config(
                cfg_r,
                listen_r,
                port_r,
                health=health_inline(
                    path="/health",
                    interval_ms=100,
                    timeout_ms=200,
                ),
            )
            rc_bad, out_bad = ctl_reload(ctrl, cfg_r)
            # Still 503 from prior /healthz generation (invalid rejected)
            c_keep, _ = curl_req(f"{base_r}/api/r")
            # Restore valid /health
            write_single_config(
                cfg_r,
                listen_r,
                port_r,
                health=health_inline(path="/health", interval_ms=200, timeout_ms=100),
            )
            rc_fix, _ = ctl_reload(ctrl, cfg_r)
            ok_fix = wait_until(
                lambda: curl_req(f"{base_r}/api/r")[0] == 200, timeout=10.0
            )
            checks["reload"] = {
                "ok": (
                    c0 == 200
                    and rc_ok == 0
                    and ok_path
                    and rc_bad != 0
                    and c_keep == 503
                    and rc_fix == 0
                    and ok_fix
                    and pr.poll() is None
                ),
                "baseline": c0,
                "reload_path_rc": rc_ok,
                "path_effect": ok_path,
                "invalid_rc": rc_bad,
                "invalid_out": out_bad[-400:],
                "kept_after_invalid": c_keep,
                "restore_rc": rc_fix,
                "restored": ok_fix,
                "same_pid": pr.poll() is None,
            }
            stop_proc(pr)
            procs.pop()
            stop_httpd(httpd_r)
            httpds.clear()

        # --- Default disabled: no probes when health_check omitted ---
        port_d = pick_port()
        httpd_d, h_d = start_peer(port_d, b"D")
        httpds.append(httpd_d)
        assert wait_listen(port_d)
        listen_d = pick_port()
        cfg_d = tmp / "default.toml"
        write_single_config(cfg_d, listen_d, port_d, health="")
        env_d = os.environ.copy()
        env_d["EXYONQ_CONFIG"] = str(cfg_d)
        pd = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_d)],
            stdout=(EV / "exyonq-cap024-default.log").open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env_d,
        )
        procs.append(pd)
        assert wait_listen(listen_d)
        time.sleep(1.0)
        c_d, b_d = curl_req(f"http://127.0.0.1:{listen_d}/api/d")
        checks["default_disabled_no_probes"] = {
            "ok": c_d == 200 and MARKER in b_d and h_d.health_count == 0,
            "code": c_d,
            "health_count": h_d.health_count,
        }
        stop_proc(pd)
        procs.pop()
        stop_httpd(httpd_d)
        httpds.clear()

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["EXCEPTION"] = repr(exc)
        checks["exception"] = {"ok": False, "error": repr(exc)}
    finally:
        for proc in procs:
            stop_proc(proc)
        for httpd in httpds:
            stop_httpd(httpd)

    result["CHECKS"] = checks
    all_ok = bool(checks) and all(v.get("ok") for v in checks.values())
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if all_ok else "FAIL"
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "checks": {k: v.get("ok") for k, v in checks.items()}}, indent=2))
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
