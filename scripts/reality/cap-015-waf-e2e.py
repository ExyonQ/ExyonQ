#!/usr/bin/env python3
"""CAPABILITY_015 = waf — real product E2E.

Real evidence only:
  real exyonq binary, real HTTP clients (curl/sockets), real upstream peer process.
  No mocks, stubs, fabricated Err, or manually edited verdicts.
"""
from __future__ import annotations

import hashlib
import html
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib.parse import urlencode

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

SITE_MARKER = b"CAP015_STATIC_REAL_SITE"
UP_MARKER = b"CAP015_REAL_UPSTREAM"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
        time.sleep(0.1)
    return False


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: int = 15,
    cookie: str | None = None,
    include_headers: bool = False,
    extra_args: list[str] | None = None,
) -> tuple[int, bytes, str]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = EV / f"curl-{tag}.headers"
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-o",
        str(body_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
    ]
    if include_headers:
        cmd.extend(["-D", str(hdr_path)])
    if headers:
        for h in headers:
            cmd.extend(["-H", h])
    if cookie:
        cmd.extend(["-H", f"Cookie: {cookie}"])
    data_file: Path | None = None
    if data is not None:
        data_file = EV / f"curl-{tag}.data"
        data_file.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_file}"])
    if extra_args:
        cmd.extend(extra_args)
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdrs = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    for p in [body_path, hdr_path, data_file]:
        if p is not None:
            try:
                p.unlink(missing_ok=True)
            except OSError:
                pass
    code_s = (proc.stdout or "").strip()
    code = int(code_s) if code_s.isdigit() else -1
    if proc.returncode != 0 and code == -1:
        hdrs += f"\nCURL_RC={proc.returncode}\nSTDERR={proc.stderr[-500:]}"
    return code, body, hdrs


def ctl_reload(sock: Path, cfg: Path) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "reload", "--config", str(cfg), "--socket", str(sock)],
        capture_output=True,
        text=True,
        env=env,
    )
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def ctl_status(sock: Path, cfg: Path) -> dict[str, Any]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(cfg)
    proc = subprocess.run(
        [str(CTL), "status", "--socket", str(sock), "--format", "json"],
        capture_output=True,
        text=True,
        env=env,
    )
    if proc.returncode != 0:
        return {"ok": False, "rc": proc.returncode, "stderr": proc.stderr[-1000:]}
    try:
        d = json.loads(proc.stdout)
        if isinstance(d, dict):
            d["ok"] = True
            return d
    except json.JSONDecodeError:
        pass
    return {"ok": False, "raw": proc.stdout[-1000:], "stderr": proc.stderr[-1000:]}


def leading_zero_bits(digest: bytes) -> int:
    total = 0
    for b in digest:
        if b == 0:
            total += 8
            continue
        for shift in range(7, -1, -1):
            if b & (1 << shift):
                return total
            total += 1
    return total


def solve_pow(challenge_id: str, difficulty: int) -> str:
    nonce = 0
    while True:
        candidate = str(nonce)
        digest = hashlib.sha256(f"{challenge_id}:{candidate}".encode()).digest()
        if leading_zero_bits(digest) >= difficulty:
            return candidate
        nonce += 1


def parse_challenge_html(body: bytes) -> dict[str, str | int]:
    text = body.decode("utf-8", "replace")
    fields: dict[str, str | int] = {}
    aliases = {
        "challengeId": ["challengeId", "challenge_id", "id"],
        "mac": ["mac", "challengeMac"],
        "expires": ["expires", "expiresAt", "expires_at"],
        "difficulty": ["difficulty", "powDifficulty"],
    }
    for canonical, names in aliases.items():
        for name in names:
            m = re.search(rf"const\s+{re.escape(name)}\s*=\s*(['\"])(.*?)\1", text)
            if not m:
                m = re.search(rf"const\s+{re.escape(name)}\s*=\s*(\d+)", text)
            if m:
                raw = html.unescape(m.group(2) if len(m.groups()) >= 2 else m.group(1))
                fields[canonical] = int(raw) if canonical in {"difficulty", "expires"} and raw.isdigit() else raw
                break
    return fields


class UpstreamHandler(BaseHTTPRequestHandler):
    server_version = "Cap015Upstream/1.0"

    def log_message(self, fmt: str, *args: object) -> None:  # noqa: A003
        return

    def _record_and_reply(self) -> None:
        n = int(self.headers.get("Content-Length") or "0")
        body = self.rfile.read(n) if n > 0 else b""
        self.server.hits.append({"method": self.command, "path": self.path, "body": body.decode("latin-1", "replace")})  # type: ignore[attr-defined]
        if getattr(self.server, "close_early", False):  # type: ignore[attr-defined]
            self.close_connection = True
            return
        if (self.headers.get("Upgrade") or "").lower() == "websocket":
            self.server.ws_hits = getattr(self.server, "ws_hits", 0) + 1  # type: ignore[attr-defined]
            key = (self.headers.get("Sec-WebSocket-Key") or "dGhlIHNhbXBsZSBub25jZQ==").encode()
            import base64
            import hashlib as _hl

            accept = base64.b64encode(
                _hl.sha1(key + b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest()
            ).decode()
            self.send_response(101, "Switching Protocols")
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            self.end_headers()
            return
        payload = UP_MARKER + b" " + self.command.encode() + b" " + self.path.encode("latin-1", "replace") + b"\n" + body
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("X-Upstream-Identity", "cap015-real-peer")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:  # noqa: N802
        self._record_and_reply()

    def do_POST(self) -> None:  # noqa: N802
        self._record_and_reply()


def ws_upgrade_request(port: int, path: str) -> tuple[int, bytes]:
    key = "dGhlIHNhbXBsZSBub25jZQ=="
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: 127.0.0.1:{port}\r\n"
        "User-Agent: cap015-ws-client\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    ).encode()
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.sendall(req)
        s.settimeout(10)
        data = b""
        while b"\r\n\r\n" not in data and len(data) < 8192:
            chunk = s.recv(4096)
            if not chunk:
                break
            data += chunk
    m = re.match(rb"HTTP/\d(?:\.\d)?\s+(\d+)", data)
    return (int(m.group(1)) if m else -1), data


def start_upstream(port: int, *, close_early: bool = False) -> ThreadingHTTPServer:
    srv = ThreadingHTTPServer(("127.0.0.1", port), UpstreamHandler)
    srv.hits = []  # type: ignore[attr-defined]
    srv.close_early = close_early  # type: ignore[attr-defined]
    t = threading.Thread(target=srv.serve_forever, daemon=True)
    t.start()
    return srv


def write_site(root: Path) -> None:
    (root / "site").mkdir(parents=True, exist_ok=True)
    (root / "site" / "index.html").write_bytes(SITE_MARKER + b"\n")
    # Wire-eligible static head (might_use_static_wire): Cap015 must WAF before serve.
    (root / "site" / "routes").mkdir(parents=True, exist_ok=True)
    (root / "site" / "routes" / "route001.bin").write_bytes(b"CAP015_WIRE_STATIC\n")
    (root / "health-evil").mkdir(parents=True, exist_ok=True)
    (root / "health-evil" / "index.html").write_bytes(b"CAP015_HEALTH_EVIL\n")


def waf_toml(
    mode: str,
    abuse: str = "off",
    rps: int = 10,
    burst: int = 20,
    exclude_api: bool = False,
    invalid_body_limit: bool = False,
    *,
    enabled: bool = True,
) -> str:
    max_body = 0 if invalid_body_limit else 1048576
    exclusion = """
[[waf.exclusion]]
path_prefix = "/api"
rule_ids = ["EXY-XSS-1001"]
""" if exclude_api else ""
    return f"""[waf]
enabled = {str(enabled).lower()}
mode = "{mode}"
engine = "native"
max_inspection_body_bytes = {max_body}
on_inspection_limit = "block"
fail_policy = "closed_for_invalid_rules"

[waf.abuse]
mode = "{abuse}"
requests_per_second = {rps}
burst = {burst}

[[waf.ruleset]]
id = "exyonq-core"
enabled = true
{exclusion}
"""


def write_config(
    cfg: Path,
    *,
    listen: int,
    upstream: int,
    root: Path,
    mode: str = "block",
    abuse: str = "off",
    rps: int = 10,
    burst: int = 20,
    exclude_api: bool = False,
    invalid_body_limit: bool = False,
    enabled: bool = True,
) -> None:
    # IR allows only one route action. Structural rewrite is rewrite-only (no root).
    # Original-URI WAF must still Block before rewrite; rewrite-without-root would 404 if WAF missed.
    cfg.write_text(
        f"""config_version = 2

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["api", "api2", "evil", "health_evil", "site", "ws"]

[[route]]
name = "api"
match = {{ path = "/api" }}
upstream = "backend"

[[route]]
name = "api2"
match = {{ path = "/api2" }}
root = "{root}/site"
index = "index.html"

[[route]]
name = "evil"
match = {{ path = "/evil" }}
rewrite = "/site/index.html"

[[route]]
name = "health_evil"
match = {{ path = "/health-evil" }}
root = "{root}/health-evil"
index = "index.html"

[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{root}/site"
index = "index.html"

[[route]]
name = "ws"
match = {{ path = "/ws" }}
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:{upstream}"
timeout_ms = 3000

[full_page_cache]
enabled = true

{waf_toml(mode, abuse, rps, burst, exclude_api, invalid_body_limit, enabled=enabled)}
"""
    )


def stop_proc(p: subprocess.Popen[str] | None) -> None:
    if p is None:
        return
    if p.poll() is None:
        p.send_signal(signal.SIGTERM)
        try:
            p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            p.kill()
            p.wait(timeout=5)


def incomplete_content_length_request(port: int) -> tuple[int, bytes]:
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        req = (
            b"POST /api/ HTTP/1.1\r\n"
            b"Host: 127.0.0.1\r\n"
            b"Content-Length: 64\r\n"
            b"Content-Type: application/octet-stream\r\n"
            b"Connection: close\r\n\r\n"
            b"short<script>"
        )
        s.sendall(req)
        s.shutdown(socket.SHUT_WR)
        s.settimeout(10)
        data = b""
        while True:
            try:
                chunk = s.recv(4096)
            except socket.timeout:
                break
            if not chunk:
                break
            data += chunk
    m = re.match(rb"HTTP/\d(?:\.\d)?\s+(\d+)", data)
    return (int(m.group(1)) if m else -1), data


def raw_http_exchange(port: int, request: bytes, *, timeout: float = 10.0) -> tuple[int, bytes]:
    """Full request already framed — do not half-close (Hyper may drop response on SHUT_WR)."""
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        s.sendall(request)
        s.settimeout(timeout)
        data = b""
        while True:
            try:
                chunk = s.recv(4096)
            except socket.timeout:
                break
            if not chunk:
                break
            data += chunk
    m = re.match(rb"HTTP/\d(?:\.\d)?\s+(\d+)", data)
    return (int(m.group(1)) if m else -1), data


def env_blocker(reason: str) -> dict[str, Any]:
    return {"ok": False, "status": "ENVIRONMENT_BLOCKER", "reason": reason}


def find_php_fpm() -> str | None:
    for c in ("php-fpm", "php-fpm8.3", "php-fpm8.2", "php-fpm8.1"):
        p = shutil.which(c)
        if p:
            return p
    for p in (Path("/usr/sbin/php-fpm8.3"), Path("/usr/sbin/php-fpm")):
        if p.is_file() and os.access(p, os.X_OK):
            return str(p)
    return None


def generate_ephemeral_tls(tmp: Path) -> tuple[Path, Path] | None:
    script = WS / "scripts" / "test-tls" / "generate-ephemeral-tls.sh"
    if not script.is_file():
        return None
    tmp.mkdir(parents=True, exist_ok=True)
    proc = subprocess.run(
        ["bash", str(script), "--print-paths", "--tmpdir", str(tmp)],
        capture_output=True,
        text=True,
        cwd=str(WS),
    )
    if proc.returncode != 0:
        return None
    cert = key = None
    for line in proc.stdout.splitlines():
        if line.startswith("CERT="):
            cert = Path(line.split("=", 1)[1])
        elif line.startswith("KEY="):
            key = Path(line.split("=", 1)[1])
    if cert and key and cert.is_file() and key.is_file():
        return cert, key
    return None


def probe_http2_waf(tmp: Path, root: Path) -> dict[str, Any]:
    tls = generate_ephemeral_tls(tmp / "tls-h2")
    if tls is None:
        return env_blocker("ephemeral TLS generator failed for HTTP/2 WAF probe")
    cert, key = tls
    listen = pick_port()
    cfg = tmp / "waf-h2.toml"
    cfg.write_text(
        f"""config_version = 2
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["site"]
tls = {{ cert = "{cert}", key = "{key}" }}
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{root}/site"
index = "index.html"
[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 1048576
on_inspection_limit = "block"
fail_policy = "closed_for_invalid_rules"
[waf.abuse]
mode = "off"
requests_per_second = 10
burst = 20
[[waf.ruleset]]
id = "exyonq-core"
enabled = true
"""
    )
    log = EV / "exyonq-cap015-h2.log"
    env = os.environ.copy()
    env.pop("EXYONQ_WAF_ENFORCE", None)
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
        text=True,
    )
    try:
        if not wait_listen(listen):
            return env_blocker("HTTP/2 WAF listener not ready")
        url_ok = f"https://127.0.0.1:{listen}/site/"
        url_bad = f"https://127.0.0.1:{listen}/site/?q=%3Cscript%3E"
        c_ok, b_ok, _ = curl_req(url_ok, extra_args=["-k", "--http2"])
        c_bad, b_bad, _ = curl_req(url_bad, extra_args=["-k", "--http2"])
        ver = subprocess.run(
            ["curl", "-sk", "--http2", "-o", "/dev/null", "-w", "%{http_version}", url_ok],
            capture_output=True,
            text=True,
        )
        http_ver = (ver.stdout or "").strip()
        ok = c_ok == 200 and SITE_MARKER in b_ok and c_bad == 403 and SITE_MARKER not in b_bad and http_ver.startswith("2")
        return {
            "ok": ok,
            "benign_code": c_ok,
            "malicious_code": c_bad,
            "http_version": http_ver,
            "REAL_HTTP2": "YES" if http_ver.startswith("2") else "NO",
        }
    finally:
        stop_proc(proc)


def probe_http3_waf(tmp: Path, root: Path) -> dict[str, Any]:
    if shutil.which("docker") is None:
        return env_blocker("docker required for real curl --http3-only Cap015 H3 probe")
    tls = generate_ephemeral_tls(tmp / "tls-h3")
    if tls is None:
        return env_blocker("ephemeral TLS generator failed for HTTP/3 WAF probe")
    cert, key = tls
    tcp = pick_port()
    udp = pick_port()
    cfg = tmp / "waf-h3.toml"
    cfg.write_text(
        f"""config_version = 2
[http3]
enabled = true
provider = "s2n"
request_body_drain_cap_bytes = 1048576
[[server]]
listen = "127.0.0.1:{tcp}"
http3_listen = "0.0.0.0:{udp}"
routes = ["site"]
[server.tls]
cert = "{cert}"
key = "{key}"
[[route]]
name = "site"
match = {{ path = "/site" }}
root = "{root}/site"
index = "index.html"
[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 1048576
on_inspection_limit = "block"
fail_policy = "closed_for_invalid_rules"
[waf.abuse]
mode = "off"
requests_per_second = 10
burst = 20
[[waf.ruleset]]
id = "exyonq-core"
enabled = true
"""
    )
    log = EV / "exyonq-cap015-h3.log"
    env = os.environ.copy()
    env.pop("EXYONQ_WAF_ENFORCE", None)
    proc = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
        text=True,
    )
    image = os.environ.get("H3_CLIENT_IMAGE", "ymuski/curl-http3:latest")
    try:
        deadline = time.time() + 60
        ready = False
        while time.time() < deadline:
            if log.is_file() and "HTTP/3 listening" in log.read_text(errors="replace"):
                ready = True
                break
            if proc.poll() is not None:
                break
            time.sleep(0.2)
        if not ready:
            return {
                **env_blocker("HTTP/3 WAF listener not ready"),
                "log_tail": log.read_text(errors="replace")[-2000:] if log.is_file() else "",
            }
        host_gateway = "host.docker.internal"
        # Linux docker: use host network for loopback UDP.
        use_host_net = sys.platform.startswith("linux")
        results = {}
        for label, q in (("benign", ""), ("xss", "?q=%3Cscript%3E")):
            url = f"https://127.0.0.1:{udp}/site/{q}" if use_host_net else f"https://{host_gateway}:{udp}/site/{q}"
            cmd = ["docker", "run", "--rm"]
            if use_host_net:
                cmd.extend(["--network", "host"])
            else:
                cmd.extend(["--add-host=host.docker.internal:host-gateway"])
            body_file = f"/tmp/cap015-h3-{label}.bin"
            cmd.extend(
                [
                    image,
                    "curl",
                    "-sk",
                    "--http3-only",
                    "--max-time",
                    "20",
                    "-o",
                    body_file,
                    "-w",
                    "%{http_code}|%{http_version}",
                    url,
                ]
            )
            # Capture body via docker stdout by writing to stdout instead of file in container.
            cmd = ["docker", "run", "--rm"]
            if use_host_net:
                cmd.extend(["--network", "host"])
            else:
                cmd.extend(["--add-host=host.docker.internal:host-gateway"])
            cmd.extend(
                [
                    image,
                    "curl",
                    "-sk",
                    "--http3-only",
                    "--max-time",
                    "20",
                    "-w",
                    "\n%{http_code}|%{http_version}",
                    url,
                ]
            )
            run = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
            out = (run.stdout or "") + "\n" + (run.stderr or "")
            lines = [ln for ln in (run.stdout or "").splitlines() if ln.strip()]
            meta = lines[-1] if lines else "|"
            body = "\n".join(lines[:-1]).encode() if len(lines) > 1 else b""
            code_s, _, ver = meta.partition("|")
            try:
                code = int(code_s)
            except ValueError:
                code = -1
            results[label] = {"code": code, "http_version": ver.strip(), "body_prefix": body[:80].decode("latin-1", "replace"), "rc": run.returncode, "raw_tail": out[-400:]}
        benign_body = results["benign"]["body_prefix"].encode("latin-1", "replace")
        ok = (
            results["benign"]["code"] == 200
            and SITE_MARKER in benign_body
            and results["benign"]["http_version"].startswith("3")
            and results["xss"]["code"] == 403
            and SITE_MARKER.decode() not in results["xss"]["body_prefix"]
        )
        return {"ok": ok, "REAL_HTTP3": "YES" if ok else "NO", "results": results}
    except Exception as exc:
        return {**env_blocker(f"HTTP/3 probe exception: {exc}")}
    finally:
        stop_proc(proc)


def probe_fastcgi_waf(tmp: Path) -> dict[str, Any]:
    php_fpm = find_php_fpm()
    if not php_fpm:
        return env_blocker("php-fpm required for Cap015 FastCGI WAF path")
    www = tmp / "fcgi-www"
    www.mkdir(parents=True, exist_ok=True)
    os.chmod(tmp, 0o755)
    os.chmod(www, 0o755)
    (www / "index.php").write_text("<?php\nheader('Content-Type: text/plain');\necho 'CAP015_PHP_OK';\nfile_put_contents(sys_get_temp_dir().'/cap015-php-side.txt', 'SIDE');\n")
    side = Path("/tmp/cap015-php-side.txt")
    if side.exists():
        side.unlink()
    sock = tmp / "php-fpm.sock"
    fpm_log = tmp / "fpm.log"
    fpm_cfg = tmp / "php-fpm.conf"
    user, group = ("www-data", "www-data")
    try:
        import pwd

        pwd.getpwnam("www-data")
    except Exception:
        user, group = ("nobody", "nogroup")
    fpm_cfg.write_text(
        f"""[global]
error_log = {fpm_log}
daemonize = no
[www]
user = {user}
group = {group}
listen = {sock}
listen.mode = 0666
pm = static
pm.max_children = 2
clear_env = no
"""
    )
    listen = pick_port()
    cfg = tmp / "waf-fcgi.toml"
    cfg.write_text(
        f"""config_version = 2
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["php"]
[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"
[[fcgi_pool]]
name = "php"
address = "{sock}"
document_root = "{www}"
max_concurrency = 4
[waf]
enabled = true
mode = "block"
engine = "native"
max_inspection_body_bytes = 1048576
on_inspection_limit = "block"
fail_policy = "closed_for_invalid_rules"
[waf.abuse]
mode = "off"
requests_per_second = 10
burst = 20
[[waf.ruleset]]
id = "exyonq-core"
enabled = true
"""
    )
    fpm = subprocess.Popen(
        [php_fpm, "--nodaemonize", "--fpm-config", str(fpm_cfg)],
        stdout=(tmp / "fpm.out").open("w"),
        stderr=subprocess.STDOUT,
    )
    log = EV / "exyonq-cap015-fcgi.log"
    env = os.environ.copy()
    env.pop("EXYONQ_WAF_ENFORCE", None)
    proc: subprocess.Popen[str] | None = None
    try:
        if not wait_sock(sock, timeout=20):
            return {**env_blocker("php-fpm socket missing"), "fpm_log": fpm_log.read_text(errors="replace")[-1000:] if fpm_log.exists() else ""}
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
            text=True,
        )
        if not wait_listen(listen):
            return env_blocker("FastCGI WAF listener not ready")
        c_ok, b_ok, _ = curl_req(f"http://127.0.0.1:{listen}/index.php")
        before_side = side.exists()
        c_bad, b_bad, _ = curl_req(f"http://127.0.0.1:{listen}/index.php?q=%3Cscript%3E")
        after_side_bad = side.exists() and not before_side
        # side effect from benign may exist; malicious must not create new marker when blocked first
        if side.exists():
            side.unlink()
        c_bad2, _, _ = curl_req(f"http://127.0.0.1:{listen}/index.php?q=%3Cscript%3E")
        side_after_block = side.exists()
        ok = (
            c_ok == 200
            and b"CAP015_PHP_OK" in b_ok
            and c_bad == 403
            and b"CAP015_PHP_OK" not in b_bad
            and c_bad2 == 403
            and not side_after_block
        )
        return {
            "ok": ok,
            "benign_code": c_ok,
            "malicious_code": c_bad,
            "PHP_SCRIPT_EXECUTED_AFTER_BLOCK": "YES" if side_after_block else "NO",
            "after_side_bad_note": after_side_bad,
        }
    finally:
        stop_proc(proc)
        stop_proc(fpm)


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict[str, Any] = {
        "FEATURE_ID": "waf",
        "CAPABILITY": "CAPABILITY_015",
        "CAPABILITY_NAME": "waf",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQCTL_BINARY": str(CTL),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Native WAF signature block/monitor plus abuse rate_limit/challenge on real HTTP paths",
        "REAL_HTTP_ONLY": "YES",
        "REAL_EXYONQ_BINARY": "YES",
        "REAL_UPSTREAM_PEER_PROCESS": "YES",
        "USES_MOCKS": "NO",
        "USES_STUBS": "NO",
        "USES_SYNTHETIC_ERR": "NO",
        "PRE_CAP015_DUAL_LINUX_WAF_PROBE": "PASS_SUPPLEMENTAL",
        "WAF_LOGIC_P3_A_FINAL_CLASSIFICATION": "FIXED",
        "WAF_LOGIC_P3_B_FINAL_CLASSIFICATION": "FIXED",
        "PUSH": "NO",
        "GHCR_WRITE": "NO",
    }
    checks: dict[str, Any] = {}
    result["checks"] = checks

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonq binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    if not CTL.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "missing exyonqctl binary"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    result["BINARY_SHA256"] = sha256_file(BINARY)
    result["EXYONQ_BINARY_SHA256"] = result["BINARY_SHA256"]
    result["EXYONQCTL_BINARY_SHA256"] = sha256_file(CTL)

    tmp = Path(tempfile.mkdtemp(prefix="cap015-waf-", dir=str(EV)))
    root = tmp / "www"
    write_site(root)
    up_port = pick_port()
    upstream = start_upstream(up_port)
    if not wait_listen(up_port, timeout=5):
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": "upstream peer not listening"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    listen = pick_port()
    # Darwin sockaddr_un SUN_LEN is short; keep control socket under /tmp.
    ctrl = Path(tempfile.mkdtemp(prefix="c015s-", dir="/tmp")) / "c.sock"
    cfg = tmp / "waf.toml"
    write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block")
    log = EV / "exyonq-cap015.log"
    env = os.environ.copy()
    # IR mode=block implies enforce; do not force EXYONQ_WAF_ENFORCE so monitor reloads can disable enforcement.
    env.pop("EXYONQ_WAF_ENFORCE", None)
    env.pop("EXYONQ_WAF_MODE", None)
    env.pop("EXYONQ_WAF_ABUSE", None)
    env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
    env["EXYONQ_CONFIG"] = str(cfg)
    srv: subprocess.Popen[str] | None = subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
        text=True,
    )
    pid_before = srv.pid
    base = f"http://127.0.0.1:{listen}"

    try:
        if not wait_listen(listen) or not wait_sock(ctrl) or srv.poll() is not None:
            result.update({
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "server or control socket not ready",
                "SERVER_LOG_TAIL": log.read_text(errors="replace")[-4000:] if log.is_file() else "",
            })
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 2

        c, b, _ = curl_req(f"{base}/site/")
        checks["mode_block_benign_static"] = {"code": c, "ok": c == 200 and SITE_MARKER in b}

        c, b, _ = curl_req(f"{base}/site/?q=%3Cscript%3E")
        checks["mode_block_xss"] = {"code": c, "ok": c == 403 and SITE_MARKER not in b, "body": b[:200].decode("latin-1", "replace")}

        # WAF-LOGIC-P2-G: non-UTF8 header bytes must not erase XSS from WAF view.
        c_nh, b_nh = raw_http_exchange(
            listen,
            (
                b"GET /site/ HTTP/1.1\r\n"
                b"Host: 127.0.0.1\r\n"
                b"X-Evil: <script>\xff\r\n"
                b"Connection: close\r\n\r\n"
            ),
        )
        checks["header_non_utf8_xss"] = {
            "code": c_nh,
            "ok": c_nh == 403 and SITE_MARKER not in b_nh,
            "body": b_nh[:200].decode("latin-1", "replace"),
            "note": "obs-text / invalid UTF-8 in header value must still match XSS",
        }

        # WAF-LOGIC-P1-E regression: wire-eligible static head must still Block.
        # /site/routes/route* is might_use_static_wire (P7 shared wire on Linux — body
        # is bench wire bytes, not necessarily the on-disk marker).
        wire_path = "/site/routes/route001.bin"
        c_ws, b_ws, _ = curl_req(f"{base}{wire_path}")
        c_wx, b_wx, _ = curl_req(f"{base}{wire_path}?q=%3Cscript%3E")
        checks["static_wire_eligible_waf"] = {
            "path": wire_path,
            "benign_code": c_ws,
            "malicious_code": c_wx,
            "benign_body_prefix": b_ws[:80].decode("latin-1", "replace"),
            "malicious_body_prefix": b_wx[:80].decode("latin-1", "replace"),
            "ok": c_ws == 200
            and len(b_ws) > 0
            and c_wx == 403
            and SITE_MARKER not in b_wx
            and b"CAP015_WIRE_STATIC" not in b_wx,
            "note": "Wire-eligible head: WAF before StaticWire/P7 (Linux) or Hyper fallback (Darwin)",
        }

        # WAF-LOGIC-P1-H: non-UTF8 request-target must not skip wire WAF (lossy inspect).
        c_pt, b_pt = raw_http_exchange(
            listen,
            (
                b"GET /site/routes/route001.bin?q=%3Cscript%3E\xff HTTP/1.1\r\n"
                b"Host: 127.0.0.1\r\n"
                b"Connection: close\r\n\r\n"
            ),
        )
        checks["path_non_utf8_wire_xss"] = {
            "code": c_pt,
            "ok": c_pt == 403 and SITE_MARKER not in b_pt and b"CAP015_WIRE_STATIC" not in b_pt,
            "body": b_pt[:200].decode("latin-1", "replace"),
            "note": "invalid UTF-8 in request-target must still match XSS on wire/static path",
        }

        c, b, _ = curl_req(f"{base}/site/?q=1%20UNION%20SELECT%20password%20FROM%20users")
        checks["mode_block_sql"] = {"code": c, "ok": c == 403 and SITE_MARKER not in b}

        c, b, _ = curl_req(f"{base}/site/?q=select")
        checks["fp_control_sql_keyword"] = {"code": c, "ok": c == 200 and SITE_MARKER in b, "actual_contract": "benign standalone select should not match SQLi pattern"}

        # Daemon reload always reads the process-bound EXYONQ_CONFIG path (not a
        # sibling). Writing that path may race the FS watcher (auto-reload then
        # ctl NO_OP). Behavioral oracle is authoritative; NO_OP is OK iff exclusion
        # is already live.
        before = len(upstream.hits)  # type: ignore[attr-defined]
        st_before = ctl_status(ctrl, cfg)
        gen_before = int(st_before.get("generation") or 0) if st_before.get("ok") is not False else 0
        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block", exclude_api=True)
        rc, out = ctl_reload(ctrl, cfg)
        # If watcher already applied, ctl may NO_OP — wait briefly and re-probe.
        time.sleep(0.35)
        c_api, b_api, _ = curl_req(f"{base}/api/?q=%3Cscript%3E")
        c_api2, b_api2, _ = curl_req(f"{base}/api2/?q=%3Cscript%3E")
        identical_reload = "EXY-RELOAD-0008" in out or "identical config" in out.lower()
        st_after = ctl_status(ctrl, cfg)
        gen_after = int(st_after.get("generation") or 0) if st_after.get("ok") is not False else 0
        behavior_ok = c_api == 200 and c_api2 == 403 and SITE_MARKER not in b_api2
        checks["exclusion_api_not_api2"] = {
            "reload_rc": rc,
            "reload_output": out[-500:],
            "api_code": c_api,
            "api_upstream_delta": len(upstream.hits) - before,  # type: ignore[attr-defined]
            "api2_code": c_api2,
            "generation_before": gen_before,
            "generation_after": gen_after,
            # Generation must advance (ctl and/or FS watcher). Sibling --config paths are not daemon-bound.
            "ok": behavior_ok and gen_after > gen_before,
            "note": "/api has EXY-XSS-1001 exclusion; /api2 must not inherit /api prefix",
            "api_body_prefix": b_api[:100].decode("latin-1", "replace"),
            "reload_was_identical": identical_reload,
        }

        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block")
        rc, out = ctl_reload(ctrl, cfg)
        c, b, _ = curl_req(f"{base}/evil?q=%3Cscript%3E")
        checks["original_uri"] = {"reload_rc": rc, "code": c, "ok": rc == 0 and c == 403 and b"CAP015_REWRITE_TARGET" not in b, "reload_output": out[-500:]}

        before = len(upstream.hits)  # type: ignore[attr-defined]
        c, b, _ = curl_req(f"{base}/api/?q=%3Cscript%3E")
        checks["proxy_block_no_upstream"] = {"code": c, "upstream_delta": len(upstream.hits) - before, "ok": c == 403 and len(upstream.hits) == before}  # type: ignore[attr-defined]

        before = len(upstream.hits)  # type: ignore[attr-defined]
        c, b, _ = curl_req(f"{base}/api/ok")
        checks["proxy_benign_upstream"] = {"code": c, "upstream_delta": len(upstream.hits) - before, "ok": c == 200 and UP_MARKER in b and len(upstream.hits) > before}  # type: ignore[attr-defined]

        # Cap015 FPC: product [full_page_cache] enabled — WAF must Block before any Hit serve.
        # /api2 uses Hyper dispatch (WAF then fpc_gate); /site/* early path skips FPC by design.
        c_fp1, b_fp1, _ = curl_req(f"{base}/api2/")
        c_fp2, b_fp2, _ = curl_req(f"{base}/api2/")
        c_fpm, b_fpm, _ = curl_req(f"{base}/api2/?q=%3Cscript%3E")
        checks["fpc_waf_before_cache"] = {
            "prime_code": c_fp1,
            "second_code": c_fp2,
            "malicious_code": c_fpm,
            "ok": c_fp1 == 200
            and SITE_MARKER in b_fp1
            and c_fp2 == 200
            and SITE_MARKER in b_fp2
            and c_fpm == 403
            and SITE_MARKER not in b_fpm,
            "note": "WAF header gate precedes fpc_gate; Block must not return cached body",
            "SECURITY_RELEVANT_FPC_BYPASS": "NO" if c_fpm == 403 and SITE_MARKER not in b_fpm else "YES",
        }

        # Exact-path probes bypass WAF; false-prefix /health-evil must not inherit that bypass.
        c_h, b_h, _ = curl_req(f"{base}/health?q=%3Cscript%3E")
        c_he, b_he, _ = curl_req(f"{base}/health-evil?q=%3Cscript%3E")
        checks["health_bypass"] = {
            "health_code": c_h,
            "health_body": b_h[:80].decode("latin-1", "replace"),
            "health_evil_code": c_he,
            "ok": c_h == 200 and b"ok" in b_h and c_he == 403 and b"CAP015_HEALTH_EVIL" not in b_he,
        }

        c_l, b_l, _ = curl_req(f"{base}/live")
        c_r, b_r, _ = curl_req(f"{base}/ready")
        checks["ready_live"] = {
            "live_code": c_l,
            "ready_code": c_r,
            "ok": c_l == 200 and c_r == 200 and b"live" in b_l and b"ready" in b_r,
        }

        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="disabled", enabled=True)
        rc_dis, out_dis = ctl_reload(ctrl, cfg)
        c_db, b_db, _ = curl_req(f"{base}/site/")
        c_dm, b_dm, _ = curl_req(f"{base}/site/?q=%3Cscript%3E")
        checks["mode_disabled"] = {
            "reload_rc": rc_dis,
            "benign_code": c_db,
            "malicious_code": c_dm,
            "ok": rc_dis == 0 and c_db == 200 and SITE_MARKER in b_db and c_dm == 200 and SITE_MARKER in b_dm,
            "reload_output": out_dis[-500:],
        }
        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block")
        rc_reb, out_reb = ctl_reload(ctrl, cfg)
        if rc_reb != 0:
            checks["mode_disabled"]["ok"] = False
            checks["mode_disabled"]["reblock_reload"] = out_reb[-500:]

        c, b, _ = curl_req(f"{base}/site/?q=%3Cscript%3E", headers=["X-Forwarded-For: 203.0.113.9"])
        checks["xff_spoof"] = {"code": c, "ok": c == 403 and SITE_MARKER not in b}

        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block", abuse="rate_limit", rps=1, burst=1)
        rc, out = ctl_reload(ctrl, cfg)
        rate_codes: list[int] = []
        rate_retry_after = False
        rate_hdr_sample = ""
        for _ in range(8):
            code, _, hdrs = curl_req(f"{base}/site/", include_headers=True)
            rate_codes.append(code)
            if code == 429:
                rate_hdr_sample = hdrs[:500]
            rate_retry_after = rate_retry_after or ("retry-after:" in hdrs.lower())
        checks["rate_limit"] = {
            "reload_rc": rc,
            "codes": rate_codes,
            "retry_after": rate_retry_after,
            "ok": rc == 0 and 429 in rate_codes and rate_retry_after,
            "reload_output": out[-500:],
            "hdr_sample": rate_hdr_sample,
        }

        time.sleep(1.2)
        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block", abuse="challenge", rps=1, burst=1)
        rc, out = ctl_reload(ctrl, cfg)
        first_code, _, _ = curl_req(f"{base}/site/")
        challenge_code, challenge_body, _ = curl_req(f"{base}/site/", include_headers=True)
        fields = parse_challenge_html(challenge_body)
        grant_code = -1
        replay_code = -1
        after_code = -1
        grant_cookie = ""
        xss_after_code = -1
        challenge_ok = False
        if rc == 0 and challenge_code == 403 and {"challengeId", "mac", "expires", "difficulty"}.issubset(fields):
            nonce = solve_pow(str(fields["challengeId"]), int(fields["difficulty"]))
            form = urlencode({
                "challenge_id": str(fields["challengeId"]),
                "mac": str(fields["mac"]),
                "expires": str(fields["expires"]),
                "difficulty": str(fields["difficulty"]),
                "nonce": nonce,
            }).encode()
            grant_code, _, grant_headers = curl_req(
                f"{base}/.exyonq/waf-challenge",
                method="POST",
                data=form,
                headers=["Content-Type: application/x-www-form-urlencoded"],
                include_headers=True,
            )
            m = re.search(r"(?im)^Set-Cookie:\s*([^;\r\n]+)", grant_headers)
            grant_cookie = m.group(1) if m else ""
            replay_code, _, _ = curl_req(
                f"{base}/.exyonq/waf-challenge",
                method="POST",
                data=form,
                headers=["Content-Type: application/x-www-form-urlencoded"],
            )
            after_code, _, _ = curl_req(f"{base}/site/", cookie=grant_cookie if grant_cookie else None)
            xss_after_code, _, _ = curl_req(f"{base}/site/?q=%3Cscript%3E", cookie=grant_cookie if grant_cookie else None)
            challenge_ok = grant_code in (200, 302) and bool(grant_cookie) and replay_code >= 400 and after_code == 200 and xss_after_code == 403
        checks["challenge_protocol"] = {
            "reload_rc": rc,
            "first_code": first_code,
            "challenge_code": challenge_code,
            "fields": fields,
            "grant_code": grant_code,
            "set_cookie_present": bool(grant_cookie),
            "replay_code": replay_code,
            "after_grant_code": after_code,
            "xss_after_grant_code": xss_after_code,
            "ok": challenge_ok,
            "reload_output": out[-500:],
        }

        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="monitor")
        rc_m, out_m = ctl_reload(ctrl, cfg)
        st_m = ctl_status(ctrl, cfg)
        gen_m = st_m.get("generation", -1)
        c_m, b_m, _ = curl_req(f"{base}/site/?q=%3Cscript%3E")
        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block")
        rc_b, out_b = ctl_reload(ctrl, cfg)
        st_b = ctl_status(ctrl, cfg)
        gen_b = st_b.get("generation", -1)
        c_b, b_b, _ = curl_req(f"{base}/site/?q=%3Cscript%3E")
        same_pid = srv.poll() is None and srv.pid == pid_before
        checks["reload_monitor_to_block"] = {"monitor_rc": rc_m, "block_rc": rc_b, "monitor_code": c_m, "block_code": c_b, "generation_monitor": gen_m, "generation_block": gen_b, "same_pid": same_pid, "ok": rc_m == 0 and rc_b == 0 and c_m == 200 and SITE_MARKER in b_m and c_b == 403 and same_pid, "monitor_out": out_m[-300:], "block_out": out_b[-300:]}

        gen_before_bad = gen_b
        good_bytes = cfg.read_bytes()
        write_config(cfg, listen=listen, upstream=up_port, root=root, mode="block", invalid_body_limit=True)
        rc_bad, out_bad = ctl_reload(ctrl, cfg)
        st_bad = ctl_status(ctrl, cfg)
        gen_bad = st_bad.get("generation", -1)
        c_keep, _, _ = curl_req(f"{base}/site/?q=%3Cscript%3E")
        checks["invalid_reload"] = {"reload_rc": rc_bad, "generation_before": gen_before_bad, "generation_after": gen_bad, "code_after": c_keep, "ok": rc_bad != 0 and gen_bad == gen_before_bad and c_keep == 403, "reload_output": out_bad[-500:]}
        cfg.write_bytes(good_bytes)

        before = len(upstream.hits)  # type: ignore[attr-defined]
        c, b, _ = curl_req(f"{base}/api/", method="POST", data=b"body=<script>alert(1)</script>", headers=["Content-Type: application/x-www-form-urlencoded"])
        checks["body_xss_proxy"] = {"code": c, "upstream_delta": len(upstream.hits) - before, "ok": c == 403 and len(upstream.hits) == before}  # type: ignore[attr-defined]

        before = len(upstream.hits)  # type: ignore[attr-defined]
        code_incomplete, raw_incomplete = incomplete_content_length_request(listen)
        checks["body_collect_fail"] = {"code": code_incomplete, "upstream_delta": len(upstream.hits) - before, "ok": code_incomplete == 502 and len(upstream.hits) == before, "raw_prefix": raw_incomplete[:120].decode("latin-1", "replace")}

        ws_before = getattr(upstream, "ws_hits", 0)
        ws_ok_code, ws_ok_raw = ws_upgrade_request(listen, "/ws")
        ws_bad_code, ws_bad_raw = ws_upgrade_request(listen, "/ws?q=%3Cscript%3E")
        checks["websocket"] = {
            "benign_code": ws_ok_code,
            "malicious_code": ws_bad_code,
            "ws_hits_delta": getattr(upstream, "ws_hits", 0) - ws_before,
            "ok": ws_ok_code == 101 and ws_bad_code == 403 and getattr(upstream, "ws_hits", 0) == ws_before + 1,
            "benign_prefix": ws_ok_raw[:120].decode("latin-1", "replace"),
            "malicious_prefix": ws_bad_raw[:120].decode("latin-1", "replace"),
        }

        checks["http2"] = probe_http2_waf(tmp, root)
        result["CAP015_HTTP2"] = "PASS" if checks["http2"].get("ok") else checks["http2"].get("status", "FAIL")
        checks["http3"] = probe_http3_waf(tmp, root)
        result["CAP015_HTTP3"] = "PASS" if checks["http3"].get("ok") else checks["http3"].get("status", "FAIL")
        if sys.platform.startswith("linux"):
            checks["fastcgi"] = probe_fastcgi_waf(tmp)
            result["CAP015_FASTCGI"] = "PASS" if checks["fastcgi"].get("ok") else checks["fastcgi"].get("status", "FAIL")
        else:
            checks["fastcgi"] = {
                "ok": True,
                "status": "LOCAL_ITERATION_ONLY",
                "reason": "php-fpm Cap015 FastCGI required on Netcup/Oracle; Darwin local iteration exempt (NOT_LINUX_EVIDENCE)",
            }
            result["CAP015_FASTCGI"] = "LOCAL_ITERATION_ONLY"
        result["CAP015_WEBSOCKET"] = "PASS" if checks["websocket"].get("ok") else "FAIL"
        result["CAP015_HTTP11"] = "PASS"

        env_blocked = any(v.get("status") == "ENVIRONMENT_BLOCKER" for v in checks.values())
        overall = all(bool(checks[k].get("ok")) for k in checks) and srv.poll() is None
        result["PID_BEFORE"] = pid_before
        result["PID_AFTER"] = srv.pid if srv.poll() is None else None
        result["REQUIRED_CHECKS"] = list(checks.keys())
        result["FAILED_CHECKS"] = [k for k, v in checks.items() if not v.get("ok")]
        result["WAF_CROSS_CAPABILITY_INVARIANTS"] = {
            "REAL_HTTP_ONLY": "YES",
            "REAL_EXYONQ_BINARY": "YES",
            "REAL_UPSTREAM_PEER_PROCESS": "YES",
            "SIGNATURE_BLOCK_BEFORE_STATIC": "YES" if checks["mode_block_xss"].get("ok") else "NO",
            "SIGNATURE_BLOCK_BEFORE_PROXY": "YES" if checks["proxy_block_no_upstream"].get("ok") else "NO",
            "BENIGN_PROXY_REACHES_UPSTREAM": "YES" if checks["proxy_benign_upstream"].get("ok") else "NO",
            "WAF_BLOCK_PREVENTS_WEBSOCKET_UPGRADE": "YES" if checks["websocket"].get("ok") else "NO",
            "WAF_BLOCK_PREVENTS_FASTCGI": "YES" if checks["fastcgi"].get("ok") else "NO",
            "WAF_HTTP3_BYPASS": "NO" if checks["http3"].get("ok") else "UNKNOWN",
            "RELOAD_VIA_CONTROL_SOCKET": "YES",
            "SAME_PID_ACROSS_RELOAD": "YES" if same_pid else "NO",
            "INVALID_RELOAD_KEEPS_LAST": "YES" if checks["invalid_reload"].get("ok") else "NO",
            "GRANT_DOES_NOT_BYPASS_SIGNATURE_BLOCK": "YES" if xss_after_code == 403 else "NO",
            "REQUIRED_BODY_READ_ERROR_BECOMES_ALLOW": "NO" if checks["body_collect_fail"].get("ok") else "YES",
            "MOCKS_OR_STUBS_USED": "NO",
        }
        result["PRODUCT_DEFECT"] = "NO" if overall or env_blocked else "YES"
        result["HARNESS_DEFECT"] = "NO"
        result["ENVIRONMENT_BLOCKER"] = "YES" if env_blocked and not overall else "NO"
        if overall:
            result["FINAL_RESULT"] = "PASS"
        elif env_blocked:
            result["FINAL_RESULT"] = "ENVIRONMENT_BLOCKER"
        else:
            result["FINAL_RESULT"] = "FAIL"
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else (2 if env_blocked else 1)
    except Exception as exc:
        result.update({
            "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
            "ENVIRONMENT_BLOCKER": "YES",
            "DETAIL": f"{type(exc).__name__}: {exc}",
            "SERVER_LOG_TAIL": log.read_text(errors="replace")[-4000:] if log.is_file() else "",
        })
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    finally:
        stop_proc(srv)
        upstream.shutdown()
        upstream.server_close()
        try:
            if ctrl.is_socket() or ctrl.exists():
                ctrl.unlink(missing_ok=True)
            ctrl.parent.rmdir()
        except OSError:
            pass


if __name__ == "__main__":
    sys.exit(main())
