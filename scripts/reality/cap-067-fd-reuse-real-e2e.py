#!/usr/bin/env python3
"""Cap067 FD-reuse REAL product E2E (Linux process + real FS + real sockets).

Not library tests. TARGET_PATH is kernel sendfile/sendfile64 on the live process.

Do NOT enable [modules.metrics]: ServerState.modules_enabled() forces Hyper for
all prefixed TCP dispatch and bypasses Cap067. After LA-CAP054-008, site GET
/metrics is also not OpenMetrics. Proof is strace + HTTP body oracles.

ZERO_FAKE: real files, real sockets, real exyonq binary. No mock FS / synthetic oracle.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import socket
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH = os.environ.get("ARCH_LABEL", "unknown")
HOST = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(BIN.parent / "exyonqctl")))

RESULTS: list[dict] = []
HOT_SIZE = 64 * 1024


def record(name: str, status: str, **extra) -> None:
    row = {"scenario": name, "status": status, **extra}
    RESULTS.append(row)
    print(f"[{status}] {name} {extra}", flush=True)


def sha256(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_port(port: int, timeout: float = 60.0) -> bool:
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.3):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def curl(url: str, *, method: str = "GET", headers: list[str] | None = None,
         timeout: int = 30, path_as_is: bool = False) -> tuple[int, str, bytes, str]:
    tag = f"{time.time_ns()}"
    body_p = EV / "c-{tag}.body"
    hdr_p = EV / "c-{tag}.hdr"
    cmd = ["curl", "-sS", "--http1.1", "--max-time", str(timeout), "-D", str(hdr_p)]
    if path_as_is:
        cmd.append("--path-as-is")
    if method.upper() == "HEAD":
        cmd.append("--head")
    else:
        cmd.extend(["-o", str(body_p), "-X", method])
    for h in headers or []:
        cmd.extend(["-H", h])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = b"" if method.upper() == "HEAD" else (body_p.read_bytes() if body_p.is_file() else b"")
    hdr = hdr_p.read_text(errors="replace") if hdr_p.is_file() else ""
    status = ""
    for line in hdr.splitlines():
        if line.startswith("HTTP/"):
            status = line.strip()
    return proc.returncode, status, body, hdr


def http_status_code(status_line: str) -> str:
    parts = status_line.split()
    return parts[1] if len(parts) >= 2 else ""


def write_config(path: Path, port: int, root: Path) -> None:
    # No [modules.metrics]: that sets modules_enabled and forces Hyper, skipping Cap067.
    path.write_text(
        f"""config_version = 1

[logging]
level = "info"
format = "text"

[logging.console]
enabled = true
stream = "stdout"

[logging.access]
enabled = false

[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
index = "index.html"
""",
        encoding="utf-8",
    )


def fd_count(pid: int) -> int:
    fd_dir = Path(f"/proc/{pid}/fd")
    try:
        return len(list(fd_dir.iterdir()))
    except OSError:
        return -1


def read_http11(sock: socket.socket) -> tuple[int, bytes, str]:
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(8192)
        if not chunk:
            break
        buf += chunk
    if b"\r\n\r\n" not in buf:
        return 0, buf, ""
    raw_hdr, rest = buf.split(b"\r\n\r\n", 1)
    hdr = raw_hdr.decode("latin1", errors="replace")
    status_line = hdr.split("\r\n", 1)[0]
    clen = None
    for line in hdr.split("\r\n"):
        if line.lower().startswith("content-length:"):
            clen = int(line.split(":", 1)[1].strip())
    body = rest
    if clen is not None:
        while len(body) < clen:
            chunk = sock.recv(65536)
            if not chunk:
                break
            body += chunk
        body = body[:clen]
    code = 0
    parts = status_line.split()
    if len(parts) >= 2 and parts[1].isdigit():
        code = int(parts[1])
    return code, body, hdr


def keepalive_gets(port: int, path: str, n: int) -> list[tuple[int, bytes]]:
    out: list[tuple[int, bytes]] = []
    with socket.create_connection(("127.0.0.1", port), timeout=15) as sock:
        sock.settimeout(15)
        req = f"GET {path} HTTP/1.1\r\nHost: t\r\nConnection: keep-alive\r\n\r\n".encode()
        for _ in range(n):
            sock.sendall(req)
            code, body, _ = read_http11(sock)
            out.append((code, body))
    return out


def count_strace_from(prefix: Path, offset: int = 0) -> dict[str, int]:
    keys = ("openat2", "newfstatat", "close", "sendfile", "sendfile64", "fstatat")
    counts = {k: 0 for k in keys}
    for p in sorted(prefix.parent.glob(prefix.name + "*")):
        try:
            data = p.read_bytes()
        except OSError:
            continue
        text = (data[offset:] if p == prefix else data).decode("utf-8", errors="replace")
        for k in keys:
            counts[k] += len(re.findall(rf"\b{k}\(", text))
    return counts


def start_strace(pid: int, prefix: Path) -> subprocess.Popen | None:
    log = open(EV / "strace-attach.err", "w", encoding="utf-8")
    proc = subprocess.Popen(
        [
            "strace", "-f", "-s", "0",
            "-e", "trace=openat2,newfstatat,close,sendfile,sendfile64",
            "-o", str(prefix),
            "-p", str(pid),
        ],
        stdout=log,
        stderr=subprocess.STDOUT,
    )
    time.sleep(0.4)
    if proc.poll() is not None:
        log.close()
        return None
    return proc


def serve_under_strace(cfg: Path, sock_path: Path, log_path: Path, prefix: Path) -> subprocess.Popen:
    """Yama often blocks strace -p; wrapping the child still traces the real binary."""
    if sock_path.exists():
        sock_path.unlink()
    log = open(log_path, "w", encoding="utf-8")
    env = os.environ.copy()
    env.pop("EXYONQ_EPOLL_STATIC", None)
    env.pop("EXYONQ_EPOLL_SENDFILE", None)
    env["EXYONQ_CONTROL_SOCKET"] = str(sock_path)
    env["EXYONQ_CONFIG"] = str(cfg)
    return subprocess.Popen(
        [
            "strace", "-f", "-s", "0",
            "-e", "trace=openat2,newfstatat,close,sendfile,sendfile64",
            "-o", str(prefix),
            "--", str(BIN), "serve", "--config", str(cfg),
        ],
        stdout=log,
        stderr=subprocess.STDOUT,
        env=env,
        cwd=str(WS),
    )


def stop_strace(proc: subprocess.Popen | None) -> None:
    if proc is None:
        return
    proc.send_signal(signal.SIGINT)
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=3)


def start_serve(cfg: Path, sock_path: Path, log_path: Path) -> subprocess.Popen:
    if sock_path.exists():
        sock_path.unlink()
    log = open(log_path, "w", encoding="utf-8")
    env = os.environ.copy()
    env.pop("EXYONQ_EPOLL_STATIC", None)
    env.pop("EXYONQ_EPOLL_SENDFILE", None)
    env["EXYONQ_CONTROL_SOCKET"] = str(sock_path)
    env["EXYONQ_CONFIG"] = str(cfg)
    return subprocess.Popen(
        [str(BIN), "serve", "--config", str(cfg)],
        stdout=log,
        stderr=subprocess.STDOUT,
        env=env,
        cwd=str(WS),
    )


def stop_serve(proc: subprocess.Popen) -> int | None:
    if proc.poll() is None:
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
    return proc.returncode


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    if os.uname().sysname != "Linux":
        record("platform", "FAIL", detail="Cap067 requires Linux")
        return finish(1)
    if not BIN.is_file():
        record("binary", "FAIL", path=str(BIN))
        return finish(1)

    root = EV / "www"
    root.mkdir(parents=True, exist_ok=True)
    outside = EV / "outside-secret.txt"
    outside.write_bytes(b"OUTSIDE-ROOT-SECRET-CAP067-FD")
    hot = b"HOT-FILE-V1" + bytes([0x41]) * (HOT_SIZE - 11)
    files_written = {
        "hot.bin": hot,
        "index.html": b"<html>cap067-fd</html>",
    }
    for name, data in files_written.items():
        (root / name).write_bytes(data)
    escape = root / "escape.link"
    if escape.exists() or escape.is_symlink():
        escape.unlink()
    escape.symlink_to(outside)

    port = pick_port()
    cfg = EV / "exyonq.toml"
    write_config(cfg, port, root)
    sock_path = EV / "control.sock"
    proc = start_serve(cfg, sock_path, EV / "exyonq.log")
    bin_sha = hashlib.sha256(BIN.read_bytes()).hexdigest()
    try:
        if not wait_port(port):
            tail = (EV / "exyonq.log").read_text(errors="replace")[-2500:]
            record("listen", "FAIL", log_tail=tail)
            return finish(1)
        record("listen", "PASS", port=port, pid=proc.pid, binary_sha256=bin_sha)
        fd_before = fd_count(proc.pid)
        record("fd_count_before", "PASS" if fd_before >= 0 else "FAIL", count=fd_before)

        base = f"http://127.0.0.1:{port}"

        # Tiny-header keepalive GET: proves real static bytes without metrics module.
        first = keepalive_gets(port, "/hot.bin", 1)
        first_ok = len(first) == 1 and first[0][0] == 200 and first[0][1] == hot
        record(
            "repeated_static_get_hit",
            "PASS" if first_ok else "FAIL",
            http_status=first[0][0] if first else None,
            body_sha=sha256(first[0][1]) if first else "",
            expect_sha=sha256(hot),
            body_len=len(first[0][1]) if first else 0,
        )
        if not first_ok:
            return finish(1)

        ka = keepalive_gets(port, "/hot.bin", 8)
        ka_ok = all(c == 200 and b == hot for c, b in ka) and len(ka) == 8
        record(
            "keepalive",
            "PASS" if ka_ok else "FAIL",
            n=len(ka),
            first_code=ka[0][0] if ka else None,
        )
        if not ka_ok:
            return finish(1)

        n_trace = 20
        strace_prefix = EV / "strace.out"
        strace_proc = start_strace(proc.pid, strace_prefix)
        strace_mode = "attach"
        strace_err = ""
        geo_hot = hot
        geo_child: subprocess.Popen | None = None
        trace_offset = 0
        if strace_proc is None:
            strace_err = (EV / "strace-attach.err").read_text(errors="replace")[-800:]
            strace_mode = "spawn_child"
            geo_port = pick_port()
            geo_cfg = EV / "exyonq-geo.toml"
            geo_root = EV / "www-geo"
            geo_root.mkdir(parents=True, exist_ok=True)
            (geo_root / "hot.bin").write_bytes(hot)
            write_config(geo_cfg, geo_port, geo_root)
            geo_sock = EV / "control-geo.sock"
            geo_child = serve_under_strace(geo_cfg, geo_sock, EV / "exyonq-geo.log", strace_prefix)
            if not wait_port(geo_port):
                tail = (EV / "exyonq-geo.log").read_text(errors="replace")[-1500:]
                record("fd_reuse_syscalls", "FAIL", detail="strace-wrapped serve failed to listen", log_tail=tail, attach_err=strace_err)
                return finish(1)
            _ = keepalive_gets(geo_port, "/hot.bin", 2)
            time.sleep(0.2)
            # Count only the subsequent burst (startup/warmup opens excluded).
            if strace_prefix.is_file():
                trace_offset = strace_prefix.stat().st_size
            traced = keepalive_gets(geo_port, "/hot.bin", n_trace)
            time.sleep(0.3)
            stop_strace(geo_child)
        else:
            traced = keepalive_gets(port, "/hot.bin", n_trace)
            time.sleep(0.3)
            stop_strace(strace_proc)

        sc = count_strace_from(strace_prefix, trace_offset)
        (EV / "syscall_counts.json").write_text(
            json.dumps(
                {
                    "n": n_trace,
                    "counts": sc,
                    "strace_mode": strace_mode,
                    "trace_offset": trace_offset,
                    "attach_err": strace_err,
                },
                indent=2,
            )
            + "\n"
        )
        traced_ok = all(c == 200 and b == geo_hot for c, b in traced) and len(traced) == n_trace
        sendfile_n = sc["sendfile"] + sc["sendfile64"]
        openat2_per = sc["openat2"] / n_trace if n_trace else -1
        newfstatat_per = sc["newfstatat"] / n_trace if n_trace else -1
        close_per = sc["close"] / n_trace if n_trace else -1
        reuse_ok = traced_ok and sendfile_n >= n_trace and openat2_per < 0.25
        record(
            "fd_reuse_syscalls",
            "PASS" if reuse_ok else "FAIL",
            openat2_per_req=round(openat2_per, 4),
            newfstatat_per_req=round(newfstatat_per, 4),
            close_per_req=round(close_per, 4),
            sendfile_total=sendfile_n,
            n=n_trace,
            strace_mode=strace_mode,
            attach_err=strace_err or None,
            SENDFILE_FAST_PATH_EXECUTED="YES" if sendfile_n >= n_trace else "NO",
            ARM64_FD_CACHE_HIT_BEHAVIOR="HIT_PATH_NO_OPENAT2_PER_REQ" if reuse_ok else "NOT_PROVEN",
        )
        if not reuse_ok:
            return finish(1)

        fd_peak = fd_count(proc.pid)
        record("fd_count_peak", "PASS" if fd_peak >= 0 else "FAIL", count=fd_peak)

        tmp = root / "hot.bin.tmp"
        new_body = b"HOT-FILE-V2" + bytes([0x42]) * (HOT_SIZE - 11)
        tmp.write_bytes(new_body)
        os.rename(tmp, root / "hot.bin")
        renamed = keepalive_gets(port, "/hot.bin", 1)
        rename_ok = len(renamed) == 1 and renamed[0][0] == 200 and renamed[0][1] == new_body
        record(
            "atomic_rename",
            "PASS" if rename_ok else "FAIL",
            http_status=renamed[0][0] if renamed else None,
            body_sha=sha256(renamed[0][1]) if renamed else "",
            expect_sha=sha256(new_body),
        )
        if not rename_ok:
            return finish(1)

        (root / "hot.bin").unlink()
        deleted = keepalive_gets(port, "/hot.bin", 1)
        del_ok = bool(deleted) and deleted[0][0] == 404
        rec_body = b"HOT-FILE-V3" + bytes([0x43]) * (HOT_SIZE - 11)
        (root / "hot.bin").write_bytes(rec_body)
        recreated = keepalive_gets(port, "/hot.bin", 1)
        rec_ok = len(recreated) == 1 and recreated[0][0] == 200 and recreated[0][1] == rec_body
        record(
            "delete_recreate",
            "PASS" if del_ok and rec_ok else "FAIL",
            delete_status=deleted[0][0] if deleted else None,
            recreate_status=recreated[0][0] if recreated else None,
            recreate_sha=sha256(recreated[0][1]) if recreated else "",
        )
        if not (del_ok and rec_ok):
            return finish(1)

        env = os.environ.copy()
        env["EXYONQ_CONTROL_SOCKET"] = str(sock_path)
        # Cap048 honesty: live reload publishes the daemon-bound path only.
        env["EXYONQ_CONFIG"] = str(cfg.resolve())
        if CTL.is_file() and sock_path.exists():
            cfg.write_text(cfg.read_text(encoding="utf-8") + "\n# reload-mark\n", encoding="utf-8")
            rr = subprocess.run(
                [
                    str(CTL),
                    "reload",
                    "--config",
                    str(cfg.resolve()),
                    "--socket",
                    str(sock_path),
                ],
                capture_output=True,
                text=True,
                env=env,
                timeout=30,
            )
            (EV / "reload.stdout").write_text(rr.stdout)
            (EV / "reload.stderr").write_text(rr.stderr)
            after_reload = keepalive_gets(port, "/hot.bin", 1)
            log_txt = (EV / "exyonq.log").read_text(errors="replace")
            gen_hits = re.findall(r"generation=(\d+)", log_txt)
            reload_ok = (
                rr.returncode == 0
                and after_reload
                and after_reload[0][0] == 200
                and after_reload[0][1] == rec_body
            )
            record(
                "reload_generation",
                "PASS" if reload_ok else "FAIL",
                ctl_rc=rr.returncode,
                http_status=after_reload[0][0] if after_reload else None,
                generations_logged=gen_hits,
                stderr_tail=rr.stderr[-500:],
            )
            if not reload_ok:
                return finish(1)
        else:
            record(
                "reload_generation",
                "NOT_SUPPORTED_ON_THIS_PATH",
                ctl_exists=CTL.is_file(),
                sock_exists=sock_path.exists(),
            )

        secret = b"OUTSIDE-ROOT-SECRET-CAP067-FD"
        trav_ok = True
        trav_codes = []
        for url, pais in [
            (f"{base}/../outside-secret.txt", False),
            (f"http://127.0.0.1:{port}/%2e%2e/outside-secret.txt", True),
            (f"{base}/hot.bin/../../outside-secret.txt", True),
        ]:
            rc, st, body, _ = curl(url, path_as_is=pais)
            code = http_status_code(st)
            trav_codes.append(code)
            if code == "200" and (body == secret or secret in body):
                trav_ok = False
        record("path_traversal", "PASS" if trav_ok else "FAIL", codes=trav_codes)

        rc, st, body, _ = curl(f"{base}/escape.link")
        code = http_status_code(st)
        symlink_ok = body != secret and code != "200"
        record("symlink_containment", "PASS" if symlink_ok else "FAIL", http_status=st, code=code)

        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=15)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        still = Path(f"/proc/{proc.pid}").exists()
        fd_after = fd_count(proc.pid)
        record(
            "fd_cleanup",
            "PASS" if not still else "FAIL",
            pid_exists=still,
            fd_count_after=fd_after,
            exit_code=proc.returncode,
        )

        fails = sum(1 for r in RESULTS if r["status"] == "FAIL")
        return finish(1 if fails else 0)
    finally:
        stop_serve(proc)


def finish(code: int) -> int:
    overall = "PASS" if code == 0 else "FAIL"
    reuse = next((r for r in RESULTS if r["scenario"] == "fd_reuse_syscalls"), {})
    peak = next((r for r in RESULTS if r["scenario"] == "fd_count_peak"), {})
    before = next((r for r in RESULTS if r["scenario"] == "fd_count_before"), {})
    cleanup = next((r for r in RESULTS if r["scenario"] == "fd_cleanup"), {})
    payload = {
        "CAPABILITY_ID": "067",
        "FEATURE_ID": "linux-epoll-sendfile-fd-reuse",
        "ARCH": ARCH,
        "HOST": HOST,
        "HEAD": HEAD,
        "OVERALL": overall,
        "STARTED_UTC": datetime.now(timezone.utc).isoformat(),
        "results": RESULTS,
        "ARM64_OPENAT2_CALLS_PER_REQ": reuse.get("openat2_per_req"),
        "ARM64_NEWFSTATAT_CALLS_PER_REQ": reuse.get("newfstatat_per_req"),
        "ARM64_CLOSE_CALLS_PER_REQ": reuse.get("close_per_req"),
        "ARM64_FD_CACHE_HIT_BEHAVIOR": reuse.get("ARM64_FD_CACHE_HIT_BEHAVIOR"),
        "ARM64_FD_COUNT_BEFORE": before.get("count"),
        "ARM64_PEAK_FD_COUNT": peak.get("count"),
        "ARM64_FD_COUNT_AFTER_SHUTDOWN": cleanup.get("fd_count_after"),
        "SENDFILE_TARGET_PATH_EXECUTED": reuse.get("SENDFILE_FAST_PATH_EXECUTED") == "YES",
        "SENDFILE_ORACLE": "strace_sendfile_syscalls_not_prometheus_complete_total",
        "METRICS_MODULE": "DISABLED_PRESERVES_CAP067_WIRE",
        "ZERO_FAKE": True,
    }
    OUT.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"OVERALL": overall, "OUT": str(OUT)}))
    return code


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as e:
        record("uncaught", "FAIL", error=str(e))
        OUT.write_text(json.dumps({"OVERALL": "FAIL", "results": RESULTS, "error": str(e)}, indent=2) + "\n")
        raise
