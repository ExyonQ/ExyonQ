#!/usr/bin/env python3
"""Real E2E for nginx `slice`: 4 MiB upstream, 1 MiB segments, segment cache.

Modes:
  --bin target/debug/exyonq   host binary + upstream container on :19090
  --image exyonq:ng18-slice   ExyonQ container + upstream on a Docker network
"""
import argparse
import http.client
import os
import shutil
import socket
import subprocess
import tempfile
import time
from pathlib import Path

SIZE = 4 * 1024 * 1024
BODY = (b"ng18-slice-segment-body\n" * ((SIZE // 24) + 1))[:SIZE]
BODY += b"\0" * (SIZE - len(BODY))


def wait_port(port, timeout=30):
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def get(path):
    conn = http.client.HTTPConnection("127.0.0.1", 18080, timeout=30)
    conn.request("GET", path, headers={"Host": "slice.test", "Connection": "close"})
    resp = conn.getresponse()
    body = resp.read()
    headers = {k.lower(): v for k, v in resp.getheaders()}
    conn.close()
    return resp.status, headers, body


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None)
    ap.add_argument("--image", default=None)
    args = ap.parse_args()
    if not args.bin and not args.image:
        ap.error("one of --bin or --image is required")

    work = Path(tempfile.mkdtemp(prefix="slice-e2e-"))
    logs = work / "logs"
    logs.mkdir()
    up = work / "upstream"
    up.mkdir()
    (up / "big.bin").write_bytes(BODY)
    (work / "upstream.conf").write_text("""
worker_processes 1;
events { worker_connections 128; }
http {
  log_format ranges '$request $http_range';
  access_log /logs/access.log ranges;
  server {
    listen 8080;
    root /srv;
    location / { }
  }
}
""")
    upstream_target = "http://slice-upstream:8080" if args.image else "http://127.0.0.1:19090"
    (work / "exyonq.toml").write_text(f"""
config_version = 1
[logging]
level = "warn"
format = "text"
[[server]]
listen = "0.0.0.0:8080"
routes = ["api"]
[[route]]
name = "api"
match = {{ path = "/" }}
upstream = "u"
cache = "micro"
[[upstream]]
name = "u"
target = "{upstream_target}"
[[cache_policy]]
name = "micro"
ttl_seconds = 60
max_object_bytes = 8388608
[modules.slice.routes."api"]
size = 1048576
""")
    network = None
    upstream_cid = None
    exy_cid = None
    exy_proc = None
    ok = False
    try:
        up_args = [
            "docker", "run", "--rm", "-d", "--name", "slice-upstream",
            "-v", f"{up}:/srv:ro",
            "-v", f"{logs}:/logs",
            "-v", f"{work / 'upstream.conf'}:/etc/nginx/nginx.conf:ro",
        ]
        if args.image:
            network = f"slice-net-{int(time.time())}"
            subprocess.check_output(["docker", "network", "create", network], text=True)
            up_args += ["--network", network]
        else:
            up_args += ["-p", "19090:8080"]
        up_args += ["nginx:1.27-alpine"]
        subprocess.run(["docker", "rm", "-f", "slice-upstream"], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        upstream_cid = subprocess.check_output(up_args, text=True).strip()
        wait_port(19090) if not args.image else time.sleep(1)

        if args.image:
            exy_cid = subprocess.check_output(
                [
                    "docker", "run", "--rm", "-d", "--name", "slice-exy",
                    "--network", network, "-p", "18080:8080",
                    "-v", f"{work / 'exyonq.toml'}:/etc/exyonq/config.toml:ro",
                    args.image,
                ],
                text=True,
            ).strip()
        else:
            env = dict(os.environ)
            env["EXYONQ_CONTROL_SOCKET"] = "/tmp/exyonq-slice.sock"
            env["EXYONQ_CACHE_PURGE_SOCKET"] = "/tmp/exyonq-slice-purge.sock"
            exy_proc = subprocess.Popen(
                [args.bin, "serve", "-c", str(work / "exyonq.toml")],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.STDOUT,
                env=env,
            )
        wait_port(18080)

        s1, h1, b1 = get("/big.bin")
        assert s1 == 200 and b1 == BODY, (s1, len(b1))
        assert h1.get("x-cache") == "miss", h1
        first_ranges = [ln for ln in (logs / "access.log").read_text().splitlines() if "bytes=" in ln]
        assert len(first_ranges) == 4, first_ranges

        s2, h2, b2 = get("/big.bin")
        assert s2 == 200 and b2 == BODY, (s2, len(b2))
        assert h2.get("x-cache") == "hit", h2
        second_ranges = [ln for ln in (logs / "access.log").read_text().splitlines() if "bytes=" in ln]
        assert len(second_ranges) == 4, second_ranges

        print("slice E2E PASS: 4 MiB body, 4 segments, second GET cache hit, 0 extra ranges")
        ok = True
        return 0
    finally:
        if not ok:
            for name in ("slice-exy", "slice-upstream"):
                logs = subprocess.run(
                    ["docker", "logs", name], text=True,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                )
                print(f"--- {name} logs ---\n{logs.stdout[-4000:]}")
        if exy_proc:
            exy_proc.terminate()
            try:
                exy_proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                exy_proc.kill()
        for cid in (exy_cid, upstream_cid):
            if cid:
                subprocess.run(["docker", "rm", "-f", cid], check=False,
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if network:
            subprocess.run(["docker", "network", "rm", network], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
