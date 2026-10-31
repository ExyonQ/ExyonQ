#!/usr/bin/env python3
"""CRS FILES / FILES_NAMES / FILES_COMBINED_SIZE / XML differential.

Same custom SecRule set, same raw HTTP requests, real ExyonQ WAF and real
ModSecurity v3 (owasp/modsecurity-crs nginx image). Compares the enforcement
decision (403 vs pass) and the upstream hit for every case.
"""
import argparse
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
WORK = Path("/tmp/crs-files-xml-diff")
ORIGIN_PORT = 19130
EXY_PORT = 18131
REF_PORT = 18141
CONTAINER = "crs-files-xml-ref"

ORIGIN_HITS = 0
ORIGIN_LOCK = threading.Lock()

RULES = r"""
SecRule FILES "@rx \.php$" "id:900001,phase:2,deny,status:403,msg:'file php'"
SecRule FILES_NAMES "@rx ^upload$" "id:900002,phase:2,deny,status:403,msg:'field upload'"
SecRule FILES_COMBINED_SIZE "@gt 100" "id:900003,phase:2,deny,status:403,msg:'files too big'"
SecRule XML:/* "@rx <script>" "id:900004,phase:2,deny,status:403,msg:'xml text'"
SecRule XML://@* "@rx evil" "id:900005,phase:2,deny,status:403,msg:'xml attr'"
SecRule XML:/* "@rx alert" "id:900006,phase:2,deny,status:403,msg:'xml alert text'"
"""


def multipart(fields, boundary="XyZb0undary"):
    out = bytearray()
    for field, filename, content, ctype in fields:
        out += f"--{boundary}\r\n".encode()
        disposition = f'Content-Disposition: form-data; name="{field}"'
        if filename is not None:
            disposition += f'; filename="{filename}"'
        out += (disposition + "\r\n").encode()
        out += f"Content-Type: {ctype}\r\n\r\n".encode()
        out += content + b"\r\n"
    out += f"--{boundary}--\r\n".encode()
    return f"multipart/form-data; boundary={boundary}", bytes(out)


def cases():
    clean_mp, clean_body = multipart([("doc", "note.txt", b"hello", "text/plain")])
    php_mp, php_body = multipart(
        [("doc", "shell.php", b"<?php echo 1; ?>", "application/octet-stream")]
    )
    field_mp, field_body = multipart([("upload", "note.txt", b"hello", "text/plain")])
    big_mp, big_body = multipart(
        [
            ("f1", "a.bin", b"A" * 60, "application/octet-stream"),
            ("f2", "b.bin", b"B" * 60, "application/octet-stream"),
        ]
    )
    binary_mp, binary_body = multipart(
        [("blob", "data.bin", bytes(range(64)), "application/octet-stream")]
    )
    return [
        {
            "name": "multipart_php_filename",
            "ctype": php_mp,
            "body": php_body,
            "block": True,
        },
        {
            "name": "multipart_upload_field",
            "ctype": field_mp,
            "body": field_body,
            "block": True,
        },
        {
            "name": "multipart_combined_size",
            "ctype": big_mp,
            "body": big_body,
            "block": True,
        },
        {
            "name": "multipart_clean",
            "ctype": clean_mp,
            "body": clean_body,
            "block": False,
        },
        {
            "name": "multipart_binary",
            "ctype": binary_mp,
            "body": binary_body,
            "block": False,
        },
        {
            # XML:/* exposes element text, not markup: a <script> element is
            # text "alert(1)", so a rule looking for literal "<script>" must not match.
            "name": "xml_markup_not_text",
            "ctype": "application/xml",
            "body": b"<root><item><script>x</script></item></root>",
            "block": False,
        },
        {
            "name": "xml_text_alert",
            "ctype": "application/xml",
            "body": b"<root><item>alert(1)</item></root>",
            "block": True,
        },
        {
            "name": "xml_attr_evil",
            "ctype": "application/xml",
            "body": b'<root id="evil">ok</root>',
            "block": True,
        },
        {
            "name": "xml_clean",
            "ctype": "application/xml",
            "body": b"<root><item>hello</item></root>",
            "block": False,
        },
        {
            "name": "xml_xxe_not_expanded",
            "ctype": "application/xml",
            "body": (
                b'<!DOCTYPE r [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>'
                b"<r>&xxe;</r>"
            ),
            "block": False,
        },
        {
            "name": "urlencoded_not_files",
            "ctype": "application/x-www-form-urlencoded",
            "body": b"doc=shell.php",
            "block": False,
        },
    ]


def origin_server():
    class Handler(socketserver.BaseRequestHandler):
        def handle(self):
            global ORIGIN_HITS
            data = b""
            try:
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
                        size = int(line, 16)
                        if size == 0:
                            break
                        while len(buf) < size + 2:
                            buf += self.request.recv(65536)
                        buf = buf[size + 2 :]
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
    """Docker publishes the host port before nginx listens; wait for real HTTP."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            sock = socket.create_connection(("127.0.0.1", port), timeout=2)
            sock.sendall(b"GET / HTTP/1.1\r\nHost: crs.test\r\nConnection: close\r\n\r\n")
            data = sock.recv(64)
            sock.close()
            if data.startswith(b"HTTP/"):
                return
        except OSError:
            pass
        time.sleep(0.3)
    raise RuntimeError(f"port {port} did not answer HTTP")


def send(port: int, ctype: str, body: bytes) -> int:
    for attempt in range(3):
        try:
            return _send_once(port, ctype, body)
        except OSError:
            if attempt == 2:
                raise
            time.sleep(0.5)
    raise RuntimeError("unreachable")


def _send_once(port: int, ctype: str, body: bytes) -> int:
    sock = socket.create_connection(("127.0.0.1", port), timeout=15)
    try:
        head = (
            f"POST / HTTP/1.1\r\nHost: crs.test\r\nContent-Type: {ctype}\r\n"
            f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n"
        ).encode()
        sock.sendall(head + body)
        data = b""
        while b"\r\n" not in data:
            chunk = sock.recv(65536)
            if not chunk:
                break
            data += chunk
        if not data:
            return 0
        return int(data.split(b"\r\n")[0].split(b" ")[1])
    finally:
        sock.close()


def exy_config() -> str:
    return f"""config_version = 1
[waf]
enabled = true
mode = "block"
[[waf.ruleset]]
id = "files-xml"
rules_file = "{WORK / 'rules.conf'}"
inbound_threshold = 0
[waf.ruleset.builtins]
sql_injection = "off"
xss = "off"
path_traversal = "off"
command_injection = "off"
header_anomaly = "off"
bot_ua = "off"
ip_filter = {{ enabled = false }}
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


def start_exy() -> subprocess.Popen:
    require_free_port(EXY_PORT)
    exy = subprocess.Popen(
        [str(ROOT / "target/debug/exyonq"), "serve", "-c", str(WORK / "exy.toml")],
        env={**os.environ, "EXYONQ_CONTROL_SOCKET": "/tmp/crs-files-xml.sock"},
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    wait_port(EXY_PORT)
    return exy


def start_reference() -> None:
    subprocess.run(
        ["docker", "rm", "-f", CONTAINER],
        check=False,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
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
            "-v", f"{WORK / 'rules.conf'}:/etc/modsecurity.d/custom/rules.conf:ro",
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


def main() -> int:
    global ORIGIN_HITS
    parser = argparse.ArgumentParser()
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()

    shutil.rmtree(WORK, ignore_errors=True)
    WORK.mkdir(parents=True)
    (WORK / "rules.conf").write_text(RULES)
    (WORK / "exy.toml").write_text(exy_config())
    (WORK / "setup.conf.template").write_text(
        "Include /etc/modsecurity.d/modsecurity.conf\n"
        "Include /etc/modsecurity.d/custom/*.conf\n"
    )

    origin = origin_server()
    exy = start_exy()
    start_reference()
    failures = 0
    try:
        for case in cases():
            with ORIGIN_LOCK:
                ORIGIN_HITS = 0
            exy_status = send(EXY_PORT, case["ctype"], case["body"])
            with ORIGIN_LOCK:
                exy_hits = ORIGIN_HITS
            with ORIGIN_LOCK:
                ORIGIN_HITS = 0
            ref_status = send(REF_PORT, case["ctype"], case["body"])
            with ORIGIN_LOCK:
                ref_hits = ORIGIN_HITS
            expected = 403 if case["block"] else 200
            exy_ok = exy_status == expected and exy_hits == (0 if case["block"] else 1)
            ref_ok = ref_status == expected and ref_hits == (0 if case["block"] else 1)
            ok = exy_ok and ref_ok
            failures += 0 if ok else 1
            print(
                f"{case['name']}: {'PASS' if ok else 'FAIL'} "
                f"exyonq={exy_status}/hits={exy_hits} "
                f"modsecurity={ref_status}/hits={ref_hits} "
                f"expected={expected}"
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
    print(f"failures={failures}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
