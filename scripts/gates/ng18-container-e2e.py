#!/usr/bin/env python3
"""Real container E2E for the ExyonQ image (not a smoke test).

Starts a real upstream NGINX + the ExyonQ image on a Docker network and
asserts static, proxy, gunzip and invalid-gzip behaviour with exact bodies
and headers. Test-only.
"""
import argparse
import gzip
import hashlib
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

SMALL = b"ng18-container-e2e-body\n" * 256
PLAIN = b"ng18-container-plain-body\n" * 64


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, text=True, capture_output=True, **kw)


def wait_port(port, timeout=30):
    import socket
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


def http(port, path, accept_encoding=None):
    import http.client
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    headers = {"Host": "e2e.test", "Connection": "close"}
    if accept_encoding:
        headers["Accept-Encoding"] = accept_encoding
    try:
        conn.request("GET", path, headers=headers)
        resp = conn.getresponse()
        body = resp.read()
        return {
            "status": resp.status,
            "content_encoding": resp.getheader("content-encoding"),
            "body": body,
        }
    except Exception as exc:
        return {"error": type(exc).__name__}
    finally:
        conn.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--image", required=True)
    ap.add_argument("--platform", default=None)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()

    out = Path(args.out) if args.out else Path(".exyonq-local/evidence/ng18-container-e2e")
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="ng18-e2e-"))
    net = f"ng18-e2e-{int(time.time())}"
    exy = f"ng18-e2e-exy-{int(time.time())}"
    up = "upstream"
    platform = ["--platform", args.platform] if args.platform else []

    # Upstream files/config
    up_root = work / "upstream"
    up_root.mkdir()
    (up_root / "gzip").write_bytes(gzip.compress(SMALL, mtime=0))
    (up_root / "plain").write_bytes(PLAIN)
    (up_root / "invalid").write_bytes(b"not-a-gzip-stream")
    (work / "upstream.conf").write_text("""
worker_processes 1;
events { worker_connections 128; }
http {
  access_log off;
  server {
    listen 8080;
    root /srv;
    location = /gzip { default_type text/plain; add_header Content-Encoding gzip; }
    location = /plain { default_type text/plain; }
    location = /invalid { default_type text/plain; add_header Content-Encoding gzip; }
    location = /proxy/gzip { alias /srv/gzip; default_type text/plain; add_header Content-Encoding gzip; }
    location = /proxy/plain { alias /srv/plain; default_type text/plain; }
    location = /proxy/invalid { alias /srv/invalid; default_type text/plain; add_header Content-Encoding gzip; }
  }
}
""")

    # ExyonQ config + static file
    static = work / "static"
    static.mkdir()
    (static / "ok.txt").write_bytes(b"ng18-static-e2e\n")
    (work / "exyonq.toml").write_text("""
config_version = 1
[logging]
level = "warn"
format = "text"
[[server]]
listen = "0.0.0.0:8080"
routes = ["static", "proxy"]
[[route]]
name = "static"
match = { path = "/static/" }
root = "/var/www/e2e"
[[route]]
name = "proxy"
match = { path = "/proxy/" }
upstream = "u"
[[upstream]]
name = "u"
target = "http://upstream:8080"
[modules.gunzip]
enabled = true
""")

    checks = []
    try:
        run(["docker", "network", "create", net])
        subprocess.run(["docker", "rm", "-f", "upstream"], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        run([
            "docker", "run", "-d", "--rm", "--name", up, "--network", net,
            "-v", f"{up_root}:/srv:ro",
            "-v", f"{work / 'upstream.conf'}:/etc/nginx/nginx.conf:ro",
            "nginx:1.27-alpine",
        ])
        run([
            "docker", "run", "-d", "--rm", "--name", exy, *platform,
            "--network", net, "-p", "18080:8080",
            "-v", f"{work / 'exyonq.toml'}:/etc/exyonq/config.toml:ro",
            "-v", f"{static}:/var/www/e2e:ro",
            args.image,
        ])
        wait_port(18080)

        static_r = http(18080, "/static/ok.txt")
        checks.append({
            "check": "static body",
            "ok": static_r.get("status") == 200 and static_r.get("body") == b"ng18-static-e2e\n",
            "got": static_r.get("status"),
        })

        plain = http(18080, "/proxy/plain")
        checks.append({
            "check": "proxy plain body",
            "ok": plain.get("status") == 200 and plain.get("body") == PLAIN,
            "got": plain.get("status"),
        })

        gz_identity = http(18080, "/proxy/gzip", "identity")
        checks.append({
            "check": "gunzip identity body",
            "ok": gz_identity.get("status") == 200
            and gz_identity.get("content_encoding") is None
            and gz_identity.get("body") == SMALL,
            "got": gz_identity.get("status"),
        })

        gz_gzip = http(18080, "/proxy/gzip", "gzip")
        checks.append({
            "check": "gzip passthrough",
            "ok": gz_gzip.get("status") == 200
            and gz_gzip.get("content_encoding") == "gzip"
            and gzip.decompress(gz_gzip.get("body", b"")) == SMALL,
            "got": gz_gzip.get("status"),
        })

        invalid = http(18080, "/proxy/invalid", "identity")
        checks.append({
            "check": "invalid gzip aborts connection",
            "ok": "error" in invalid,
            "got": invalid.get("error"),
        })
    finally:
        if not all(c["ok"] for c in checks):
            for name in (exy, up):
                logs = subprocess.run(
                    ["docker", "logs", name], text=True,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                )
                print(f"--- {name} logs ---\n{logs.stdout[-4000:]}")
        for name in (exy, up):
            subprocess.run(["docker", "rm", "-f", name], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(["docker", "network", "rm", net], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    result = {"image": args.image, "checks": checks, "pass": all(c["ok"] for c in checks)}
    (out / f"container-e2e-{args.image.replace(':','-')}.json").write_text(
        json.dumps(result, indent=2)
    )
    print(json.dumps(result, indent=2))
    return 0 if result["pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
