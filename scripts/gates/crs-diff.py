#!/usr/bin/env python3
"""Generic CRS differential: same rules, same requests, ExyonQ vs ModSecurity v3.

Both engines load the exact same `SecRule` files. The origin is real; the only
difference under test is the WAF decision. Compares the HTTP status (and whether
the origin was reached) per case.

Usage:
  crs-diff.py --rules FILE [--corpus FILE.json] [--builtins on|off]
"""
import argparse
import json
import os
import shutil
import socket
import socketserver
import subprocess
import sys
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORK = Path("/tmp/crs-diff")
ORIGIN_PORT = 19130
EXY_PORT = 18131
REF_PORT = 18141
CONTAINER = "crs-diff-ref"

ORIGIN_HITS = 0
ORIGIN_LOCK = threading.Lock()


def origin_server():
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            global ORIGIN_HITS
            try:
                data = b""
                while b"\r\n\r\n" not in data:
                    chunk = self.request.recv(65536)
                    if not chunk:
                        break
                    data += chunk
                head, rest = data.split(b"\r\n\r\n", 1)
                headers = {}
                for line in head.split(b"\r\n")[1:]:
                    name, _, value = line.decode("latin1").partition(":")
                    headers[name.strip().lower()] = value.strip()
                if headers.get("transfer-encoding", "").lower() == "chunked":
                    buf = rest
                    while True:
                        while b"\r\n" not in buf:
                            buf += self.request.recv(65536)
                        line, buf = buf.split(b"\r\n", 1)
                        if int(line, 16) == 0:
                            break
                else:
                    need = int(headers.get("content-length", "0"))
                    while len(rest) < need:
                        rest += self.request.recv(65536)
                with ORIGIN_LOCK:
                    ORIGIN_HITS += 1
                self.request.sendall(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                )
            except OSError:
                pass

    class Origin(socketserver.ThreadingTCPServer):
        daemon_threads = True
        allow_reuse_address = True

    server = Origin(("0.0.0.0", ORIGIN_PORT), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def require_free_port(port: int) -> None:
    """A stale ExyonQ from a crashed run would silently serve the old config."""
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            pass
    except OSError:
        return
    # A crashed run leaves the old server holding the port; the harness owns it.
    subprocess.run(["pkill", "-f", f"exyonq serve -c"], check=False)
    time.sleep(0.5)


def wait_port(port: int, timeout: float = 40.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def wait_http(port: int, timeout: float = 90.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            # CRS cold start on a slow host can exceed a couple of seconds.
            sock = socket.create_connection(("127.0.0.1", port), timeout=20)
            sock.sendall(b"GET / HTTP/1.1\r\nHost: crs.test\r\nConnection: close\r\n\r\n")
            data = sock.recv(64)
            sock.close()
            if data.startswith(b"HTTP/"):
                return
        except OSError:
            pass
        time.sleep(0.3)
    raise RuntimeError(f"port {port} did not answer HTTP")


def send(port: int, case: dict) -> tuple[int, int]:
    for attempt in range(3):
        try:
            return _send_once(port, case)
        except OSError:
            if attempt == 2:
                raise
            time.sleep(0.5)
    raise RuntimeError("unreachable")


def _send_once(port: int, case: dict) -> tuple[int, int]:
    global ORIGIN_HITS
    method = case.get("method", "GET")
    path = case.get("path", "/")
    body = case.get("body", "").encode("latin1")
    headers = case.get("headers", {})
    lines = [f"{method} {path} HTTP/1.1"]
    if not any(name.lower() == "host" for name in headers):
        lines.append("Host: crs.test")
    lines.append("Connection: close")
    for name, value in headers.items():
        lines.append(f"{name}: {value}")
    if body or method in ("POST", "PUT", "PATCH"):
        lines.append(f"Content-Length: {len(body)}")
    head = ("\r\n".join(lines) + "\r\n\r\n").encode("latin1")
    with ORIGIN_LOCK:
        ORIGIN_HITS = 0
    sock = socket.create_connection(("127.0.0.1", port), timeout=20)
    try:
        sock.sendall(head + body)
        data = b""
        while b"\r\n" not in data:
            chunk = sock.recv(65536)
            if not chunk:
                break
            data += chunk
        if not data:
            return 0, 0
        status = int(data.split(b"\r\n")[0].split(b" ")[1])
        return status, ORIGIN_HITS
    finally:
        sock.close()


def exy_config(rules: Path, builtins: str, threshold: int) -> str:
    toggles = ""
    if builtins == "off":
        toggles = """
[waf.ruleset.builtins]
sql_injection = "off"
xss = "off"
path_traversal = "off"
command_injection = "off"
header_anomaly = "off"
bot_ua = "off"
ip_filter = { enabled = false }
"""
    return f"""config_version = 1
[waf]
enabled = true
mode = "block"
max_inspection_body_bytes = 1048576
[[waf.ruleset]]
id = "diff"
rules_file = "{rules}"
inbound_threshold = {threshold}
{toggles}
[[server]]
listen = "127.0.0.1:{EXY_PORT}"
routes = ["all"]
[[route]]
name = "all"
match = {{ path = "/" }}
upstream = "origin"
[[upstream]]
name = "origin"
target = "http://127.0.0.1:{ORIGIN_PORT}"
"""


def start_exy(rules: Path, builtins: str, threshold: int) -> subprocess.Popen:
    (WORK / "exy.toml").write_text(exy_config(rules, builtins, threshold))
    require_free_port(EXY_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={**os.environ, "EXYONQ_CONTROL_SOCKET": "/tmp/crs-diff.sock"},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy


def start_reference(rules: Path, crs_dir: Path | None = None) -> None:
    subprocess.run(
        ["docker", "rm", "-f", CONTAINER],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    include = "/ref-main.conf" if crs_dir else str(rules)
    (WORK / "setup.conf.template").write_text(
        "Include /etc/modsecurity.d/modsecurity.conf\n"
        f"Include {include}\n"
    )
    if sys.platform == "darwin":
        # Docker Desktop: containers reach the host origin through the
        # host-gateway alias and the published port reaches the Mac.
        net_args = [
            "--add-host=host.docker.internal:host-gateway",
            "-p", f"{REF_PORT}:8080",
            "-e", f"BACKEND=http://host.docker.internal:{ORIGIN_PORT}",
        ]
    else:
        # Native Linux: firewalls (ufw INPUT DROP) commonly block container ->
        # host traffic on the docker0 gateway, so run the reference on the host
        # network; it then reaches the origin over loopback and listens on
        # REF_PORT directly.
        net_args = [
            "--network", "host",
            "-e", f"PORT={REF_PORT}",
            "-e", "SSL_PORT=18443",
            "-e", f"BACKEND=http://127.0.0.1:{ORIGIN_PORT}",
        ]
    subprocess.run(
        [
            "docker", "run", "-d", "--rm", "--name", CONTAINER,
            *net_args,
            "-e", "MODSEC_RULE_ENGINE=On",
            "-e", "MODSEC_REQ_BODY_LIMIT=1048576",
            "-e", "PROXY_HOST_HEADER=crs.test",
            *(["-v", f"{crs_dir}:/crs:ro", "-v", f"{WORK / 'ref-main.conf'}:/ref-main.conf:ro"] if crs_dir else ["-v", f"{rules}:{rules}:ro"]),
            "-v", f"{WORK / 'setup.conf.template'}:/etc/nginx/templates/modsecurity.d/setup.conf.template:ro",
            "owasp/modsecurity-crs:nginx-alpine",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    wait_http(REF_PORT)


def stop_reference() -> None:
    subprocess.run(
        ["docker", "rm", "-f", CONTAINER],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )


def run_concurrent(case: dict) -> int:
    """Per-request state isolation: N parallel requests, each with its own token."""
    count = int(case["concurrent"])
    results = {}

    def worker(index: int) -> None:
        token = f"tok-{index}-{os.getpid()}"
        req = dict(case)
        req["path"] = case["path"].replace("{token}", token)
        req["headers"] = {**case.get("headers", {}), "X-Token": token}
        results[index] = send(EXY_PORT, req)[0]

    threads = [threading.Thread(target=worker, args=(i,)) for i in range(count)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    ok = len(results) == count and all(status == case["expect"] for status in results.values())
    print(
        f"{case.get('name', 'concurrent')}: {'PASS' if ok else 'FAIL'} "
        f"concurrent={count} statuses={sorted(set(results.values()))} "
        f"expect={case['expect']}"
    )
    return 0 if ok else 1


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rules")
    parser.add_argument("--crs", help="CRS checkout: rules/ + crs-setup.conf.example")
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--builtins", choices=["on", "off"], default="off")
    parser.add_argument("--threshold", type=int, default=5)
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    crs_dir = Path(args.crs).resolve() if args.crs else None
    if crs_dir:
        rules = WORK / "exy-main.conf"
    else:
        rules = Path(args.rules).resolve()
    corpus = json.loads(Path(args.corpus).read_text())
    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    if crs_dir:
        (WORK / "exy-main.conf").write_text(
            f"Include {crs_dir}/crs-setup.conf.example\nInclude {crs_dir}/rules/*.conf\n"
        )
        (WORK / "ref-main.conf").write_text(
            "Include /crs/crs-setup.conf.example\nInclude /crs/rules/*.conf\n"
        )

    origin = origin_server()
    exy = start_exy(rules, args.builtins, args.threshold)
    start_reference(rules, crs_dir)
    failures = 0
    try:
        for case in corpus:
            if case.get("concurrent"):
                failures += run_concurrent(case)
                continue
            exy_status, exy_hits = send(EXY_PORT, case)
            ref_status, ref_hits = send(REF_PORT, case)
            same = exy_status == ref_status and exy_hits == ref_hits
            failures += 0 if same else 1
            print(
                f"{case.get('name', case.get('path'))}: "
                f"{'PASS' if same else 'FAIL'} "
                f"exyonq={exy_status}/hits={exy_hits} "
                f"modsecurity={ref_status}/hits={ref_hits}"
            )
    finally:
        exy.terminate()
        try:
            exy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            exy.kill()
        origin.shutdown()
        if not args.keep:
            stop_reference()
    print(f"failures={failures}/{len(corpus)}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
