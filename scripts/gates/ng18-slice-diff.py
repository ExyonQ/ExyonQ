#!/usr/bin/env python3
"""NG-18 / ADR-064 Phase E — differential slice corpus (NGINX vs ExyonQ).

4 MiB upstream, 1 MiB slices. Compares the assembled body and the upstream
Range count on the first (miss) and second (cache hit) request.
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
WORK = Path("/tmp/ng18-slice-diff")
SIZE = 4 * 1024 * 1024
BODY = (b"ng18-slice-segment-body\n" * ((SIZE // 24) + 1))[:SIZE]
BODY += b"\0" * (SIZE - len(BODY))
ORIGIN_PORT = 19090
NGINX_PORT = 18081
EXY_PORT = 18080
NETWORK = "ng18-slice-diff"
ORIGIN = "sld-origin"
PROXY = "sld-proxy"


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
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=30)
    try:
        conn.request("GET", path, headers={"Host": "slice.test", "Connection": "close"})
        resp = conn.getresponse()
        body = resp.read()
        return {
            "status": resp.status,
            "sha": hashlib.sha256(body).hexdigest(),
            "length": len(body),
            "accept_ranges": resp.getheader("accept-ranges"),
        }
    finally:
        conn.close()


def range_lines() -> int:
    log = WORK / "logs/access.log"
    if not log.exists():
        return 0
    return sum(1 for line in log.read_text(errors="replace").splitlines() if line.strip())


def start() -> subprocess.Popen:
    shutil.rmtree(WORK, ignore_errors=True)
    (WORK / "srv").mkdir(parents=True)
    (WORK / "logs").mkdir(parents=True)
    (WORK / "cache").mkdir(parents=True)
    os.chmod(WORK / "cache", 0o777)
    (WORK / "srv/big.bin").write_bytes(BODY)
    (WORK / "origin.conf").write_text(f"""worker_processes 1; pid /tmp/sld-origin.pid; error_log /dev/stderr warn;
events {{ worker_connections 128; }}
http {{
  log_format ranges '$request $http_range';
  access_log /logs/access.log ranges;
  server {{ listen 8080; root /srv; location / {{ }} }}
}}
""")
    (WORK / "proxy.conf").write_text(f"""worker_processes 1; pid /tmp/sld-proxy.pid; error_log /dev/stderr notice;
events {{ worker_connections 128; }}
http {{
  proxy_cache_path /cache levels=1 keys_zone=slicecache:10m inactive=60s;
  server {{ listen {NGINX_PORT};
    location / {{
      slice 1m;
      proxy_cache slicecache;
      proxy_cache_valid 200 206 60s;
      proxy_ignore_headers Cache-Control Expires Set-Cookie;
      proxy_cache_key $uri$slice_range;
      proxy_set_header Range $slice_range;
      proxy_pass http://{ORIGIN}:8080;
    }}
  }}
}}
""")
    (WORK / "exy.toml").write_text(f"""config_version = 1
[logging]
level = "warn"
format = "text"
[[server]]
listen = "127.0.0.1:{EXY_PORT}"
routes = ["api"]
[[route]]
name = "api"
match = {{ path = "/" }}
upstream = "u"
cache = "micro"
[[upstream]]
name = "u"
target = "http://127.0.0.1:{ORIGIN_PORT}"
[[cache_policy]]
name = "micro"
ttl_seconds = 60
max_object_bytes = 8388608
[modules.slice.routes."api"]
size = 1048576
""")
    subprocess.run(["docker", "rm", "-f", ORIGIN, PROXY], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["docker", "network", "create", NETWORK], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", ORIGIN, "--network", NETWORK,
         "-p", f"{ORIGIN_PORT}:8080",
         "-v", f"{WORK / 'srv'}:/srv:ro",
         "-v", f"{WORK / 'logs'}:/logs",
         "-v", f"{WORK / 'origin.conf'}:/etc/nginx/nginx.conf:ro",
         "nginx:1.27-alpine"],
        check=True, stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", PROXY, "--network", NETWORK,
         "-p", f"{NGINX_PORT}:{NGINX_PORT}",
         "-v", f"{WORK / 'proxy.conf'}:/etc/nginx/nginx.conf:ro",
         "-v", f"{WORK / 'cache'}:/cache",
         "nginx:1.27-alpine"],
        check=True, stdout=subprocess.DEVNULL,
    )
    wait_port(ORIGIN_PORT)
    wait_port(NGINX_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={
            **os.environ,
            "EXYONQ_CONTROL_SOCKET": "/tmp/ng18-slice.sock",
            "EXYONQ_CACHE_PURGE_SOCKET": "/tmp/ng18-slice-purge.sock",
        },
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy


def run_side(port: int, label: str) -> dict:
    log = WORK / "logs/access.log"
    log.write_text("")
    time.sleep(0.2)
    first = fetch(port, "/big.bin")
    time.sleep(0.5)
    first_ranges = range_lines()
    second = fetch(port, "/big.bin")
    time.sleep(0.5)
    second_ranges = range_lines() - first_ranges
    return {"first": first, "second": second, "first_ranges": first_ranges, "second_ranges": second_ranges}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    exy = start()
    failures = 0
    try:
        nginx = run_side(NGINX_PORT, "nginx")
        exyonq = run_side(EXY_PORT, "exyonq")
        for side in (nginx, exyonq):
            print(f"{side['first']['sha']} {side['first']['length']} ranges={side['first_ranges']}/{side['second_ranges']}")
        same_body = (
            nginx["first"]["status"] == exyonq["first"]["status"] == 200
            and nginx["first"]["sha"] == exyonq["first"]["sha"]
            and nginx["first"]["length"] == exyonq["first"]["length"] == SIZE
        )
        same_ranges = (
            nginx["first_ranges"] == exyonq["first_ranges"] == 4
            and nginx["second_ranges"] == exyonq["second_ranges"] == 0
        )
        print(f"body: {'PASS' if same_body else 'FAIL'}")
        print(f"ranges: {'PASS' if same_ranges else 'FAIL'} nginx={nginx['first_ranges']}/{nginx['second_ranges']} exyonq={exyonq['first_ranges']}/{exyonq['second_ranges']}")
        failures += 0 if same_body else 1
        failures += 0 if same_ranges else 1
    finally:
        exy.terminate()
        try:
            exy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            exy.kill()
        if not args.keep:
            subprocess.run(["docker", "rm", "-f", ORIGIN, PROXY], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
