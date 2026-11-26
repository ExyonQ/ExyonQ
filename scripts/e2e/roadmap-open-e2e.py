#!/usr/bin/env python3
"""Real native E2E for the still-open Roadmap 0.9.2 rows.

These scenarios use the built `exyonq` listener and real sockets. The upstream
is a real HTTP/1.1 peer; no request or response is synthesized in the product
path. Run on Linux, with an isolated work directory and control socket.
"""

import argparse
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path


def log(message):
    print(message, flush=True)


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def wait_port(port, timeout=10.0):
    end = time.time() + timeout
    while time.time() < end:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return True
        except OSError:
            time.sleep(0.05)
    return False


class Run:
    def __init__(self, binary, work):
        self.binary = str(Path(binary).resolve())
        self.work = Path(work)
        self.work.mkdir(parents=True, exist_ok=True)
        self.control = self.work / "control.sock"
        self.config = self.work / "exyonq.toml"
        self.log_path = self.work / "exyonq.log"
        self.proc = None
        self.port = None

    def write_config(self, body):
        self.config.write_text(body)
        return self.config

    def start(self, config, rust_log="info"):
        env = os.environ.copy()
        env["TMPDIR"] = str(self.work)
        env["EXYONQ_CONTROL_SOCKET"] = str(self.control)
        env["RUST_LOG"] = rust_log
        self.log = open(self.log_path, "ab", buffering=0)
        self.proc = subprocess.Popen(
            [self.binary, "serve", "-c", str(config)],
            stdin=subprocess.DEVNULL,
            stdout=self.log,
            stderr=subprocess.STDOUT,
            env=env,
        )
        return self.proc

    def stop(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)
        self.proc = None
        if getattr(self, "log", None):
            self.log.close()
            self.log = None

    def log_text(self):
        return self.log_path.read_text(errors="replace") if self.log_path.exists() else ""


class HttpUpstream:
    def __init__(self):
        self.port = free_port()
        self.requests = []
        self.lock = threading.Lock()
        outer = self

        class Handler(__import__("socketserver").BaseRequestHandler):
            def handle(self):
                sock = self.request
                sock.settimeout(60)
                try:
                    data = b""
                    while b"\r\n\r\n" not in data:
                        chunk = sock.recv(65536)
                        if not chunk:
                            return
                        data += chunk
                    head, _, rest = data.partition(b"\r\n\r\n")
                    lines = head.split(b"\r\n")
                    first = lines[0].split(b" ")
                    path = first[1].decode("latin-1") if len(first) > 1 else "/"
                    headers = {}
                    for line in lines[1:]:
                        if b":" in line:
                            k, v = line.split(b":", 1)
                            headers[k.strip().lower().decode("latin-1")] = v.strip().decode("latin-1")
                    length = int(headers.get("content-length", "0") or 0)
                    body = rest
                    while len(body) < length:
                        chunk = sock.recv(min(65536, length - len(body)))
                        if not chunk:
                            return
                        body += chunk
                    body = body[:length]
                    with outer.lock:
                        outer.requests.append((path, headers, body))

                    if "/slow" in path:
                        time.sleep(2.0)
                    if "/chunked-large" in path:
                        payload = b"Z" * (5 * 1024 * 1024)
                        sock.sendall(
                            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n"
                            b"Connection: close\r\n\r\n"
                        )
                        for off in range(0, len(payload), 1024 * 1024):
                            piece = payload[off : off + 1024 * 1024]
                            sock.sendall(
                                (format(len(piece), "x") + "\r\n").encode()
                                + piece
                                + b"\r\n"
                            )
                        sock.sendall(b"0\r\n\r\n")
                        return
                    if "/large" in path:
                        payload = b"L" * (13_107_200)
                        sock.sendall(
                            (
                                f"HTTP/1.1 200 OK\r\nContent-Length: {len(payload)}\r\n"
                                "Connection: close\r\n\r\n"
                            ).encode()
                            + payload
                        )
                        return
                    if "/body" in path:
                        payload = b"BODY:" + body
                    elif "/two" in path:
                        payload = b"two"
                    elif "/one" in path:
                        payload = b"one"
                    else:
                        payload = b"ok"
                    conn = "close"
                    sock.sendall(
                        (
                            f"HTTP/1.1 200 OK\r\nContent-Length: {len(payload)}\r\n"
                            f"Connection: {conn}\r\n\r\n"
                        ).encode()
                        + payload
                    )
                except (OSError, ValueError):
                    return

        import socketserver

        self.server = socketserver.ThreadingTCPServer(("127.0.0.1", self.port), Handler)
        self.server.daemon_threads = True
        self.server.allow_reuse_address = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def read_response(sock, buffered=b"", timeout=30):
    sock.settimeout(timeout)
    data = buffered
    while b"\r\n\r\n" not in data:
        chunk = sock.recv(65536)
        if not chunk:
            return None, b"", b""
        data += chunk
    head, _, rest = data.partition(b"\r\n\r\n")
    lines = head.split(b"\r\n")
    try:
        status = int(lines[0].split(b" ")[1])
    except (IndexError, ValueError):
        return None, b"", b""
    headers = {}
    for line in lines[1:]:
        if b":" in line:
            k, v = line.split(b":", 1)
            headers[k.strip().lower()] = v.strip()
    if headers.get(b"transfer-encoding", b"").lower() == b"chunked":
        body = b""
        while True:
            while b"\r\n" not in rest:
                chunk = sock.recv(65536)
                if not chunk:
                    return status, body, b""
                rest += chunk
            size_line, _, rest = rest.partition(b"\r\n")
            size = int(size_line.split(b";", 1)[0], 16)
            while len(rest) < size + 2:
                chunk = sock.recv(65536)
                if not chunk:
                    return status, body, b""
                rest += chunk
            body += rest[:size]
            rest = rest[size + 2 :]
            if size == 0:
                return status, body, rest
    length = int(headers.get(b"content-length", b"0") or 0)
    while len(rest) < length:
        chunk = sock.recv(65536)
        if not chunk:
            break
        rest += chunk
    return status, rest[:length], rest[length:]


def one_request(
    port, method, path, *, source="127.0.0.1", headers=None, body=b"", timeout=30, auto_host=True
):
    sock = socket.create_connection(("127.0.0.1", port), timeout=5, source_address=(source, 0))
    lines = [f"{method} {path} HTTP/1.1"]
    if auto_host:
        lines.append("Host: open.local")
    lines.append("Connection: close")
    for key, value in headers or []:
        lines.append(f"{key}: {value}")
    if body:
        lines.append(f"Content-Length: {len(body)}")
    sock.sendall(("\r\n".join(lines) + "\r\n\r\n").encode() + body)
    status, payload, _ = read_response(sock, timeout=timeout)
    sock.close()
    return status, payload


def base(port, upstream, *, extra="", modules="", logging="", waf="", route_extra=""):
    return f"""config_version = 1
{modules}
{logging}
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]
{extra}
[[route]]
name = "site"
match = {{ path = "/" }}
upstream = "backend"
{route_extra}
[[upstream]]
name = "backend"
target = "http://127.0.0.1:{upstream.port}"
{waf}
"""


def static_base(port, root, *, extra="", waf=""):
    return f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["site"]
{extra}
[[route]]
name = "site"
match = {{ path = "/" }}
root = "{root}"
{waf}
"""


def start(run, config, port):
    run.start(config)
    if not wait_port(port):
        log(f"START_FAIL\n{run.log_text()[-2000:]}")
        return False
    return True


def waf_block(rules):
    return f"""[waf]
enabled = true
mode = "block"
[[waf.ruleset]]
id = "open-local"
rules_file = "{rules}"
"""


def scenario_f07(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "f07")
    port = free_port()
    config = run.write_config(base(port, up))
    if not start(run, config, port):
        return 1
    sock = socket.create_connection(("127.0.0.1", port), timeout=10)
    sock.sendall(
        b"GET /one HTTP/1.1\r\nHost: open.local\r\n\r\n"
        b"GET /two HTTP/1.1\r\nHost: open.local\r\nConnection: close\r\n\r\n"
    )
    s1, b1, rest = read_response(sock)
    s2, b2, _ = read_response(sock, rest)
    sock.close()
    body_status, body_payload = one_request(port, "GET", "/body", body=b"hi=test")
    sent = [(p, body) for p, _, body in up.requests]
    ok = (
        s1 == 200
        and s2 == 200
        and b1 == b"one"
        and b2 == b"two"
        and body_status == 200
        and body_payload == b"BODY:hi=test"
        and any(p == "/body" and b == b"hi=test" for p, b in sent)
    )
    log(f"F07_PIPELINE_STATUSES={s1},{s2}")
    log(f"F07_PIPELINE_BODIES={b1.decode(errors='replace')},{b2.decode(errors='replace')}")
    log(f"F07_BODY_STATUS={body_status}")
    log(f"F07_BODY_UPSTREAM={body_payload.decode(errors='replace')}")
    run.stop()
    up.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"F07={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_f13(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "f13")
    port = free_port()
    config = run.write_config(base(port, up))
    if not start(run, config, port):
        return 1
    dup, _ = one_request(
        port,
        "GET",
        "/ok",
        headers=[("Host", "a.local"), ("Host", "b.local")],
        auto_host=False,
    )
    te, _ = one_request(
        port,
        "POST",
        "/ok",
        headers=[("Transfer-Encoding", "chunked"), ("Content-Length", "4")],
        body=b"test",
    )
    ok_status, ok_body = one_request(
        port,
        "GET",
        "/ok",
        headers=[("Authorization", "Bearer secret"), ("Cookie", "a=b"), ("Range", "bytes=0-1")],
    )
    paths = [p for p, _, _ in up.requests]
    valid = next(
        (
            (h, b)
            for p, h, b in up.requests
            if p == "/ok" and h.get("authorization")
        ),
        (None, None),
    )
    leaked = bool(valid and valid[0].get("authorization") and valid[0].get("cookie") and valid[0].get("range"))
    ok = (
        dup == 400
        and te in (400, 413)
        and ok_status == 200
        and ok_body == b"ok"
        and leaked
        and paths.count("/ok") == 1
        and "/ambig" not in paths
    )
    log(f"F13_DUPLICATE_HOST_STATUS={dup}")
    log(f"F13_TE_CL_STATUS={te}")
    log(f"F13_VALID_STATUS={ok_status}")
    log(f"F13_UPSTREAM_REQUESTS={len(paths)}")
    log(f"F13_HEADERS_FORWARDED={leaked}")
    run.stop()
    up.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"F13={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq028(binary, work):
    run = Run(binary, work / "exq028")
    www = run.work / "www"
    www.mkdir()
    (www / "403.html").write_text("EXQ028_ERROR_PAGE")
    rules = run.work / "rules.conf"
    rules.write_text('SecRule REQUEST_URI "@contains /blocked" "id:990028,deny"\n')
    port = free_port()
    config = run.write_config(
        static_base(
            port,
            www,
            extra='error_pages = { "403" = "/403.html" }',
            waf=waf_block(rules),
        )
    )
    if not start(run, config, port):
        return 1
    html_status, html_body = one_request(port, "GET", "/blocked", headers=[("Accept", "text/html")])
    any_status, any_body = one_request(port, "GET", "/blocked", headers=[("Accept", "*/*")])
    ok = (
        html_status == 403
        and any_status == 403
        and b"EXQ028_ERROR_PAGE" in html_body
        and b"EXQ028_ERROR_PAGE" in any_body
    )
    log(f"EXQ028_HTML_STATUS={html_status}")
    log(f"EXQ028_HTML_BODY={html_body.decode(errors='replace')}")
    log(f"EXQ028_WILDCARD_STATUS={any_status}")
    log(f"EXQ028_WILDCARD_BODY={any_body.decode(errors='replace')}")
    run.stop()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ028={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq029(binary, work, ctl):
    run = Run(binary, work / "exq029")
    www = run.work / "www"
    www.mkdir()
    (www / "404-a.html").write_text("EXQ029_A")
    (www / "404-b.html").write_text("EXQ029_B")
    port = free_port()
    config = run.write_config(
        static_base(port, www, extra='error_pages = { "404" = "/404-a.html" }')
    )
    if not start(run, config, port):
        return 1
    pre_status, pre = one_request(port, "GET", "/missing-a", headers=[("Accept", "*/*")])
    run.write_config(static_base(port, www, extra='error_pages = { "404" = "/404-b.html" }'))
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(config)
    reload = subprocess.run(
        [ctl, "reload", "--socket", str(run.control)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        env=env,
    )
    time.sleep(0.6)
    post_status, post = one_request(port, "GET", "/missing-b", headers=[("Accept", "*/*")])
    ok = (
        pre_status == 404
        and post_status == 404
        and b"EXQ029_A" in pre
        and b"EXQ029_B" in post
        and reload.returncode == 0
    )
    log(f"EXQ029_PRE_STATUS={pre_status}")
    log(f"EXQ029_PRE_BODY={pre.decode(errors='replace')}")
    log(f"EXQ029_RELOAD_RC={reload.returncode}")
    log(f"EXQ029_POST_STATUS={post_status}")
    log(f"EXQ029_POST_BODY={post.decode(errors='replace')}")
    run.stop()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ029={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq030(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "exq030")
    port = free_port()
    config = run.write_config(base(port, up))
    if not start(run, config, port):
        return 1
    holder = {}

    def slow():
        try:
            holder["status"], holder["body"] = one_request(port, "GET", "/slow", timeout=20)
        except OSError as err:
            holder["error"] = str(err)

    thread = threading.Thread(target=slow)
    thread.start()
    time.sleep(0.3)
    run.proc.terminate()
    time.sleep(0.3)
    new_status = None
    retry = None
    try:
        sock = socket.create_connection(("127.0.0.1", port), timeout=5)
        sock.sendall(b"GET / HTTP/1.1\r\nHost: open.local\r\nConnection: close\r\n\r\n")
        head = b""
        while b"\r\n\r\n" not in head:
            chunk = sock.recv(4096)
            if not chunk:
                break
            head += chunk
        sock.close()
        first = head.split(b"\r\n", 1)[0]
        parts = first.split(b" ")
        new_status = int(parts[1]) if len(parts) > 1 and parts[1].isdigit() else None
        for line in head.split(b"\r\n"):
            if line.lower().startswith(b"retry-after:"):
                retry = line.split(b":", 1)[1].strip().decode()
    except OSError:
        pass
    thread.join(timeout=20)
    try:
        rc = run.proc.wait(timeout=15) if run.proc else None
    except subprocess.TimeoutExpired:
        rc = None
    ok = (
        holder.get("status") == 200
        and holder.get("body") == b"ok"
        and new_status == 503
        and retry == "5"
        and rc == 0
    )
    log(f"EXQ030_INFLIGHT_STATUS={holder.get('status')}")
    log(f"EXQ030_INFLIGHT_BODY={holder.get('body', b'').decode(errors='replace')}")
    log(f"EXQ030_NEW_STATUS={new_status}")
    log(f"EXQ030_RETRY_AFTER={retry}")
    log(f"EXQ030_EXIT={rc}")
    run.proc = None
    run.stop()
    up.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ030={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq031(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "exq031")
    rules = run.work / "rules.conf"
    rules.write_text('SecRule REQUEST_URI "@contains /blocked" "id:990031,deny"\n')
    port = free_port()
    logging = '[logging]\nformat = "json"\n[logging.console]\nenabled = true\n'
    config = run.write_config(base(port, up, logging=logging, waf=waf_block(rules)))
    if not start(run, config, port):
        return 1
    status, _ = one_request(port, "GET", "/blocked", headers=[("Accept", "*/*")])
    time.sleep(0.4)
    text = run.log_text()
    ids = []
    for line in text.splitlines():
        if '"target":"exyonq_waf"' in line and '"request_id"' in line:
            ids.append(re.findall(r"[0-9a-f]{32}", line))
    waf_ids = [x for group in ids for x in group]
    access_ids = []
    for line in text.splitlines():
        if (
            ('"access"' in line or "access event" in line)
            and ("status=403" in line or '"status":403' in line)
        ):
            access_ids.extend(re.findall(r"[0-9a-f]{32}", line))
    common = set(waf_ids) & set(access_ids)
    ok = status == 403 and bool(common) and "/blocked" in text and "open.local" in text
    log(f"EXQ031_STATUS={status}")
    log(f"EXQ031_WAF_IDS={len(waf_ids)}")
    log(f"EXQ031_ACCESS_IDS={len(access_ids)}")
    log(f"EXQ031_CORRELATED={bool(common)}")
    run.stop()
    up.close()
    if not os.environ.get("KEEP_E2E"):
        shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ031={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq032(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "exq032")
    rules = run.work / "rules.conf"
    rules.write_text('SecRule REQUEST_URI "@contains /blocked" "id:990032,deny"\n')
    port = free_port()
    abuse = """[waf.abuse]
mode = "rate_limit"
requests_per_second = 1000
burst = 1000
violation_threshold = 20
violation_window_seconds = 60
ban_seconds = 2
"""
    config = run.write_config(base(port, up, waf=waf_block(rules) + abuse))
    if not start(run, config, port):
        return 1
    first = [one_request(port, "GET", "/blocked", source="127.0.0.1")[0] for _ in range(20)]
    banned = one_request(port, "GET", "/blocked", source="127.0.0.1")[0]
    other = one_request(port, "GET", "/blocked", source="127.0.0.2")[0]
    during = one_request(port, "GET", "/open", source="127.0.0.1")[0]
    time.sleep(2.4)
    after = one_request(port, "GET", "/open", source="127.0.0.1")[0]
    ok = all(x == 403 for x in first) and banned == 429 and other == 403 and during == 429 and after == 200
    log(f"EXQ032_FIRST_403={first.count(403)}")
    log(f"EXQ032_BANNED_STATUS={banned}")
    log(f"EXQ032_OTHER_IP_STATUS={other}")
    log(f"EXQ032_DURING_BAN_STATUS={during}")
    log(f"EXQ032_AFTER_BAN_STATUS={after}")
    run.stop()
    up.close()
    shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ032={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def scenario_exq034(binary, work):
    up = HttpUpstream()
    run = Run(binary, work / "exq034")
    port = free_port()
    modules = (
        "[modules]\nresponse_body_limit_bytes = 4194304\n"
        "[modules.compression]\nenabled = true\n"
        "[modules.sub_filter.routes.site]\n"
        "types = [\"text/plain\"]\n"
        "[[modules.sub_filter.routes.site.rules]]\n"
        "pattern = \"ZZZ\"\n"
        "replacement = \"Z\"\n"
    )
    config = run.write_config(base(port, up, modules=modules))
    if not start(run, config, port):
        return 1
    small_status, small_body = one_request(port, "GET", "/chunked-large", timeout=30)
    large_status, large_body = one_request(port, "GET", "/large", timeout=60)
    ok = (
        small_status == 413
        and (b"response_body_limit" in small_body or b"limit" in small_body)
        and large_status == 200
        and len(large_body) == 13_107_200
    )
    log(f"EXQ034_UNKNOWN_LENGTH_STATUS={small_status}")
    log(f"EXQ034_LIMIT_BODY={small_body[:160].decode(errors='replace')}")
    log(f"EXQ034_LARGE_STATUS={large_status}")
    log(f"EXQ034_LARGE_BYTES={len(large_body)}")
    run.stop()
    up.close()
    if not os.environ.get("KEEP_E2E"):
        shutil.rmtree(run.work, ignore_errors=True)
    log(f"EXQ034={'PASS' if ok else 'FAIL'}")
    return 0 if ok else 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "scenario",
        choices=["f07", "f13", "exq028", "exq029", "exq030", "exq031", "exq032", "exq034", "all"],
    )
    parser.add_argument("--binary", required=True)
    parser.add_argument("--work", required=True)
    parser.add_argument("--exyonqctl", required=True)
    args = parser.parse_args()
    if sys.platform != "linux":
        log("ROADMAP_OPEN_E2E=NOT_APPLICABLE_LINUX_REQUIRED")
        return 3
    work = Path(args.work)
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)
    funcs = {
        "f07": lambda: scenario_f07(args.binary, work),
        "f13": lambda: scenario_f13(args.binary, work),
        "exq028": lambda: scenario_exq028(args.binary, work),
        "exq029": lambda: scenario_exq029(args.binary, work, args.exyonqctl),
        "exq030": lambda: scenario_exq030(args.binary, work),
        "exq031": lambda: scenario_exq031(args.binary, work),
        "exq032": lambda: scenario_exq032(args.binary, work),
        "exq034": lambda: scenario_exq034(args.binary, work),
    }
    names = list(funcs) if args.scenario == "all" else [args.scenario]
    failed = 0
    for name in names:
        rc = funcs[name]()
        if rc != 0:
            failed += 1
    log(f"ROADMAP_OPEN_E2E={'PASS' if failed == 0 else 'FAIL'} tests={len(names)} failed={failed}")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())