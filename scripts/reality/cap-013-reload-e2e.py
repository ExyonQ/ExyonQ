#!/usr/bin/env python3
"""CAPABILITY_013 = reload — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = reload
  FEATURE_NAME = Config reload
  USER_VISIBLE_CONTRACT = Atomic snapshot swap; reject bad keep last
  CONFIG_SURFACE = EXYONQ_CONTROL_SOCKET
  IMPLEMENTATION_PATH = exyonq-reload-runtime; exyonqctl reload
  PRIMARY_REALITY_STATUS_BEFORE = SMOKE_ONLY

Cap013 proves on a real running ExyonQ process:
  - EXYONQ_CONTROL_SOCKET + EXYONQ_CONFIG bound
  - real client observes config A
  - rewrite daemon-bound config to B
  - exyonqctl reload over the control socket (not process restart)
  - same PID; generation advances; clients observe B; A gone
  - invalid rewrite → reload fails; generation unchanged; B retained

EXPLICIT_NON_SCOPE:
  Cap003 TLS-reload suite as Cap013 proof
  Cap012 path-proxy as Cap013 proof
  D08_RELOAD_REAL_CONTROL (NOT_RUN_NO_VALID_REAL_HARNESS)
  NS5/historical non-evidence
  process restart presented as reload
  SIGHUP
  offline exyonqctl --check alone
  Cap014
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
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

ID_A = b"cap013-config-a-v1"
ID_B = b"cap013-config-b-v1"


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


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
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
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body


def write_config(cfg: Path, *, port: int, root: Path, route_name: str, route_path: str) -> None:
    cfg.write_text(
        f"""config_version = 2

[[server]]
listen = "127.0.0.1:{port}"
routes = ["{route_name}"]

[[route]]
name = "{route_name}"
match = {{ path = "{route_path}" }}
root = "{root}"
index = "index.html"
"""
    )


def ctl_status(sock: Path, cfg: Path) -> dict:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "status", "--socket", str(sock), "--format", "json"],
        capture_output=True,
        text=True,
        env=env,
    )
    if proc.returncode != 0:
        return {"ok": False, "rc": proc.returncode, "stderr": proc.stderr}
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError:
        return {"ok": False, "raw": proc.stdout, "stderr": proc.stderr}


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


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    missing = []
    if not BINARY.is_file():
        missing.append(str(BINARY))
    if not CTL.is_file():
        missing.append(str(CTL))
    if missing:
        OUT.write_text(
            json.dumps(
                {
                    "FEATURE_ID": "reload",
                    "CAPABILITY": "CAPABILITY_013",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binaries: {missing}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    port = pick_port()
    root_a = EV / "docroot-a"
    root_b = EV / "docroot-b"
    root_a.mkdir(parents=True, exist_ok=True)
    root_b.mkdir(parents=True, exist_ok=True)
    (root_a / "index.html").write_bytes(ID_A)
    (root_b / "index.html").write_bytes(ID_B)

    cfg = EV / "exyonq-cap013.toml"
    ctrl = EV / "control.sock"
    if ctrl.exists() or ctrl.is_symlink():
        ctrl.unlink()

    write_config(cfg, port=port, root=root_a, route_name="cap013a", route_path="/cap013")
    log = EV / "exyonq-cap013.log"
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(cfg)
    env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)

    srv = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
    )
    pid_before = srv.pid
    base = f"http://127.0.0.1:{port}"
    checks: dict = {}
    result: dict = {
        "FEATURE_ID": "reload",
        "CAPABILITY": "CAPABILITY_013",
        "CAPABILITY_NAME": "reload",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQ_BINARY_SHA256": sha256_file(BINARY),
        "EXYONQCTL_BINARY": str(CTL),
        "EXYONQCTL_BINARY_SHA256": sha256_file(CTL),
        "CONFIG_SHA256_INITIAL": sha256_file(cfg),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Atomic snapshot swap; reject bad keep last",
        "SUPPORTED_BEHAVIOR": (
            "EXYONQ_CONTROL_SOCKET + exyonqctl reload → generation swap; "
            "invalid reload keeps last-good behavior"
        ),
        "EXPLICIT_NON_SCOPE": [
            "Cap003 TLS-reload suite as Cap013 proof",
            "Cap012 path-proxy as Cap013 proof",
            "D08_RELOAD_REAL_CONTROL NOT_RUN_NO_VALID_REAL_HARNESS",
            "NS5/historical non-evidence",
            "process restart presented as reload",
            "SIGHUP",
            "offline --check alone",
            "Cap014",
        ],
        "CONFIG_SURFACE": "EXYONQ_CONTROL_SOCKET",
        "OPEN_DEFECT_CONTEXT": "RD-001,RD-007",
        "D08_RELOAD_REAL_CONTROL_NOT_USED_AS_PROOF": "YES",
        "NS5_NON_EVIDENCE_NOT_USED_AS_PROOF": "YES",
        "RELOAD_VIA_PROCESS_RESTART": "NO",
        "PID_BEFORE": pid_before,
        "checks": checks,
    }

    try:
        if not wait_listen(port) or not wait_sock(ctrl) or srv.poll() is not None:
            result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
            result["DETAIL"] = "server or control socket not ready"
            result["SERVER_LOG_TAIL"] = log.read_text(errors="replace")[-4000:]
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 2

        # Positive A
        c_a, b_a = curl_req(f"{base}/cap013/")
        a_ok = c_a == 200 and b_a.startswith(ID_A) and ID_B not in b_a
        checks["initial_config_a"] = {"code": c_a, "ok": a_ok}

        st0 = ctl_status(ctrl, cfg)
        gen0 = int(st0.get("generation", -1)) if isinstance(st0.get("generation"), int) else -1
        checks["status_generation_initial"] = {"generation": gen0, "ok": gen0 >= 0}

        # Apply B via in-place rewrite of daemon-bound config + real reload.
        # Allow FS watcher debounce to settle, then control-plane reload.
        # Identical second apply must be NO_OP (no extra generation bump).
        write_config(cfg, port=port, root=root_b, route_name="cap013b", route_path="/cap013")
        time.sleep(0.5)
        rc_rel, rel_out = ctl_reload(ctrl, cfg)
        st1 = ctl_status(ctrl, cfg)
        gen1 = int(st1.get("generation", -1)) if isinstance(st1.get("generation"), int) else -1
        c_b, b_b = curl_req(f"{base}/cap013/")
        same_pid = srv.poll() is None and srv.pid == pid_before
        # Exactly one generation step A→B (watcher and/or ctl), never double-bump.
        single_step = gen1 == gen0 + 1
        b_ok = (
            rc_rel == 0
            and single_step
            and c_b == 200
            and b_b.startswith(ID_B)
            and ID_A not in b_b
            and same_pid
        )
        checks["reload_config_b"] = {
            "reload_rc": rc_rel,
            "generation_before": gen0,
            "generation_after": gen1,
            "single_generation_step": single_step,
            "code": c_b,
            "same_pid": same_pid,
            "pid_after": srv.pid if srv.poll() is None else None,
            "ok": b_ok,
            "reload_output": rel_out[-500:],
        }

        # Identical re-apply must not advance generation (EXY-RELOAD-0008 / NO_OP).
        rc_id, id_out = ctl_reload(ctrl, cfg)
        st_id = ctl_status(ctrl, cfg)
        gen_id = int(st_id.get("generation", -1)) if isinstance(st_id.get("generation"), int) else -1
        identical_ok = rc_id == 0 and gen_id == gen1
        checks["identical_reload_noop"] = {
            "reload_rc": rc_id,
            "generation_before": gen1,
            "generation_after": gen_id,
            "ok": identical_ok,
            "reload_output": id_out[-500:],
        }

        # Negative: outside route
        c_out, b_out = curl_req(f"{base}/outside-cap013")
        neg_ok = c_out != 200 and ID_A not in b_out and ID_B not in b_out
        checks["outside_route"] = {"code": c_out, "ok": neg_ok}

        # Failure: invalid config keep last (must be a real parse reject, not an ignored unknown key)
        good_bytes = cfg.read_bytes()
        cfg.write_bytes(good_bytes + b"\n[[[ broken\n")
        # Allow optional FS watcher debounce to attempt reject first, then control-plane reload.
        time.sleep(0.5)
        rc_bad, bad_out = ctl_reload(ctrl, cfg)
        st2 = ctl_status(ctrl, cfg)
        gen2 = int(st2.get("generation", -1)) if isinstance(st2.get("generation"), int) else -1
        c_keep, b_keep = curl_req(f"{base}/cap013/")
        fail_ok = (
            rc_bad != 0
            and gen2 == gen1
            and c_keep == 200
            and b_keep.startswith(ID_B)
            and ID_A not in b_keep
            and srv.poll() is None
            and srv.pid == pid_before
        )
        checks["invalid_reload_keep_last"] = {
            "reload_rc": rc_bad,
            "generation_before": gen1,
            "generation_after": gen2,
            "code": c_keep,
            "ok": fail_ok,
            "reload_output": bad_out[-500:],
        }
        # Restore valid bytes for cleanliness (daemon already kept gen B)
        cfg.write_bytes(good_bytes)
        time.sleep(0.5)

        # Listen bind change must reject (EXY-RELOAD-0005), keep last-good B.
        bad_listen_port = pick_port()
        write_config(
            cfg,
            port=bad_listen_port,
            root=root_b,
            route_name="cap013b",
            route_path="/cap013",
        )
        time.sleep(0.5)
        rc_listen, listen_out = ctl_reload(ctrl, cfg)
        st_l = ctl_status(ctrl, cfg)
        gen_l = int(st_l.get("generation", -1)) if isinstance(st_l.get("generation"), int) else -1
        c_l, b_l = curl_req(f"{base}/cap013/")
        listen_ok = (
            rc_listen != 0
            and "EXY-RELOAD-0005" in listen_out
            and gen_l == gen1
            and c_l == 200
            and b_l.startswith(ID_B)
            and same_pid
        )
        checks["listen_bind_change_rejected"] = {
            "reload_rc": rc_listen,
            "generation_before": gen1,
            "generation_after": gen_l,
            "code": c_l,
            "ok": listen_ok,
            "reload_output": listen_out[-500:],
        }
        # Restore B config (same listen as process)
        write_config(cfg, port=port, root=root_b, route_name="cap013b", route_path="/cap013")
        time.sleep(0.5)

        # Concurrency after successful B
        def one(_: int) -> bool:
            code, body = curl_req(f"{base}/cap013/")
            return code == 200 and body.startswith(ID_B) and ID_A not in body

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as pool:
            conc_ok = all(pool.map(one, range(6)))
        checks["concurrency_after_reload"] = {"ok": conc_ok, "n": 6}

        # Boundary: A marker must not reappear
        c_fin, b_fin = curl_req(f"{base}/cap013/")
        boundary_ok = c_fin == 200 and b_fin.startswith(ID_B) and ID_A not in b_fin and same_pid
        checks["a_behavior_gone"] = {"code": c_fin, "ok": boundary_ok}

        positive = a_ok and b_ok and identical_ok
        negative = neg_ok
        failure = fail_ok and listen_ok
        boundary = boundary_ok and neg_ok and listen_ok
        conc = conc_ok
        overall = positive and negative and failure and boundary and conc and (srv.poll() is None)

        result["PID_AFTER"] = srv.pid if srv.poll() is None else None
        result["CAP013_POSITIVE_STATUS"] = "PASS" if positive else "FAIL"
        result["CAP013_NEGATIVE_STATUS"] = "PASS" if negative else "FAIL"
        result["CAP013_FAILURE_STATUS"] = "PASS" if failure else "FAIL"
        result["CAP013_BOUNDARY_STATUS"] = "PASS" if boundary else "FAIL"
        result["CAP013_CONCURRENCY_OR_LIFECYCLE_STATUS"] = "PASS" if conc else "FAIL"
        result["CAP013_CROSS_CAPABILITY_INVARIANTS"] = {
            "RELOAD_VIA_CONTROL_SOCKET": "YES",
            "RELOAD_VIA_PROCESS_RESTART": "NO",
            "SAME_PID_ACROSS_RELOAD": "YES" if same_pid else "NO",
            "GENERATION_SINGLE_STEP_A_TO_B": "YES" if single_step else "NO",
            "IDENTICAL_RELOAD_NO_OP": "YES" if identical_ok else "NO",
            "INVALID_RELOAD_KEEPS_LAST": "YES" if fail_ok else "NO",
            "LISTEN_BIND_CHANGE_REJECTED": "YES" if listen_ok else "NO",
            "UPSTREAM_FAILURE_FALSE_SUCCESS": "NO",
            "D08_RELOAD_REAL_CONTROL_NOT_USED_AS_PROOF": "YES",
            "NS5_NON_EVIDENCE_NOT_USED_AS_PROOF": "YES",
            "CAP012_CLOSED_NOT_USED_AS_CAP013_PROOF": "YES",
            "CAP012_LA_002_CLASSIFICATION": "DEFENSIVE_HARDENING",
        }
        result["PRODUCT_DEFECT"] = "YES" if not overall else "NO"
        result["HARNESS_DEFECT"] = "NO"
        result["ENVIRONMENT_BLOCKER"] = "NO"
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


if __name__ == "__main__":
    sys.exit(main())
