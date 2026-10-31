#!/usr/bin/env python3
"""NG-18 / ADR-064 Phase D — differential mirror corpus.

Real origin, real mirror sink, real proxy. Compares the main response and the
sink access log (one mirrored request, method/path) between NGINX and ExyonQ.
"""
import argparse
import hashlib
import http.client
import os
import shutil
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORK = Path("/tmp/ng18-mirror-diff")
ORIGIN_PORT = 19097
SINK_PORT = 19098
NGINX_PORT = 18101
EXY_PORT = 18102
NETWORK = "ng18-mirror-diff"
ORIGIN = "mrd-origin"
SINK = "mrd-sink"
PROXY = "mrd-proxy"


def origin_config() -> str:
    return f"""worker_processes 1; pid /tmp/mrd-origin.pid; error_log /dev/stderr warn;
events {{ worker_connections 128; }}
http {{ access_log off; server {{ listen {ORIGIN_PORT};
  location = /main {{ alias /srv/main; default_type text/plain; }}
}} }}
"""


def sink_config() -> str:
    return f"""worker_processes 1; pid /tmp/mrd-sink.pid; error_log /dev/stderr warn;
events {{ worker_connections 128; }}
http {{ server {{ listen {SINK_PORT};
  access_log /srv/sink.log;
  location / {{ return 204; }}
}} }}
"""


def proxy_config() -> str:
    return f"""worker_processes 1; pid /tmp/mrd-proxy.pid; error_log /dev/stderr warn;
events {{ worker_connections 128; }}
http {{ access_log off; server {{ listen {NGINX_PORT};
  location = /main {{ mirror /mirror; proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; }}
  location = /mirror {{ proxy_pass http://{SINK}:{SINK_PORT}; }}
}} }}
"""


def exy_config() -> str:
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{EXY_PORT}"
routes = ["site", "sink"]
[[route]]
name = "site"
match = {{ path = "/main" }}
upstream = "origin"
[[route]]
name = "sink"
match = {{ path = "/mirror" }}
upstream = "sink"
[[upstream]]
name = "origin"
target = "http://127.0.0.1:{ORIGIN_PORT}"
[[upstream]]
name = "sink"
target = "http://127.0.0.1:{SINK_PORT}"
[modules.mirror.routes."site"]
destinations = ["/mirror"]
request_body = false
"""


def wait_port(port: int, timeout: float = 30.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def fetch(port: int, path: str) -> dict:
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=20)
    try:
        conn.request("GET", path, headers={"Host": "mirror.test", "Connection": "close"})
        resp = conn.getresponse()
        body = resp.read()
        return {
            "status": resp.status,
            "body": hashlib.sha256(body).hexdigest(),
            "length": len(body),
        }
    finally:
        conn.close()


def sink_lines() -> list[str]:
    log = WORK / "sink" / "sink.log"
    if not log.exists():
        return []
    return [line for line in log.read_text(errors="replace").splitlines() if line.strip()]


def start() -> subprocess.Popen:
    shutil.rmtree(WORK, ignore_errors=True)
    (WORK / "srv").mkdir(parents=True)
    (WORK / "sink").mkdir(parents=True)
    (WORK / "srv/main").write_bytes(b"main-body\n")
    (WORK / "origin.conf").write_text(origin_config())
    (WORK / "sink.conf").write_text(sink_config())
    (WORK / "proxy.conf").write_text(proxy_config())
    (WORK / "exy.toml").write_text(exy_config())
    subprocess.run(["docker", "rm", "-f", ORIGIN, SINK, PROXY], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["docker", "network", "create", NETWORK], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for name, port, conf, extra in [
        (ORIGIN, ORIGIN_PORT, "origin.conf", ["-v", f"{WORK / 'srv'}:/srv:ro"]),
        (SINK, SINK_PORT, "sink.conf", ["-v", f"{WORK / 'sink'}:/srv"]),
        (PROXY, NGINX_PORT, "proxy.conf", []),
    ]:
        subprocess.run(
            ["docker", "run", "-d", "--rm", "--name", name, "--network", NETWORK,
             "-p", f"{port}:{port}",
             "-v", f"{WORK / conf}:/etc/nginx/nginx.conf:ro", *extra,
             "nginx:1.27-alpine"],
            check=True, stdout=subprocess.DEVNULL,
        )
    wait_port(ORIGIN_PORT)
    wait_port(SINK_PORT)
    wait_port(NGINX_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={**os.environ, "EXYONQ_CONTROL_SOCKET": "/tmp/ng18-mirror.sock"},
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    exy = start()
    failures = 0
    try:
        results = {}
        for label, port in (("nginx", NGINX_PORT), ("exyonq", EXY_PORT)):
            log = WORK / "sink" / "sink.log"
            log.write_text("")
            main = fetch(port, "/main")
            time.sleep(0.5)
            lines = sink_lines()
            results[label] = {"main": main, "sink": lines}
        nginx = results["nginx"]
        exyonq = results["exyonq"]
        main_same = (
            nginx["main"]["status"] == exyonq["main"]["status"]
            and nginx["main"]["body"] == exyonq["main"]["body"]
        )
        sink_same = len(nginx["sink"]) == len(exyonq["sink"]) == 1
        print(f"main: {'PASS' if main_same else 'FAIL'} nginx={nginx['main']} exyonq={exyonq['main']}")
        print(f"sink: {'PASS' if sink_same else 'FAIL'} nginx={nginx['sink']} exyonq={exyonq['sink']}")
        failures += 0 if main_same else 1
        failures += 0 if sink_same else 1
    finally:
        exy.terminate()
        try:
            exy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            exy.kill()
        if not args.keep:
            subprocess.run(["docker", "rm", "-f", ORIGIN, SINK, PROXY], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
