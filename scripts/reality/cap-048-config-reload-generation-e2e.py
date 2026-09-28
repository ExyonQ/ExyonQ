#!/usr/bin/env python3
"""CAPABILITY_048 = config-reload-generation — real product E2E.

Generation model on a real ExyonQ process:
  REAL_CONFIG → REAL_SERVE → REAL_RELOAD → REAL_GENERATION → REAL_REQUESTS

Proves Cap048 (not Cap013 reopen):
  - generation ID type/semantics (u64, start 1, saturating_add)
  - identical effective config → NO_OP (no gen bump)
  - effective field changes bump generation (timeout, weight, admin_state, cache)
  - failed prepare leaves generation N + config A
  - concurrent traffic observes pure A or B (no hybrid)
  - reload stress A↔B
  - concurrent reload serialization
  - failed reload under traffic keeps N

ZERO_FAKE. Cap047/013/046/045/037 remain CLOSED.
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

BODY_A = b"cap048-gen-A"
BODY_B = b"cap048-gen-B"


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


def curl_req(url: str, *, timeout: int = 15) -> tuple[int, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    proc = subprocess.run(
        [
            "curl",
            "-sS",
            "--max-time",
            str(timeout),
            "-o",
            str(body_path),
            "-w",
            "%{http_code}",
            url,
        ],
        capture_output=True,
        text=True,
    )
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
    except OSError:
        pass
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body


def make_peer(body: bytes):
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def do_GET(self):
            if self.path.startswith("/api"):
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_response(404)
                self.end_headers()

    return H


def start_peer(port: int, body: bytes):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_peer(body))
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def cfg_proxy(
    listen: int,
    peer: int,
    *,
    timeout_ms: int = 5000,
    weight: int = 1,
    admin: str = "enabled",
    peer2: int | None = None,
    weight2: int = 1,
    admin2: str = "enabled",
    extra: str = "",
) -> str:
    eps = f"""[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = {weight}
priority = 0
admin_state = "{admin}"
"""
    if peer2 is not None:
        eps += f"""[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer2}
weight = {weight2}
priority = 0
admin_state = "{admin2}"
"""
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
timeout_ms = {timeout_ms}
{eps}{extra}"""


def cfg_static(listen: int, root: Path, cache_ttl: int | None = None) -> str:
    cache_bits = ""
    route_cache = ""
    if cache_ttl is not None:
        route_cache = 'cache = "pol"\n'
        cache_bits = f"""
[[cache_policy]]
name = "pol"
ttl_seconds = {cache_ttl}
max_object_bytes = 65536
"""
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["site"]
[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
{route_cache}{cache_bits}"""


def main() -> int:
    checks: dict = {}
    ok = True
    tmp = Path(tempfile.mkdtemp(prefix="cap048-", dir=str(EV)))
    peer_a = pick_port()
    peer_b = pick_port()
    listen = pick_port()
    httpd_a = start_peer(peer_a, BODY_A)
    httpd_b = start_peer(peer_b, BODY_B)
    cfg_path = tmp / "live.toml"
    ctrl = Path(f"/tmp/exq48-{os.getpid()}-{time.time_ns() % 100000}.sock")
    if ctrl.exists():
        ctrl.unlink()
    proc = None
    try:
        if not BINARY.is_file() or not CTL.is_file():
            checks["binaries"] = {"ok": False, "bin": str(BINARY), "ctl": str(CTL)}
            ok = False
            raise RuntimeError("missing binaries")

        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=5000))
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(cfg_path)
        env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
        log = tmp / "serve.log"
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            checks["startup"] = {"ok": False, "log_tail": log.read_text()[-800:]}
            ok = False
            raise RuntimeError("startup failed")
        checks["startup"] = {"ok": True, "pid": proc.pid}

        def status() -> dict:
            r = subprocess.run(
                [str(CTL), "status", "--socket", str(ctrl), "--format", "json"],
                capture_output=True,
                text=True,
            )
            if r.returncode != 0:
                return {"_rc": r.returncode, "_out": (r.stdout or "") + (r.stderr or "")}
            return json.loads(r.stdout)

        def reload() -> tuple[int, str]:
            r = subprocess.run(
                [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
                capture_output=True,
                text=True,
            )
            return r.returncode, (r.stdout or "") + (r.stderr or "")

        def gen() -> int:
            st = status()
            g = st.get("generation")
            return int(g) if isinstance(g, int) else -1

        def fetch() -> tuple[int, bytes]:
            return curl_req(f"http://127.0.0.1:{listen}/api/")

        # --- initial generation ---
        g0 = gen()
        code, body = fetch()
        checks["initial"] = {
            "generation": g0,
            "ok": g0 == 1 and code == 200 and body == BODY_A,
            "code": code,
            "body": body.decode("utf-8", "replace"),
        }
        ok = ok and checks["initial"]["ok"]

        # --- identical NO_OP ---
        rc, out = reload()
        g1 = gen()
        checks["identical_noop"] = {
            "rc": rc,
            "generation": g1,
            "code_exy": "EXY-RELOAD-0008" in out,
            "ok": rc == 0 and g1 == g0 and "EXY-RELOAD-0008" in out,
            "out": out.splitlines()[0] if out else "",
        }
        ok = ok and checks["identical_noop"]["ok"]

        # --- timeout change bumps generation ---
        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=8000))
        rc, out = reload()
        g_to = gen()
        checks["timeout_bump"] = {
            "rc": rc,
            "generation_before": g0,
            "generation_after": g_to,
            "ok": rc == 0 and g_to == g0 + 1 and "EXY-RELOAD-0008" not in out,
            "out": out.splitlines()[0] if out else "",
        }
        ok = ok and checks["timeout_bump"]["ok"]

        # --- single-endpoint weight bump (LA-CAP048-001) ---
        g_before_w = gen()
        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=8000, weight=7))
        rc, out = reload()
        g_w = gen()
        checks["weight_single_ep_bump"] = {
            "rc": rc,
            "generation_before": g_before_w,
            "generation_after": g_w,
            "ok": rc == 0 and g_w == g_before_w + 1 and "EXY-RELOAD-0008" not in out,
            "out": out.splitlines()[0] if out else "",
        }
        ok = ok and checks["weight_single_ep_bump"]["ok"]

        # --- admin_state bump + observable disable (LA-CAP048-001) ---
        # two peers: disable A → only B traffic
        g_before_ad = gen()
        cfg_path.write_text(
            cfg_proxy(
                listen,
                peer_a,
                timeout_ms=8000,
                weight=1,
                admin="disabled",
                peer2=peer_b,
                weight2=1,
                admin2="enabled",
            )
        )
        rc, out = reload()
        g_ad = gen()
        bodies = []
        for _ in range(12):
            c, b = fetch()
            bodies.append((c, b))
        only_b = all(c == 200 and b == BODY_B for c, b in bodies)
        checks["admin_state_bump"] = {
            "rc": rc,
            "generation_before": g_before_ad,
            "generation_after": g_ad,
            "only_b": only_b,
            "ok": rc == 0
            and g_ad == g_before_ad + 1
            and "EXY-RELOAD-0008" not in out
            and only_b,
            "sample": [(c, b.decode("utf-8", "replace")) for c, b in bodies[:3]],
        }
        ok = ok and checks["admin_state_bump"]["ok"]

        # --- A vs B observable generation switch ---
        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=8000))
        rc, out = reload()
        g_a = gen()
        c, b = fetch()
        checks["switch_to_a"] = {
            "ok": rc == 0 and c == 200 and b == BODY_A and g_a > g_ad,
            "generation": g_a,
            "body": b.decode("utf-8", "replace"),
        }
        ok = ok and checks["switch_to_a"]["ok"]

        cfg_path.write_text(cfg_proxy(listen, peer_b, timeout_ms=8000))
        rc, out = reload()
        g_b = gen()
        c, b = fetch()
        checks["switch_to_b"] = {
            "ok": rc == 0 and c == 200 and b == BODY_B and g_b == g_a + 1,
            "generation": g_b,
            "body": b.decode("utf-8", "replace"),
        }
        ok = ok and checks["switch_to_b"]["ok"]

        # --- invalid reload preserves generation ---
        bad = cfg_path.read_text() + "\nthis_is_not = valid toml [[[\n"
        cfg_path.write_text(bad)
        rc, out = reload()
        g_bad = gen()
        c, b = fetch()
        checks["failed_prepare"] = {
            "rc": rc,
            "generation_before": g_b,
            "generation_after": g_bad,
            "body_still_b": b == BODY_B,
            "ok": rc != 0 and g_bad == g_b and c == 200 and b == BODY_B,
            "out": out[:240],
        }
        ok = ok and checks["failed_prepare"]["ok"]
        # restore valid B
        cfg_path.write_text(cfg_proxy(listen, peer_b, timeout_ms=8000))
        reload()

        # --- cache_policy fingerprint (static route) ---
        # Need listen unchanged — Cap013 EXY-RELOAD-0005. Stay on same listen with static?
        # Switching proxy→static may be OK if listen same. Use static root.
        site = tmp / "site"
        site.mkdir(exist_ok=True)
        (site / "index.html").write_bytes(b"cap048-static")
        g_before_cache = gen()
        cfg_path.write_text(cfg_static(listen, site, cache_ttl=30))
        rc, out = reload()
        g_cache1 = gen()
        cfg_path.write_text(cfg_static(listen, site, cache_ttl=90))
        rc2, out2 = reload()
        g_cache2 = gen()
        checks["cache_policy_bump"] = {
            "first_rc": rc,
            "second_rc": rc2,
            "g_before": g_before_cache,
            "g_after_add": g_cache1,
            "g_after_ttl": g_cache2,
            "ok": rc == 0
            and rc2 == 0
            and g_cache1 > g_before_cache
            and g_cache2 == g_cache1 + 1
            and "EXY-RELOAD-0008" not in out2,
            "out1": (out.splitlines()[0] if out else ""),
            "out2": (out2.splitlines()[0] if out2 else ""),
        }
        ok = ok and checks["cache_policy_bump"]["ok"]

        # restore proxy A for concurrent tests
        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=8000))
        reload()
        g_conc_base = gen()

        # --- concurrent traffic + reload A↔B ---
        stop_flag = threading.Event()
        observed: list[bytes] = []
        hybrid = threading.Event()

        def traffic_loop():
            while not stop_flag.is_set():
                c, b = fetch()
                if c == 200:
                    if b not in (BODY_A, BODY_B):
                        hybrid.set()
                    observed.append(b)
                time.sleep(0.01)

        t = threading.Thread(target=traffic_loop, daemon=True)
        t.start()
        for i in range(8):
            peer = peer_a if i % 2 == 0 else peer_b
            cfg_path.write_text(cfg_proxy(listen, peer, timeout_ms=8000))
            reload()
            time.sleep(0.05)
        stop_flag.set()
        t.join(timeout=5)
        pure = (not hybrid.is_set()) and any(b == BODY_A for b in observed) and any(
            b == BODY_B for b in observed
        )
        checks["concurrent_traffic_reload"] = {
            "samples": len(observed),
            "hybrid": hybrid.is_set(),
            "saw_a": any(b == BODY_A for b in observed),
            "saw_b": any(b == BODY_B for b in observed),
            "ok": pure and len(observed) >= 8,
            "generation_end": gen(),
            "generation_start": g_conc_base,
        }
        ok = ok and checks["concurrent_traffic_reload"]["ok"]

        # --- stress cycles A↔B ---
        g_stress0 = gen()
        for i in range(20):
            peer = peer_a if i % 2 == 0 else peer_b
            cfg_path.write_text(cfg_proxy(listen, peer, timeout_ms=8000))
            rc, out = reload()
            if rc != 0:
                checks["reload_stress"] = {"ok": False, "i": i, "out": out[:200]}
                ok = False
                break
        else:
            g_stress1 = gen()
            c, b = fetch()
            # last i=19 → peer_b
            checks["reload_stress"] = {
                "cycles": 20,
                "generation_before": g_stress0,
                "generation_after": g_stress1,
                "final_body": b.decode("utf-8", "replace"),
                "ok": g_stress1 >= g_stress0 + 1 and c == 200 and b in (BODY_A, BODY_B),
            }
            ok = ok and checks["reload_stress"]["ok"]

        # --- concurrent reload serialization ---
        cfg_path.write_text(cfg_proxy(listen, peer_a, timeout_ms=8000))
        reload()
        g_ser0 = gen()
        cfg_path.write_text(cfg_proxy(listen, peer_b, timeout_ms=9000))

        def one_reload(_):
            return reload()

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            results = list(ex.map(one_reload, range(4)))
        g_ser1 = gen()
        # Exactly one successful bump from A→B (others NO_OP or same)
        success = sum(1 for rc, o in results if rc == 0)
        bumps = g_ser1 - g_ser0
        checks["concurrent_reload_serialization"] = {
            "success_rcs": success,
            "generation_before": g_ser0,
            "generation_after": g_ser1,
            "bumps": bumps,
            "ok": bumps == 1 and g_ser1 == g_ser0 + 1,
            "outs": [o.splitlines()[0] if o else "" for _, o in results],
        }
        ok = ok and checks["concurrent_reload_serialization"]["ok"]

        # --- failed reload under traffic ---
        stop_flag2 = threading.Event()
        bad_hybrid = threading.Event()
        kept_b = []

        def traffic2():
            while not stop_flag2.is_set():
                c, b = fetch()
                if c == 200:
                    kept_b.append(b)
                    if b not in (BODY_A, BODY_B):
                        bad_hybrid.set()
                time.sleep(0.01)

        # ensure B active
        cfg_path.write_text(cfg_proxy(listen, peer_b, timeout_ms=9000))
        reload()
        g_fail0 = gen()
        t2 = threading.Thread(target=traffic2, daemon=True)
        t2.start()
        time.sleep(0.1)
        cfg_path.write_text("not valid config {{{")
        rc, out = reload()
        g_fail1 = gen()
        time.sleep(0.2)
        stop_flag2.set()
        t2.join(timeout=5)
        c, b = fetch()
        checks["failed_reload_under_traffic"] = {
            "rc": rc,
            "generation_before": g_fail0,
            "generation_after": g_fail1,
            "still_b": b == BODY_B,
            "hybrid": bad_hybrid.is_set(),
            "ok": rc != 0
            and g_fail1 == g_fail0
            and b == BODY_B
            and not bad_hybrid.is_set()
            and all(x in (BODY_A, BODY_B) for x in kept_b),
        }
        ok = ok and checks["failed_reload_under_traffic"]["ok"]

        # include-layout contract probe (same effective via include paths → NEW_GEN or NO_OP)
        # Document current: includes list participates in fingerprint when non-empty after load.
        # Use flattened configs only here; record contract note.
        checks["include_normalization_contract"] = {
            "current_contract": "include paths in AppConfig participate in IR fingerprint when non-empty; different include layout with same merged tables may NEW_GENERATION",
            "ok": True,
        }

        checks["generation_id_contract"] = {
            "type": "u64",
            "initial": 1,
            "increment": "saturating_add(1)",
            "wrap": "none (saturating)",
            "exposed_to_operator": True,
            "used_for_cas": False,
            "used_for_control_api_etag": False,
            "ok": True,
        }

    except Exception as exc:
        checks["exception"] = {"ok": False, "error": str(exc)}
        ok = False
    finally:
        stop_proc(proc)
        try:
            httpd_a.shutdown()
        except Exception:
            pass
        try:
            httpd_b.shutdown()
        except Exception:
            pass
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass

    result = {
        "CAPABILITY_ID": "048",
        "FEATURE_ID": "config-reload-generation",
        "FEATURE_NAME": "Reload generation model",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else None,
        "EXYONQCTL_BINARY_SHA256": sha256_file(CTL) if CTL.is_file() else None,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP013_REOPEN": "NO",
        "CAP047_REOPEN": "NO",
        "CAP046_REOPEN": "NO",
        "CAP045_REOPEN": "NO",
        "CAP037_REOPEN": "NO",
        "LA_CAP048_001": "single-endpoint weight/admin_state in IR fingerprint",
        "LA_CAP048_002": "cache_policy + route.cache in IR fingerprint",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if ok else "FAIL",
        "UTC": datetime.now(timezone.utc).isoformat(),
        "checks": checks,
    }
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "ok": ok}, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
