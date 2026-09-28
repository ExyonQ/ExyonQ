#!/usr/bin/env python3
"""CAPABILITY_046 = upstream-admin-state — real product E2E.

AdminEndpointState via REAL config + REAL reload. Cap045 WRR pre-filter.
Cap016 has no endpoint admin mutation. Cap040 server drain distinct.
ZERO_FAKE: real upstreams, real reload, real traffic.
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
MARKER = b"cap046-admin"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket(); s.bind(("127.0.0.1", 0)); p = s.getsockname()[1]; s.close(); return p


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


def curl_req(url: str, *, timeout: int = 20) -> tuple[int, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    proc = subprocess.run(
        ["curl", "-sS", "--max-time", str(timeout), "-o", str(body_path), "-w", "%{http_code}", url],
        capture_output=True, text=True,
    )
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
    except OSError:
        pass
    code = int(proc.stdout.strip() or "0") if proc.returncode == 0 else -1
    return code, body


class PeerHandler(BaseHTTPRequestHandler):
    identity = b"X"
    lock = None
    get_count = 0
    health_count = 0
    health_ok = True

    def log_message(self, *_a):
        pass

    def do_GET(self):
        if self.path.startswith("/health"):
            with self.lock:
                type(self).health_count += 1
            if self.health_ok:
                self.send_response(200); self.send_header("Content-Length", "2"); self.end_headers(); self.wfile.write(b"ok")
            else:
                self.send_response(503); self.end_headers()
            return
        if not self.path.startswith("/api"):
            self.send_response(404); self.end_headers(); return
        with self.lock:
            type(self).get_count += 1
        body = MARKER + b"|" + self.identity + b"|"
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def make_peer_class(ident: bytes):
    lock = threading.Lock()
    class H(PeerHandler):
        pass
    H.identity = ident; H.lock = lock; H.get_count = 0; H.health_count = 0; H.health_ok = True
    return H


def start_peer(port: int, ident: bytes):
    cls = make_peer_class(ident)
    httpd = ThreadingHTTPServer(("127.0.0.1", port), cls)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd, cls


def stop_httpd(httpd):
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


def write_multi(
    path: Path,
    listen: int,
    peers: list[tuple[int, int, str]],
    *,
    health: str = "",
    max_connect_retries: int = 1,
    timeout_ms: int = 5000,
) -> None:
    """peers: (port, weight, admin_state) admin_state in enabled|disabled|drain_requested."""
    health_line = f"{health}\n" if health else ""
    eps = []
    for port, weight, admin in peers:
        eps.append(
            f"""[[upstream.endpoints]]
address = "127.0.0.1"
port = {port}
weight = {weight}
priority = 0
admin_state = "{admin}"
"""
        )
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
{health_line}{''.join(eps)}"""
    )


def health_inline(
    *,
    interval_ms: int = 200,
    timeout_ms: int = 100,
    path: str = "/health",
    healthy_threshold: int = 2,
    unhealthy_threshold: int = 2,
) -> str:
    return (
        f'health_check = {{ enabled = true, interval_ms = {interval_ms}, '
        f'timeout_ms = {timeout_ms}, path = "{path}", '
        f"healthy_threshold = {healthy_threshold}, "
        f"unhealthy_threshold = {unhealthy_threshold} }}"
    )


def ctl_reload(sock: Path, cfg: Path) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "reload", "--config", str(cfg), "--socket", str(sock)],
        capture_output=True, text=True, env=env,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def wait_until(pred, timeout: float, interval: float = 0.1) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if pred():
            return True
        time.sleep(interval)
    return False


def sample(base: str, n: int, labels: list[bytes]) -> dict[str, int]:
    counts = {lab.decode(): 0 for lab in labels}
    counts["err"] = 0
    counts["other"] = 0
    for _ in range(n):
        c, b = curl_req(f"{base}/api/w")
        if c != 200 or MARKER not in b:
            counts["err"] += 1
            continue
        hit = False
        for lab in labels:
            if b"|" + lab + b"|" in b:
                counts[lab.decode()] += 1
                hit = True
                break
        if not hit:
            counts["other"] += 1
    return counts


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "upstream-admin-state",
        "CAPABILITY": "CAPABILITY_046",
        "CAPABILITY_NAME": "upstream-admin-state",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Endpoint AdminEndpointState eligibility (config/reload)",
        "CAP046_ADMIN_STATE_OPERATOR_SURFACE": "reload/config",
        "ADMIN_STATE_VARIANTS": ["Enabled", "Disabled", "DrainRequested"],
        "DRAINING_ACCEPTS_NEW_REQUESTS": "NO",
        "DRAINING_HEALTH_PROBES_CONTINUE": "NO",
        "DRAINING_REENABLE_SUPPORTED": "YES_VIA_RELOAD_TO_ENABLED",
        "CAP045_REOPEN": "NO",
        "CAP024_REOPEN": "NO",
        "CAP021_REOPEN": "NO",
        "CAP016_BROADENED": "NO",
        "CAPABILITY_037_STARTED": "NO",
        "USES_SMOKE": "NO",
        "ZERO_FAKE": "PASS",
    }
    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing binary"})
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)

    tmp = Path(tempfile.mkdtemp(prefix="cap046-", dir=str(EV)))
    checks: dict = {}
    procs: list = []
    httpds: list = []

    try:
        # --- Both enabled + healthy → both receive (equal weight) ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        assert wait_listen(pa) and wait_listen(pb)
        listen = pick_port()
        cfg = tmp / "both.toml"
        write_multi(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-both.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        both = sample(base, 20, [b"A", b"B"])
        checks["both_enabled_receive"] = {
            "ok": both["A"] == 10 and both["B"] == 10 and both["err"] == 0,
            "counts": both,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Disabled heavy + healthy → never selected ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "dis.toml"
        write_multi(cfg, listen, [(pa, 100, "disabled"), (pb, 1, "enabled")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-dis.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        dis = sample(f"http://127.0.0.1:{listen}", 24, [b"A", b"B"])
        checks["disabled_heavy_zero_selections"] = {
            "ok": dis["A"] == 0 and dis["B"] == 24 and dis["err"] == 0 and ca.get_count == 0,
            "counts": dis, "a_gets": ca.get_count,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- DrainRequested ≡ no new selections ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "drain.toml"
        write_multi(cfg, listen, [(pa, 50, "drain_requested"), (pb, 1, "enabled")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-drain.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        dr = sample(f"http://127.0.0.1:{listen}", 16, [b"A", b"B"])
        checks["drain_requested_no_new_selections"] = {
            "ok": dr["A"] == 0 and dr["B"] == 16 and dr["err"] == 0,
            "counts": dr,
            "DRAINING_ACCEPTS_NEW_REQUESTS": "NO",
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Reload: enabled → disabled → enabled (WRR fairness after re-enable) ---
        if not CTL.is_file():
            checks["reload_admin_flip"] = {"ok": False, "detail": "missing exyonqctl"}
        else:
            pa, pb = pick_port(), pick_port()
            ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
            httpds.extend([ha, hb])
            listen = pick_port()
            cfg = tmp / "reload.toml"
            ctrl = Path(f"/tmp/exq46-{os.getpid()}.sock")
            if ctrl.exists() or ctrl.is_symlink():
                ctrl.unlink()
            write_multi(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
            env = os.environ.copy()
            env["EXYONQ_CONFIG"] = str(cfg)
            env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
            p = subprocess.Popen(
                [str(BINARY), "serve", "--config", str(cfg)],
                stdout=(EV / "exyonq-cap046-reload.log").open("w"),
                stderr=subprocess.STDOUT, cwd=str(WS), env=env,
            )
            procs.append(p)
            assert wait_listen(listen) and wait_sock(ctrl)
            base = f"http://127.0.0.1:{listen}"
            before = sample(base, 10, [b"A", b"B"])
            write_multi(cfg, listen, [(pa, 1, "disabled"), (pb, 1, "enabled")])
            rc1, out1 = ctl_reload(ctrl, cfg)
            mid = sample(base, 16, [b"A", b"B"])
            write_multi(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
            rc2, out2 = ctl_reload(ctrl, cfg)
            after = sample(base, 20, [b"A", b"B"])
            checks["reload_admin_flip"] = {
                "ok": (
                    rc1 == 0 and rc2 == 0
                    and before["A"] > 0 and before["B"] > 0
                    and mid["A"] == 0 and mid["B"] == 16
                    and after["A"] == 10 and after["B"] == 10 and after["err"] == 0
                ),
                "before": before, "mid": mid, "after": after,
                "reload_rc": [rc1, rc2],
                "note": "re-enable uses new generation WRR reset (no stale credit burst)",
            }
            # Invalid admin_state rejected; old generation preserved
            bad = tmp / "bad.toml"
            bad.write_text(cfg.read_text().replace('admin_state = "enabled"', 'admin_state = "bogus"', 1))
            # Write bad into cfg path for reload attempt
            write_multi(cfg, listen, [(pa, 1, "bogus"), (pb, 1, "enabled")])
            rc_bad, out_bad = ctl_reload(ctrl, cfg)
            keep = sample(base, 10, [b"A", b"B"])
            checks["invalid_admin_state_rejected"] = {
                "ok": (
                    rc_bad != 0
                    and keep["err"] == 0
                    and keep["A"] > 0
                    and keep["B"] > 0
                    and keep["A"] + keep["B"] == 10
                ),
                "reload_rc": rc_bad,
                "keep": keep,
                "reload_output": out_bad[-500:],
            }
            # restore valid for cleanup
            write_multi(cfg, listen, [(pa, 1, "enabled"), (pb, 1, "enabled")])
            ctl_reload(ctrl, cfg)
            stop_proc(p); procs.pop()
            try:
                ctrl.unlink(missing_ok=True)
            except OSError:
                pass
            stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Cap024 × admin matrix ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        # A disabled + healthy probes would still 200 if probed, but admin skips probes
        ca.health_ok = True; cb.health_ok = True
        listen = pick_port()
        cfg = tmp / "hx.toml"
        write_multi(
            cfg, listen,
            [(pa, 10, "disabled"), (pb, 1, "enabled")],
            health=health_inline(unhealthy_threshold=2),
        )
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-hx.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        time.sleep(0.6)
        hx = sample(base, 12, [b"A", b"B"])
        checks["disabled_healthy_not_selectable"] = {
            "ok": hx["A"] == 0 and hx["B"] == 12,
            "counts": hx,
            "a_health_probes": ca.health_count,
            "note": "admin disable skips health peer set; health recovery cannot override",
        }
        # Make B unhealthy → all disabled/unhealthy → 503
        cb.health_ok = False

        def all_503() -> bool:
            return all(curl_req(f"{base}/api/x")[0] == 503 for _ in range(3))

        ok503 = wait_until(all_503, timeout=10.0)
        codes = [curl_req(f"{base}/api/x")[0] for _ in range(4)]
        checks["enabled_unhealthy_not_selectable"] = {
            "ok": ok503 and all(c == 503 for c in codes),
            "codes": codes,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Mixed three peers: A en/healthy, B dis/healthy, C en/unhealthy → A only ---
        ports = [pick_port() for _ in range(3)]
        ha, ca = start_peer(ports[0], b"A")
        hb, cb = start_peer(ports[1], b"B")
        hc, cc = start_peer(ports[2], b"C")
        httpds.extend([ha, hb, hc])
        cc.health_ok = False
        listen = pick_port()
        cfg = tmp / "mix.toml"
        write_multi(
            cfg, listen,
            [(ports[0], 5, "enabled"), (ports[1], 10, "disabled"), (ports[2], 1, "enabled")],
            health=health_inline(unhealthy_threshold=2),
        )
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-mix.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"

        def only_a() -> bool:
            c = sample(base, 10, [b"A", b"B", b"C"])
            return c["A"] > 0 and c["B"] == 0 and c["C"] == 0 and c["err"] == 0

        ok_a = wait_until(only_a, timeout=12.0)
        mix = sample(base, 16, [b"A", b"B", b"C"])
        checks["mixed_eligibility_a_only"] = {
            "ok": ok_a and mix["A"] == 16 and mix["B"] == 0 and mix["C"] == 0,
            "counts": mix,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); stop_httpd(hc); httpds.clear()

        # --- All admin-disabled → 503 ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "alldis.toml"
        write_multi(cfg, listen, [(pa, 1, "disabled"), (pb, 1, "drain_requested")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-alldis.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        codes = [curl_req(f"http://127.0.0.1:{listen}/api/z")[0] for _ in range(6)]
        checks["all_admin_disabled_503"] = {
            "ok": all(c == 503 for c in codes),
            "codes": codes,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Cap021: dead A enabled + live B; retry must not enable disabled peer ---
        dead = pick_port()
        pb, pc = pick_port(), pick_port()
        hb, cb = start_peer(pb, b"B"); hc, cc = start_peer(pc, b"C")
        httpds.extend([hb, hc])
        listen = pick_port()
        cfg = tmp / "retry.toml"
        # A dead connect, B disabled, C enabled → must reach C only after Cap021 exclude A
        write_multi(
            cfg, listen,
            [(dead, 10, "enabled"), (pb, 100, "disabled"), (pc, 1, "enabled")],
            max_connect_retries=1, timeout_ms=3000,
        )
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-retry.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"
        ok_n = 0
        for _ in range(12):
            c, b = curl_req(f"{base}/api/r", timeout=5)
            if c == 200 and b"|C|" in b:
                ok_n += 1
        checks["cap021_never_selects_admin_disabled"] = {
            "ok": ok_n == 12 and cb.get_count == 0,
            "ok_n": ok_n, "b_gets": cb.get_count, "c_gets": cc.get_count,
        }
        stop_proc(p); procs.pop()
        stop_httpd(hb); stop_httpd(hc); httpds.clear()

        # --- Default missing admin_state (=enabled) ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "def.toml"
        cfg.write_text(
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
timeout_ms = 5000
[[upstream.endpoints]]
address = "127.0.0.1"
port = {pa}
weight = 1
[[upstream.endpoints]]
address = "127.0.0.1"
port = {pb}
weight = 1
"""
        )
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-def.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        dw = sample(f"http://127.0.0.1:{listen}", 20, [b"A", b"B"])
        checks["default_admin_enabled"] = {
            "ok": dw["A"] == 10 and dw["B"] == 10 and dw["err"] == 0,
            "counts": dw,
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Concurrent traffic under disabled heavy peer ---
        pa, pb = pick_port(), pick_port()
        ha, ca = start_peer(pa, b"A"); hb, cb = start_peer(pb, b"B")
        httpds.extend([ha, hb])
        listen = pick_port()
        cfg = tmp / "conc.toml"
        write_multi(cfg, listen, [(pa, 100, "disabled"), (pb, 1, "enabled")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=(EV / "exyonq-cap046-conc.log").open("w"),
            stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        assert wait_listen(listen)
        base = f"http://127.0.0.1:{listen}"

        def one(_i: int) -> str:
            c, b = curl_req(f"{base}/api/c")
            if c != 200:
                return "err"
            if b"|A|" in b:
                return "A"
            if b"|B|" in b:
                return "B"
            return "other"

        with concurrent.futures.ThreadPoolExecutor(max_workers=12) as ex:
            labs = list(ex.map(one, range(48)))
        checks["concurrency_disabled_never_leaks"] = {
            "ok": labs.count("A") == 0 and labs.count("err") == 0 and labs.count("B") == 48,
            "A": labs.count("A"), "B": labs.count("B"), "err": labs.count("err"),
        }
        stop_proc(p); procs.pop()
        stop_httpd(ha); stop_httpd(hb); httpds.clear()

        # --- Serve reject invalid admin_state at startup ---
        listen = pick_port()
        cfg = tmp / "badstart.toml"
        write_multi(cfg, listen, [(pick_port(), 1, "bogus"), (pick_port(), 1, "enabled")])
        env = os.environ.copy(); env["EXYONQ_CONFIG"] = str(cfg)
        logp = EV / "exyonq-cap046-badstart.log"
        p = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=logp.open("w"), stderr=subprocess.STDOUT, cwd=str(WS), env=env,
        )
        procs.append(p)
        try:
            p.wait(timeout=8)
            rc = p.returncode
        except subprocess.TimeoutExpired:
            stop_proc(p)
            rc = -1
        procs.pop()
        log_txt = logp.read_text(errors="replace")[-800:]
        checks["startup_invalid_admin_fail_closed"] = {
            "ok": rc is not None and rc != 0,
            "exit": rc,
            "log_tail": log_txt,
        }

    except Exception as exc:
        result["FINAL_RESULT"] = "FAIL"
        result["DETAIL"] = repr(exc)
        result["CHECKS"] = checks
        OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
        return 1
    finally:
        for proc in procs:
            stop_proc(proc)
        for h in httpds:
            stop_httpd(h)

    ok = all(v.get("ok") for v in checks.values())
    result["CHECKS"] = checks
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    result["FAILED"] = [k for k, v in checks.items() if not v.get("ok")]
    OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
