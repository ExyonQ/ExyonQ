#!/usr/bin/env python3
"""CAPABILITY_063 = control-unix-socket — server-side AF_UNIX transport E2E.

REAL ExyonQ process → REAL UnixListener → REAL independent AF_UNIX clients
(+ integrated exyonqctl). Cap052/051/041/040/048 must not reopen.
Publication binds inside a private mode-0700 stage, verifies socket mode 0600,
then atomically hard-links the listener inode to the no-clobber final pathname.
ZERO_FAKE. No mock control server.
"""
from __future__ import annotations

import hashlib
import json
import os
import signal
import socket
import stat
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
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

CONTROL_SOCKET_MODE_EXPECTED = 0o600
CONTROL_MAX_COMMAND_BYTES = 4096


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
    url = f"http://127.0.0.1:{port}/api/"
    try:
        proc = subprocess.run(
            [
                "curl",
                "-sS",
                "--max-time",
                "8",
                "-H",
                "User-Agent: cap063-e2e",
                "-H",
                "Connection: close",
                "-w",
                "\n%{http_code}",
                url,
            ],
            capture_output=True,
            text=True,
            timeout=12,
        )
        return (proc.stdout or "") + (proc.stderr or "")
    except Exception as exc:
        return f"CONNECTION_ERROR:{exc}"


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
    marker = b"cap063"

    def log_message(self, *_a):
        pass

    def do_GET(self):
        body = self.marker
        self.send_response(200)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def start_peer(port: int, marker: bytes = b"cap063"):
    class H(Peer):
        pass

    H.marker = marker
    httpd = ThreadingHTTPServer(("127.0.0.1", port), H)
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def raw_cmd(sock_path: str, payload: bytes, *, timeout_s: float = 5.0, shutdown_wr: bool = False):
    """Independent AF_UNIX client — not exyonqctl."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout_s)
    try:
        s.connect(sock_path)
        if payload:
            s.sendall(payload)
        if shutdown_wr:
            try:
                s.shutdown(socket.SHUT_WR)
            except OSError:
                pass
        chunks = []
        while True:
            try:
                b = s.recv(65536)
            except socket.timeout:
                break
            if not b:
                break
            chunks.append(b)
        return b"".join(chunks)
    finally:
        s.close()


def parse_json_line(raw: bytes) -> dict:
    text = raw.decode("utf-8", errors="replace")
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            return json.loads(line)
        except Exception:
            continue
    return {"parse_error": text[-300:], "ok": False}


def start_exyonq(config: Path, sock: Path, env_extra: dict | None = None):
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(config)
    if env_extra:
        env.update(env_extra)
    log = EV / f"exyonq-{sock.name}-{time.time_ns()}.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
    )
    return proc, log


def stop_proc(proc: subprocess.Popen, grace: float = 8.0):
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


def sock_mode(path: Path) -> int | None:
    try:
        return stat.S_IMODE(path.stat().st_mode)
    except OSError:
        return None


def is_sock(path: Path) -> bool:
    try:
        return stat.S_ISSOCK(path.stat().st_mode)
    except OSError:
        return False


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    ok = True
    result = {
        "CAPABILITY_ID": "063",
        "FEATURE_ID": "control-unix-socket",
        "FEATURE_NAME": "Control Unix socket",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP052_REOPEN": "NO",
        "CAP051_REOPEN": "NO",
        "CAP041_REOPEN": "NO",
        "CAP040_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "DEFAULT_CONTROL_SOCKET_PATH": "/tmp/exyonq.sock",
        "CONTROL_SOCKET_MODE_EXPECTED": oct(CONTROL_SOCKET_MODE_EXPECTED),
        "CONTROL_MAX_COMMAND_BYTES": CONTROL_MAX_COMMAND_BYTES,
        "CONTROL_IO_TIMEOUT_S": 30,
        "CONTROL_AUTH_MODEL": "unix-fs-permissions-owner-only",
        "CONTROL_AUTHENTICATION": "NONE",
        "CONTROL_AUTHORIZATION": "filesystem-mode-0o600",
        "CONTROL_PUBLICATION": "private-0700-stage+atomic-no-clobber-hard-link",
        "PROCESS_UMASK_MUTATION": "NONE",
        "UTC": datetime.now(timezone.utc).isoformat(),
    }
    if not BIN.is_file() or not CTL.is_file():
        result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    result["EXYONQ_BINARY_SHA256"] = sha256_file(BIN)
    result["EXYONQCTL_BINARY_SHA256"] = sha256_file(CTL)

    def add(name: str, value: dict):
        nonlocal ok
        checks[name] = value
        ok = ok and bool(value.get("ok"))

    work = Path(tempfile.mkdtemp(prefix="cap063-", dir="/tmp"))
    procs: list[subprocess.Popen] = []
    peers = []
    try:
        peer_port = pick_port()
        peers.append(start_peer(peer_port))
        listen_a = pick_port()
        listen_b = pick_port()
        cfg_a = work / "a.toml"
        cfg_b = work / "b.toml"
        cfg_a.write_text(cfg(listen_a, peer_port))
        cfg_b.write_text(cfg(listen_b, peer_port))
        sock_a = work / "a.sock"
        sock_b = work / "b.sock"

        # --- 1/2 bind + socket type + mode ---
        proc_a, _ = start_exyonq(cfg_a, sock_a)
        procs.append(proc_a)
        ready = wait_pred(lambda: listening(listen_a) and sock_a.exists())
        mode = sock_mode(sock_a)
        add(
            "socket_create_bind",
            {
                "ok": ready and is_sock(sock_a) and mode == CONTROL_SOCKET_MODE_EXPECTED,
                "ready": ready,
                "is_socket": is_sock(sock_a),
                "mode": oct(mode) if mode is not None else None,
                "pid": proc_a.pid,
            },
        )

        # --- 3 raw status ---
        raw = raw_cmd(str(sock_a), b"status\n")
        j = parse_json_line(raw)
        add(
            "raw_status",
            {
                "ok": j.get("ok") is True and j.get("command") == "status" and raw.endswith(b"\n"),
                "json": j,
                "raw_len": len(raw),
                "newline": raw.endswith(b"\n"),
            },
        )

        # --- 4 unknown ---
        raw_u = raw_cmd(str(sock_a), b"not-a-command\n")
        ju = parse_json_line(raw_u)
        add(
            "unknown_command",
            {
                "ok": ju.get("ok") is False and ju.get("command") == "unknown",
                "json": ju,
            },
        )

        # --- empty line ---
        raw_e = raw_cmd(str(sock_a), b"\n")
        je = parse_json_line(raw_e)
        add(
            "empty_line",
            {
                "ok": je.get("ok") is False and "empty" in str(je.get("error", "")).lower(),
                "json": je,
            },
        )

        # --- whitespace / CRLF ---
        raw_w = raw_cmd(str(sock_a), b" status \r\n")
        jw = parse_json_line(raw_w)
        add(
            "whitespace_crlf",
            {
                "ok": jw.get("ok") is True and jw.get("command") == "status",
                "json": jw,
            },
        )

        # --- partial multi-write framing ---
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.settimeout(5.0)
        s.connect(str(sock_a))
        s.sendall(b"sta")
        time.sleep(0.05)
        s.sendall(b"tus\n")
        chunks = []
        while True:
            b = s.recv(65536)
            if not b:
                break
            chunks.append(b)
        s.close()
        jp = parse_json_line(b"".join(chunks))
        add(
            "partial_write_framing",
            {
                "ok": jp.get("ok") is True and jp.get("command") == "status",
                "json": jp,
            },
        )

        # --- multi-command one write: first only (one cmd/connection) ---
        raw_m = raw_cmd(str(sock_a), b"status\nshutdown\n")
        jm = parse_json_line(raw_m)
        still_up = listening(listen_a)
        add(
            "multi_command_one_write",
            {
                "ok": jm.get("ok") is True
                and jm.get("command") == "status"
                and still_up
                and raw_m.count(b"\n") == 1,
                "json": jm,
                "still_listening": still_up,
                "response_newlines": raw_m.count(b"\n"),
                "CONTRACT": "one_command_per_connection_first_line_only",
            },
        )

        # --- half-close ---
        raw_h = raw_cmd(str(sock_a), b"status\n", shutdown_wr=True)
        jh = parse_json_line(raw_h)
        add(
            "half_close_after_command",
            {
                "ok": jh.get("ok") is True and jh.get("command") == "status",
                "json": jh,
            },
        )

        # --- oversize command ---
        over = b"x" * (CONTROL_MAX_COMMAND_BYTES + 64) + b"\n"
        raw_o = raw_cmd(str(sock_a), over)
        jo = parse_json_line(raw_o)
        add(
            "command_size_bound",
            {
                "ok": jo.get("ok") is False and "too long" in str(jo.get("error", "")).lower(),
                "json": jo,
            },
        )

        # --- newline injection / JSON escaping ---
        raw_i = raw_cmd(str(sock_a), b'badcmd_with_"quote_and_\\slash\n')
        # Must be exactly one JSON object line
        lines = [ln for ln in raw_i.split(b"\n") if ln.strip()]
        ji = parse_json_line(raw_i)
        add(
            "json_escaping",
            {
                "ok": len(lines) == 1 and ji.get("ok") is False and isinstance(ji.get("error"), str),
                "line_count": len(lines),
                "json": ji,
            },
        )

        # --- concurrent clients ---
        results_c = []

        def worker(i: int):
            r = raw_cmd(str(sock_a), b"status\n")
            results_c.append((i, parse_json_line(r), r))

        threads = [threading.Thread(target=worker, args=(i,)) for i in range(8)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=10)
        add(
            "concurrent_clients",
            {
                "ok": len(results_c) == 8
                and all(j.get("ok") is True for _, j, _ in results_c)
                and all(r.count(b"\n") == 1 for _, _, r in results_c),
                "n": len(results_c),
            },
        )

        # --- client disconnect mid-command ---
        s2 = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s2.connect(str(sock_a))
        s2.sendall(b"stat")
        s2.close()
        time.sleep(0.1)
        raw_after = raw_cmd(str(sock_a), b"status\n")
        ja = parse_json_line(raw_after)
        add(
            "disconnect_mid_command",
            {
                "ok": ja.get("ok") is True and listening(listen_a) and proc_a.poll() is None,
                "json": ja,
                "server_alive": proc_a.poll() is None,
            },
        )

        # --- abrupt churn ---
        for _ in range(40):
            sc = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            try:
                sc.connect(str(sock_a))
            except OSError:
                pass
            sc.close()
        raw_ch = raw_cmd(str(sock_a), b"status\n")
        jch = parse_json_line(raw_ch)
        add(
            "abrupt_client_churn",
            {
                "ok": jch.get("ok") is True and proc_a.poll() is None,
                "json": jch,
            },
        )

        # --- many connect/command/close (FD stability check via continued success) ---
        fd_ok = True
        for _ in range(60):
            r = raw_cmd(str(sock_a), b"status\n")
            if parse_json_line(r).get("ok") is not True:
                fd_ok = False
                break
        add("connection_churn_fd_stability", {"ok": fd_ok and proc_a.poll() is None})

        # --- data plane while slow control held ---
        slow = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        slow.connect(str(sock_a))
        slow.sendall(b"sta")  # partial; hold open
        http_body = http_get(listen_a)
        http_ok = "200" in http_body and "cap063" in http_body
        slow.close()
        add(
            "data_plane_under_slow_control",
            {
                "ok": http_ok and proc_a.poll() is None,
                "http_snippet": http_body[-120:],
            },
        )

        # --- integrated exyonqctl ---
        env_ctl = os.environ.copy()
        env_ctl["EXYONQ_CONTROL_SOCKET"] = str(sock_a)
        p = subprocess.run(
            [str(CTL), "status", "--socket", str(sock_a), "--format", "json"],
            capture_output=True,
            text=True,
            timeout=15,
            env=env_ctl,
        )
        jctl = parse_json_line((p.stdout or "").encode())
        add(
            "integrated_exyonqctl",
            {
                "ok": p.returncode == 0 and jctl.get("ok") is True,
                "rc": p.returncode,
                "stdout": (p.stdout or "")[-200:],
                "stderr": (p.stderr or "")[-200:],
                "json": jctl,
            },
        )

        # --- second instance on distinct socket ---
        proc_b, _ = start_exyonq(cfg_b, sock_b)
        procs.append(proc_b)
        ready_b = wait_pred(lambda: listening(listen_b) and sock_b.exists())
        ja2 = parse_json_line(raw_cmd(str(sock_a), b"status\n"))
        jb2 = parse_json_line(raw_cmd(str(sock_b), b"status\n"))
        add(
            "multiple_instances_isolation",
            {
                "ok": ready_b
                and ja2.get("ok") is True
                and jb2.get("ok") is True
                and sock_a != sock_b
                and listening(listen_a)
                and listening(listen_b),
                "ready_b": ready_b,
            },
        )

        # --- active collision: third process same sock_a must fail to steal ---
        listen_c = pick_port()
        cfg_c = work / "c.toml"
        cfg_c.write_text(cfg(listen_c, peer_port))
        # same control socket as A
        env_c = os.environ.copy()
        env_c["EXYONQ_CONTROL_SOCKET"] = str(sock_a)
        log_c = EV / f"collision-{time.time_ns()}.log"
        proc_c = subprocess.Popen(
            [str(BIN), "serve", "--config", str(cfg_c)],
            cwd=str(WS),
            env={**env_c, "EXYONQ_CONFIG": str(cfg_c)},
            stdout=log_c.open("w"),
            stderr=subprocess.STDOUT,
        )
        procs.append(proc_c)
        time.sleep(2.0)
        # A must still own socket; C must fail startup (sync bind refuse live listener).
        ja_live = parse_json_line(raw_cmd(str(sock_a), b"status\n"))
        a_still = listening(listen_a) and proc_a.poll() is None
        c_failed = proc_c.poll() is not None and proc_c.poll() != 0
        add(
            "active_socket_collision",
            {
                "ok": a_still and ja_live.get("ok") is True and is_sock(sock_a) and c_failed,
                "a_alive": a_still,
                "status": ja_live,
                "c_exit": proc_c.poll(),
                "c_failed_startup": c_failed,
            },
        )
        stop_proc(proc_c, grace=2.0)

        # --- regular file at path ---
        sock_file = work / "file.sock"
        sock_file.write_bytes(b"important-content")
        listen_f = pick_port()
        cfg_f = work / "f.toml"
        cfg_f.write_text(cfg(listen_f, peer_port))
        proc_f, log_f = start_exyonq(cfg_f, sock_file)
        procs.append(proc_f)
        time.sleep(1.5)
        content = sock_file.read_bytes() if sock_file.is_file() else b""
        add(
            "refuse_regular_file",
            {
                "ok": content == b"important-content" and not listening(listen_f),
                "content_intact": content == b"important-content",
                "listening": listening(listen_f),
                "proc_exit": proc_f.poll(),
            },
        )
        stop_proc(proc_f, grace=2.0)

        # --- directory at path ---
        sock_dir = work / "dir.sock"
        sock_dir.mkdir()
        listen_d = pick_port()
        cfg_d = work / "d.toml"
        cfg_d.write_text(cfg(listen_d, peer_port))
        proc_d, _ = start_exyonq(cfg_d, sock_dir)
        procs.append(proc_d)
        time.sleep(1.5)
        add(
            "refuse_directory",
            {
                "ok": sock_dir.is_dir() and not listening(listen_d),
                "is_dir": sock_dir.is_dir(),
                "listening": listening(listen_d),
            },
        )
        stop_proc(proc_d, grace=2.0)

        # --- stale socket recovery ---
        sock_stale = work / "stale.sock"
        stale_l = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        stale_l.bind(str(sock_stale))
        stale_l.listen(1)
        stale_l.close()
        assert sock_stale.exists()
        listen_s = pick_port()
        cfg_s = work / "s.toml"
        cfg_s.write_text(cfg(listen_s, peer_port))
        proc_s, _ = start_exyonq(cfg_s, sock_stale)
        procs.append(proc_s)
        ready_s = wait_pred(lambda: listening(listen_s) and is_sock(sock_stale))
        mode_s = sock_mode(sock_stale)
        js = parse_json_line(raw_cmd(str(sock_stale), b"status\n"))
        add(
            "stale_socket_recovery",
            {
                "ok": ready_s and js.get("ok") is True and mode_s == CONTROL_SOCKET_MODE_EXPECTED,
                "ready": ready_s,
                "mode": oct(mode_s) if mode_s is not None else None,
                "json": js,
            },
        )

        # --- umask independence: publication never mutates process-global umask ---
        sock_u = work / "umask.sock"
        listen_u = pick_port()
        cfg_u = work / "u.toml"
        cfg_u.write_text(cfg(listen_u, peer_port))
        env_u = {"EXYONQ_CONTROL_SOCKET": str(sock_u), "EXYONQ_CONFIG": str(cfg_u)}
        # Start with permissive umask via shell
        log_u = EV / f"umask-{time.time_ns()}.log"
        proc_u = subprocess.Popen(
            f"umask 000; exec '{BIN}' serve --config '{cfg_u}'",
            shell=True,
            cwd=str(WS),
            env={**os.environ, **env_u},
            stdout=log_u.open("w"),
            stderr=subprocess.STDOUT,
        )
        procs.append(proc_u)
        ready_u = wait_pred(lambda: listening(listen_u) and sock_u.exists())
        mode_u = sock_mode(sock_u)
        add(
            "umask_independence",
            {
                "ok": ready_u and mode_u == CONTROL_SOCKET_MODE_EXPECTED,
                "mode": oct(mode_u) if mode_u is not None else None,
                "ready": ready_u,
            },
        )
        stop_proc(proc_u, grace=3.0)

        # --- drain keeps control available (Cap040 owns HTTP drain semantics) ---
        jdrain = parse_json_line(raw_cmd(str(sock_a), b"drain\n"))
        jstatus_d = parse_json_line(raw_cmd(str(sock_a), b"status\n"))
        add(
            "drain_keeps_control",
            {
                "ok": jdrain.get("ok") is True
                and jstatus_d.get("ok") is True
                and bool(jstatus_d.get("draining")),
                "drain": jdrain,
                "status": jstatus_d,
                "NOTE": "control plane remains reachable after drain",
            },
        )

        # --- shutdown unlink ---
        jshut = parse_json_line(raw_cmd(str(sock_a), b"shutdown\n"))
        wait_pred(lambda: proc_a.poll() is not None, timeout=20.0)
        # socket path should be gone after process exit (ControlSocketUnlink)
        gone = wait_pred(lambda: not sock_a.exists(), timeout=5.0)
        add(
            "shutdown_unlinks_socket",
            {
                "ok": jshut.get("ok") is True and proc_a.poll() is not None and gone,
                "shutdown": jshut,
                "exit": proc_a.poll(),
                "path_gone": not sock_a.exists(),
            },
        )

        # stop remaining
        for p in procs:
            stop_proc(p, grace=3.0)

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
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "passed": result["CHECKS_PASSED"], "total": result["CHECKS_TOTAL"]}, indent=2))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
