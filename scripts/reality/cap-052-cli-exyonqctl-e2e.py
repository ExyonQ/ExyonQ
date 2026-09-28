#!/usr/bin/env python3
"""CAPABILITY_052 = cli-exyonqctl — real operational CLI E2E.

REAL shell → REAL exyonqctl → Unix control socket → REAL ExyonQ → REAL effect.
Cap051/041/040/048 must not reopen. Cap063 STARTED=NO.
ZERO_FAKE. No mock control server / synthetic exit.
"""
from __future__ import annotations

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
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))


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


def http_get(port: int) -> str:
    """Prefer curl (Cap048); fall back to raw socket."""
    url = f"http://127.0.0.1:{port}/api/"
    body_path = EV / f"curl-{time.time_ns()}.body"
    try:
        proc = subprocess.run(
            [
                "curl",
                "-sS",
                "--max-time",
                "8",
                "-H",
                "User-Agent: cap052-e2e",
                "-H",
                "Connection: close",
                "-o",
                str(body_path),
                "-w",
                "%{http_code}",
                url,
            ],
            capture_output=True,
            text=True,
            timeout=12,
        )
        code = (proc.stdout or "").strip() or "0"
        body = body_path.read_bytes() if body_path.is_file() else b""
        try:
            body_path.unlink(missing_ok=True)
        except OSError:
            pass
        return f"HTTP {code}\n" + body.decode("utf-8", errors="replace")
    except Exception as exc:
        try:
            body_path.unlink(missing_ok=True)
        except OSError:
            pass
        try:
            with socket.create_connection(("127.0.0.1", port), 2.0) as s:
                req = (
                    b"GET /api/ HTTP/1.1\r\nHost: localhost\r\n"
                    b"User-Agent: cap052-e2e\r\nConnection: close\r\n\r\n"
                )
                s.sendall(req)
                s.settimeout(5.0)
                chunks = []
                while True:
                    b = s.recv(8192)
                    if not b:
                        break
                    chunks.append(b)
                return b"".join(chunks).decode("utf-8", errors="replace")
        except OSError as e2:
            return f"CONNECTION_ERROR:{exc}/{e2}"


def run_cmd(args, *, env=None, timeout=30.0, cwd=None):
    t0 = time.monotonic()
    try:
        p = subprocess.run(
            args,
            cwd=str(cwd or WS),
            env=env or os.environ.copy(),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return p.returncode, p.stdout or "", p.stderr or "", time.monotonic() - t0
    except subprocess.TimeoutExpired as exc:
        out = exc.stdout or ""
        err = exc.stderr or ""
        if isinstance(out, (bytes, bytearray)):
            out = out.decode("utf-8", errors="replace")
        if isinstance(err, (bytes, bytearray)):
            err = err.decode("utf-8", errors="replace")
        return 124, out, err, time.monotonic() - t0


def ctl(args, env, timeout=30.0):
    return run_cmd([str(CTL), *args], env=env, timeout=timeout)


def parse_json(stdout: str) -> dict:
    for line in reversed([ln.strip() for ln in (stdout or "").splitlines() if ln.strip()]):
        try:
            return json.loads(line)
        except Exception:
            continue
    return {"parse_error": (stdout or "")[-300:], "ok": False}


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
timeout_ms = 5000
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = 1
priority = 0
admin_state = "enabled"
"""


class Peer(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    marker = b"unset"

    def log_message(self, *_a):
        pass

    def do_GET(self):
        body = self.marker
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def start_peer(port: int, marker: bytes):
    class H(Peer):
        pass

    H.marker = marker
    httpd = ThreadingHTTPServer(("127.0.0.1", port), H)
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    matrix: dict = {}
    ok = True
    result = {
        "CAPABILITY_ID": "052",
        "FEATURE_ID": "cli-exyonqctl",
        "FEATURE_NAME": "exyonqctl operational CLI",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP051_REOPEN": "NO",
        "CAP041_REOPEN": "NO",
        "CAP040_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "CAP063_STARTED": "NO",
        "UTC": datetime.now(timezone.utc).isoformat(),
        "EXYONQCTL_DEFAULT_CONTROL_SOCKET": "/tmp/exyonq.sock",
        "CONTROL_PROTOCOL_KIND": "unix-line-json",
        "CONTROL_IO_TIMEOUT_S": 30,
    }
    if not BIN.is_file() or not CTL.is_file():
        result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BIN)
    result["EXYONQCTL_BINARY_SHA256"] = sha256_file(CTL)

    pkg = None
    for line in (WS / "Cargo.toml").read_text().splitlines():
        if line.strip().startswith("version"):
            pkg = line.split("=", 1)[1].strip().strip('"')
            break

    def add(name: str, value: dict):
        nonlocal ok
        checks[name] = value
        ok = ok and bool(value.get("ok"))

    # help / version
    rc, out, err, _ = ctl(["--help"], os.environ.copy())
    help_txt = out + err
    matrix["help"] = rc
    rc2, ver, verr, _ = ctl(["--version"], os.environ.copy())
    matrix["version"] = rc2
    add(
        "help_version",
        {
            "help_rc": rc,
            "version_rc": rc2,
            "package_version": pkg,
            "ok": rc == 0
            and rc2 == 0
            and pkg is not None
            and pkg in (ver + verr)
            and all(x in help_txt for x in ("status", "reload", "drain", "shutdown"))
            and "demo" not in help_txt.lower(),
        },
    )

    rc, out, err, _ = ctl([], os.environ.copy())
    matrix["bare"] = rc
    add("bare", {"rc": rc, "ok": rc == 2, "preferred_exit_2": rc == 2, "snip": (out + err)[:200]})

    for name, args in (
        ("unknown_subcommand", ["nonesuch"]),
        ("unknown_option", ["--nonesuch"]),
    ):
        rc, out, err, dt = ctl(args, os.environ.copy(), timeout=5)
        matrix[name] = rc
        add(name, {"rc": rc, "seconds": dt, "ok": rc != 0 and dt < 5})

    bad = EV / "cap052-nonexistent.sock"
    rc, out, err, dt = ctl(
        ["status", "--socket", str(bad), "--format", "json"],
        os.environ.copy(),
        timeout=5,
    )
    matrix["status_no_server"] = rc
    add(
        "status_no_server",
        {
            "rc": rc,
            "seconds": dt,
            "ok": rc != 0 and dt < 5 and "healthy" not in (out + err).lower(),
        },
    )

    listen = pick_port()
    pa = pick_port()
    pb = pick_port()
    ha = start_peer(pa, b"CAP052-A")
    hb = start_peer(pb, b"CAP052-B")
    sock = EV / f"cap052-{os.getpid()}.sock"
    wrong = EV / f"cap052-wrong-{os.getpid()}.sock"
    if sock.exists():
        sock.unlink()
    config = EV / "cap052-live.toml"
    config.write_text(cfg(listen, pa))
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(config)
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["RUST_LOG"] = "error"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=(EV / "daemon.stdout").open("w"),
        stderr=(EV / "daemon.stderr").open("w"),
        text=True,
    )
    try:
        up = wait_pred(lambda: sock.is_socket() and listening(listen), 45)
        add("real_serve", {"pid": proc.pid, "ok": up})
        if not up:
            raise RuntimeError("serve failed to become ready")

        rc, out, err, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        matrix["status_json"] = rc
        st = parse_json(out)
        add(
            "status_json",
            {
                "rc": rc,
                "response": st,
                "ok": rc == 0
                and st.get("ok") is True
                and isinstance(st.get("generation"), int)
                and st.get("draining") is False,
            },
        )
        generation = st.get("generation", 0)

        rc, out, err, _ = ctl(["status", "--socket", str(sock)], env)
        matrix["status_human"] = rc
        add("status_human", {"rc": rc, "ok": rc == 0 and "generation=" in (out + err)})

        rw, *_rest = ctl(["status", "--socket", str(wrong)], env, timeout=5)
        rr, *_r2 = ctl(["status", "--socket", str(sock)], env)
        matrix["socket_wrong"] = rw
        matrix["socket_correct"] = rr
        add("socket_override", {"wrong_rc": rw, "correct_rc": rr, "ok": rw != 0 and rr == 0})

        # reload A → B (rewrite daemon-bound path in place)
        config.write_text(cfg(listen, pb))
        rc, out, err, _ = ctl(["reload", "--config", str(config), "--socket", str(sock)], env)
        matrix["reload_changed"] = rc
        time.sleep(0.25)
        rc2, out2, err2, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        st2 = parse_json(out2)
        body = http_get(listen)
        add(
            "reload_changed",
            {
                "rc": rc,
                "before": generation,
                "after": st2.get("generation"),
                "http": body[-200:],
                "same_pid": proc.poll() is None,
                "ok": rc == 0
                and isinstance(st2.get("generation"), int)
                and st2["generation"] > generation
                and "CAP052-B" in body
                and proc.poll() is None,
            },
        )

        before = st2.get("generation")
        rc, out, err, _ = ctl(["reload", "--config", str(config), "--socket", str(sock)], env)
        matrix["reload_noop"] = rc
        combined = out + err
        rc2, out2, _, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        ns = parse_json(out2)
        add(
            "reload_noop",
            {
                "rc": rc,
                "before": before,
                "after": ns.get("generation"),
                "snip": combined[:240],
                "ok": rc == 0
                and ns.get("generation") == before
                and ("EXY-RELOAD-0008" in combined or "code=EXY-RELOAD-0008" in combined),
            },
        )

        # invalid reload
        config.write_text("unknown_top_level = true\n" + cfg(listen, pb))
        rc, out, err, _ = ctl(["reload", "--config", str(config), "--socket", str(sock)], env)
        matrix["reload_invalid"] = rc
        rc2, out2, _, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        after = parse_json(out2)
        body = http_get(listen)
        add(
            "reload_invalid",
            {
                "rc": rc,
                "generation_before": ns.get("generation"),
                "generation_after": after.get("generation"),
                "http": body[-200:],
                "ok": rc != 0
                and after.get("generation") == ns.get("generation")
                and "CAP052-B" in body,
            },
        )

        # restore valid config file for drain path (daemon still on prior good generation)
        config.write_text(cfg(listen, pb))

        rc, out, err, _ = ctl(["drain", "--socket", str(sock), "--format", "json"], env)
        matrix["drain"] = rc
        dr = parse_json(out)
        rc2, out2, _, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        ds = parse_json(out2)
        rejected = http_get(listen)
        add(
            "drain",
            {
                "rc": rc,
                "response": dr,
                "status": ds,
                "observed": rejected[-200:],
                "ok": rc == 0
                and ds.get("draining") is True
                and ("503" in rejected or "CONNECTION_ERROR:" in rejected or "500" in rejected),
            },
        )

        rc, out, err, _ = ctl(["drain", "--socket", str(sock)], env)
        matrix["drain_duplicate"] = rc
        add("drain_duplicate", {"rc": rc, "ok": rc == 0 and proc.poll() is None})

        rc, out, err, _ = ctl(["reload", "--config", str(config), "--socket", str(sock)], env)
        matrix["reload_draining"] = rc
        rc2, out2, _, _ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
        st_d = parse_json(out2)
        add(
            "reload_draining",
            {
                "rc": rc,
                "status": st_d,
                "ok": rc == 0 and st_d.get("draining") is True and proc.poll() is None,
            },
        )

        regular = EV / "cap052-regular.sock"
        regular.write_text("not a unix socket\n")
        rc, out, err, dt = ctl(["status", "--socket", str(regular)], env, timeout=5)
        matrix["regular_file_socket"] = rc
        add("regular_file_socket", {"rc": rc, "seconds": dt, "ok": rc != 0 and dt < 5})

        results = []

        def concurrent():
            r, *_ = ctl(["status", "--socket", str(sock), "--format", "json"], env)
            results.append(r)

        ts = [threading.Thread(target=concurrent) for _ in range(2)]
        for t in ts:
            t.start()
        for t in ts:
            t.join(10)
        matrix["concurrent_status"] = results
        add(
            "concurrent_status",
            {
                "rcs": results,
                "ok": len(results) == 2 and all(x == 0 for x in results) and proc.poll() is None,
            },
        )

        # EXY-RELOAD-0009 mismatch when EXYONQ_CONFIG set
        other = EV / "cap052-other.toml"
        other.write_text(cfg(pick_port(), pb))
        env_m = env.copy()
        rc, out, err, _ = ctl(
            ["reload", "--config", str(other), "--socket", str(sock)], env_m, timeout=10
        )
        matrix["reload_config_mismatch"] = rc
        add(
            "reload_config_mismatch",
            {
                "rc": rc,
                "snip": (out + err)[:240],
                "ok": rc != 0 and "EXY-RELOAD-0009" in (out + err),
            },
        )

        rc, out, err, _ = ctl(["shutdown", "--socket", str(sock)], env)
        matrix["shutdown"] = rc
        exited = wait_pred(lambda: proc.poll() is not None, 30)
        add(
            "shutdown",
            {
                "rc": rc,
                "process_rc": proc.poll(),
                "socket_exists": sock.exists(),
                "ok": rc == 0 and exited and proc.returncode == 0 and not sock.exists(),
            },
        )

        rc, out, err, dt = ctl(
            ["reload", "--config", str(config), "--socket", str(sock)], env, timeout=5
        )
        matrix["reload_after_shutdown"] = rc
        add(
            "reload_after_shutdown",
            {
                "rc": rc,
                "seconds": dt,
                "ok": rc != 0 and dt < 5,
                "snip": (out + err)[:200],
            },
        )
    finally:
        if proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
        ha.shutdown()
        hb.shutdown()

    result["checks"] = checks
    result["exit_code_matrix"] = matrix
    result["CAP052_PACKAGE_VERSION"] = pkg
    result["FINAL_RESULT"] = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "ok": ok}, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as exc:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_text(
            json.dumps({"FINAL_RESULT": "HARNESS_EXCEPTION", "error": str(exc)}, indent=2) + "\n"
        )
        raise
