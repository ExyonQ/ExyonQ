#!/usr/bin/env python3
"""Real E2E: proxy_redirect regex with captures through the ExyonQ binary."""
import http.client
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path


def wait_port(port, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def main():
    work = Path(tempfile.mkdtemp(prefix="redirect-regex-e2e-"))
    (work / "upstream.conf").write_text("""
worker_processes 1;
events { worker_connections 128; }
http {
  access_log off;
  server {
    listen 8080;
    location = /redirect { return 302 http://127.0.0.1:19090/target; }
  }
}
""")
    (work / "exyonq.toml").write_text("""
config_version = 1
[logging]
level = "warn"
format = "text"
[[server]]
listen = "127.0.0.1:18080"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/" }
upstream = "u"
[[route.location_rewrite]]
from = "~^http://127.0.0.1:19090/(.*)$"
to = "/$1"
[[upstream]]
name = "u"
target = "http://127.0.0.1:19090"
""")
    exy = None
    cid = None
    try:
        cid = subprocess.check_output(
            [
                "docker", "run", "--rm", "-d", "--name", "redirect-regex-up",
                "-p", "19090:8080",
                "-v", f"{work / 'upstream.conf'}:/etc/nginx/nginx.conf:ro",
                "nginx:1.27-alpine",
            ],
            text=True,
        ).strip()
        wait_port(19090)
        env = dict(os.environ)
        env["EXYONQ_CONTROL_SOCKET"] = "/tmp/exyonq-redirect.sock"
        env["EXYONQ_CACHE_PURGE_SOCKET"] = "/tmp/exyonq-redirect-purge.sock"
        exy = subprocess.Popen(
            [sys.argv[1], "serve", "-c", str(work / "exyonq.toml")],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.STDOUT,
            env=env,
        )
        wait_port(18080)
        conn = http.client.HTTPConnection("127.0.0.1", 18080, timeout=10)
        conn.request("GET", "/redirect", headers={"Host": "redirect.test", "Connection": "close"})
        resp = conn.getresponse()
        location = resp.getheader("location")
        status = resp.status
        conn.close()
        assert status == 302, status
        assert location == "/target", location
        print("proxy_redirect regex E2E PASS: 302 Location=/target")
        return 0
    finally:
        if exy:
            exy.terminate()
            try:
                exy.wait(timeout=5)
            except subprocess.TimeoutExpired:
                exy.kill()
        if cid:
            subprocess.run(["docker", "rm", "-f", cid], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
