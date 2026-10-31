#!/usr/bin/env python3
"""NG-18 / ADR-064 Phase C — differential sub_filter corpus.

Runs the same origin/proxy matrix against real NGINX and ExyonQ and compares
status, body and the headers affected by sub_filter (content-type,
last-modified). Real upstream + real proxy, no mocks.
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
WORK = Path("/tmp/ng18-sub-filter-diff")
ORIGIN_PORT = 19094
NGINX_PORT = 18099
EXY_PORT = 18100
NETWORK = "ng18-sub-filter-diff"
ORIGIN = "sfd-origin"
PROXY = "sfd-proxy"


def body_for(path: str) -> bytes:
    if path.endswith("/large"):
        return b"foo " * 300_000
    if path.endswith("/case"):
        return b"FOO foo\n"
    if path.endswith("/plain"):
        return b"foo foo http://a\n"
    if path.endswith("/lm"):
        return b"foo\n"
    return b"foo foo http://a\n"


def origin_config() -> str:
    locations = []
    for path in [
        "/html",
        "/plain",
        "/case",
        "/large",
        "/lm",
        "/once/html",
        "/once/large",
        "/multi/html",
        "/plain/plain",
        "/lmon/lm",
    ]:
        name = path.strip("/").replace("/", "-") or "root"
        ctype = "text/plain" if path.endswith("plain") else "text/html"
        lm = "add_header Last-Modified 'Wed, 21 Oct 2015 07:28:00 GMT';" if path.endswith("/lm") else ""
        locations.append(
            f"    location = {path} {{ alias /srv/{name}; default_type {ctype}; {lm} }}"
        )
    return f"""worker_processes 1; pid /tmp/sfd-origin.pid; error_log /dev/stderr warn;
events {{ worker_connections 256; }}
http {{ access_log off; server {{ listen {ORIGIN_PORT};
{chr(10).join(locations)}
}} }}
"""


def proxy_config() -> str:
    return f"""worker_processes 1; pid /tmp/sfd-proxy.pid; error_log /dev/stderr warn;
events {{ worker_connections 256; }}
http {{ access_log off; server {{ listen {NGINX_PORT};
  location / {{ proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; sub_filter 'foo' 'bar'; sub_filter_once off; sub_filter_types text/html; }}
  location /once/ {{ proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; sub_filter 'foo' 'bar'; sub_filter_once on; sub_filter_types text/html; }}
  location /multi/ {{ proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; sub_filter 'foo' 'bar'; sub_filter 'http://' 'https://'; sub_filter_once off; sub_filter_types text/html; }}
  location /plain/ {{ proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; sub_filter 'foo' 'bar'; sub_filter_once off; sub_filter_types text/plain; }}
  location /lmon/ {{ proxy_pass http://{ORIGIN}:{ORIGIN_PORT}; sub_filter 'foo' 'bar'; sub_filter_last_modified on; sub_filter_types text/html; }}
}} }}
"""


def exy_config() -> str:
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{EXY_PORT}"
routes = ["base", "once", "multi", "plain", "lmon"]
[[route]]
name = "base"
match = {{ path = "/" }}
upstream = "u"
[[route]]
name = "once"
match = {{ path = "/once/" }}
upstream = "u"
[[route]]
name = "multi"
match = {{ path = "/multi/" }}
upstream = "u"
[[route]]
name = "plain"
match = {{ path = "/plain/" }}
upstream = "u"
[[route]]
name = "lmon"
match = {{ path = "/lmon/" }}
upstream = "u"
[[upstream]]
name = "u"
target = "http://127.0.0.1:{ORIGIN_PORT}"
[modules.sub_filter.routes."base"]
once = false
types = ["text/html"]
[[modules.sub_filter.routes."base".rules]]
pattern = "foo"
replacement = "bar"
[modules.sub_filter.routes."once"]
once = true
types = ["text/html"]
[[modules.sub_filter.routes."once".rules]]
pattern = "foo"
replacement = "bar"
[modules.sub_filter.routes."multi"]
once = false
types = ["text/html"]
[[modules.sub_filter.routes."multi".rules]]
pattern = "foo"
replacement = "bar"
[[modules.sub_filter.routes."multi".rules]]
pattern = "http://"
replacement = "https://"
[modules.sub_filter.routes."plain"]
once = false
types = ["text/plain"]
[[modules.sub_filter.routes."plain".rules]]
pattern = "foo"
replacement = "bar"
[modules.sub_filter.routes."lmon"]
once = false
types = ["text/html"]
last_modified = true
[[modules.sub_filter.routes."lmon".rules]]
pattern = "foo"
replacement = "bar"
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
        conn.request("GET", path, headers={"Host": "sf.test", "Connection": "close"})
        resp = conn.getresponse()
        body = resp.read()
        return {
            "status": resp.status,
            "body": hashlib.sha256(body).hexdigest(),
            "length": len(body),
            "content_type": resp.getheader("content-type"),
            "last_modified": resp.getheader("last-modified"),
            "text": body.decode("utf-8", "replace") if len(body) <= 64 else None,
        }
    finally:
        conn.close()


def start() -> tuple[subprocess.Popen, str, str]:
    shutil.rmtree(WORK, ignore_errors=True)
    (WORK / "srv").mkdir(parents=True)
    for path in [
        "/html",
        "/plain",
        "/case",
        "/large",
        "/lm",
        "/once/html",
        "/once/large",
        "/multi/html",
        "/plain/plain",
        "/lmon/lm",
    ]:
        name = path.strip("/").replace("/", "-") or "root"
        (WORK / "srv" / name).write_bytes(body_for(path))
    (WORK / "origin.conf").write_text(origin_config())
    (WORK / "proxy.conf").write_text(proxy_config())
    (WORK / "exy.toml").write_text(exy_config())
    subprocess.run(["docker", "rm", "-f", ORIGIN, PROXY], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["docker", "network", "create", NETWORK], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", ORIGIN, "--network", NETWORK,
         "-p", f"{ORIGIN_PORT}:{ORIGIN_PORT}",
         "-v", f"{WORK / 'srv'}:/srv:ro",
         "-v", f"{WORK / 'origin.conf'}:/etc/nginx/nginx.conf:ro",
         "nginx:1.27-alpine"],
        check=True, stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["docker", "run", "-d", "--rm", "--name", PROXY, "--network", NETWORK,
         "-p", f"{NGINX_PORT}:{NGINX_PORT}",
         "-v", f"{WORK / 'proxy.conf'}:/etc/nginx/nginx.conf:ro",
         "nginx:1.27-alpine"],
        check=True, stdout=subprocess.DEVNULL,
    )
    wait_port(ORIGIN_PORT)
    wait_port(NGINX_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={**os.environ, "EXYONQ_CONTROL_SOCKET": "/tmp/ng18-sub-filter.sock"},
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy, ORIGIN, PROXY


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    exy, _, _ = start()
    paths = [
        "/html",
        "/plain/plain",
        "/once/html",
        "/multi/html",
        "/case",
        "/large",
        "/lm",
        "/lmon/lm",
    ]
    failures = 0
    try:
        for path in paths:
            nginx = fetch(NGINX_PORT, path)
            exyonq = fetch(EXY_PORT, path)
            same = all(
                nginx[key] == exyonq[key]
                for key in ("status", "body", "content_type", "last_modified")
            )
            print(f"{path}: {'PASS' if same else 'FAIL'}")
            if not same:
                failures += 1
                print(f"  nginx={nginx}")
                print(f"  exyonq={exyonq}")
    finally:
        exy.terminate()
        try:
            exy.wait(timeout=5)
        except subprocess.TimeoutExpired:
            exy.kill()
        if not args.keep:
            subprocess.run(["docker", "rm", "-f", ORIGIN, PROXY], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print(f"rows={len(paths)} failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
