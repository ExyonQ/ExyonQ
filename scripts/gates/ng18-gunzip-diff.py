#!/usr/bin/env python3
"""NG-18 / ADR-064 Phase A — differential gunzip corpus.

Runs the same upstream and request matrix against real NGINX and (optionally)
an ExyonQ binary. Writes a machine-readable oracle and a diff report.

Test-only. Canonical Linux evidence must run on Netcup (amd64) and Oracle
(arm64); a Mac/Docker run is iteration, not evidence.
"""
import argparse
import gzip
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from http.client import HTTPConnection, IncompleteRead, RemoteDisconnected
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
FIXTURES = ROOT / "tests/fixtures/nginx/ng18/gunzip"
UPSTREAM_PORT = 19090
CONTEXTS = {
    "http": (18081, "/"),
    "server": (18082, "/"),
    "location": (18083, "/"),
    "inherit_on": (18084, "/"),
    "inherit_off": (18084, "/off/"),
    "inherit_off_on": (18085, "/on/"),
    "inherit_off_base": (18085, "/"),
}
PATHS = ["/gzip", "/gzip-large", "/plain", "/invalid", "/empty"]
ENCODINGS = [
    "identity",
    "gzip",
    "gzip;q=0",
    "*",
    "br, gzip;q=0.5",
    None,
]
HEADERS_OF_INTEREST = [
    "content-encoding",
    "content-length",
    "transfer-encoding",
    "vary",
    "etag",
    "last-modified",
    "cache-control",
]


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def decode_body(raw: bytes, encoding: str | None) -> tuple[str, str]:
    """Return (decoded_sha256, note). Gzip bodies are decoded for comparison."""
    if encoding and "gzip" in encoding.lower():
        try:
            return sha(gzip.decompress(raw)), "gzip-decoded"
        except Exception as exc:  # invalid stream is a legitimate oracle row
            return sha(raw), f"gzip-invalid:{exc}"
    return sha(raw), "identity"


def fetch(port: int, path: str, encoding: str | None, method: str = "GET") -> dict:
    conn = HTTPConnection("127.0.0.1", port, timeout=10)
    headers = {"Host": "ng18.test", "Connection": "close"}
    if encoding is not None:
        headers["Accept-Encoding"] = encoding
    try:
        conn.request(method, path, headers=headers)
        resp = conn.getresponse()
        raw = resp.read()
        hdrs = {name: resp.getheader(name) for name in HEADERS_OF_INTEREST}
        decoded, note = decode_body(raw, hdrs.get("content-encoding"))
        return {
            "status": resp.status,
            "headers": hdrs,
            "body_sha256": sha(raw),
            "body_len": len(raw),
            "decoded_sha256": decoded,
            "decode_note": note,
        }
    except (
        RemoteDisconnected,
        IncompleteRead,
        ConnectionResetError,
        BrokenPipeError,
        TimeoutError,
        socket.timeout,
    ):
        # Error paths are part of the contract (invalid gzip, upstream reset).
        # NGINX closes before headers; ExyonQ aborts mid-body — both are a
        # connection abort, so the corpus normalizes the error class.
        return {
            "error": "disconnect",
            "status": None,
            "headers": {},
            "body_sha256": None,
            "body_len": None,
            "decoded_sha256": None,
            "decode_note": "disconnect",
        }
    finally:
        conn.close()


def wait_port(port: int, timeout: float = 30.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return
        except OSError:
            time.sleep(0.2)
    raise RuntimeError(f"port {port} did not open")


SMALL = b"ng18-gunzip-body\n" * 4096
LARGE = b"ng18-large-body-0123456789\n" * 65536
PLAIN = b"ng18-plain-body\n" * 64


def write_origin_files(work: Path) -> Path:
    root = work / "origin"
    root.mkdir(parents=True, exist_ok=True)
    (root / "gzip").write_bytes(gzip.compress(SMALL, mtime=0))
    (root / "gzip-large").write_bytes(gzip.compress(LARGE, mtime=0))
    (root / "plain").write_bytes(PLAIN)
    (root / "invalid").write_bytes(b"not-a-gzip-stream")
    (root / "empty").write_bytes(gzip.compress(b"", mtime=0))
    return root


def origin_config(root: str) -> str:
    # Exact locations for the root paths and the /off//on/ aliases used by the
    # inheritance contexts (ExyonQ proxies the prefixed path verbatim).
    locations = []
    for prefix in ("", "/off", "/on"):
        for name, gzip in [
            ("gzip", True),
            ("gzip-large", True),
            ("plain", False),
            ("invalid", True),
            ("empty", True),
        ]:
            path = f"{prefix}/{name}" if prefix else f"/{name}"
            ce = "add_header Content-Encoding gzip;" if gzip else ""
            locations.append(
                f"    location = {path} {{ alias {root}/{name}; default_type text/plain; {ce} }}"
            )
    body = "\n".join(locations)
    return f"""worker_processes 1;
pid /tmp/ng18-origin.pid;
error_log /dev/stderr warn;
events {{ worker_connections 256; }}
http {{
  access_log off;
  server {{
    listen 19090;
{body}
  }}
}}
"""


def start_origin(work: Path, network: str | None, nginx_bin: str | None) -> object:
    root = write_origin_files(work)
    if nginx_bin:
        cfg = work / "origin.conf"
        cfg.write_text(origin_config(str(root)))
        proc = subprocess.Popen(
            [nginx_bin, "-c", str(cfg), "-p", str(work)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.STDOUT,
        )
        wait_port(UPSTREAM_PORT)
        return proc

    cfg = work / "origin.conf"
    cfg.write_text(origin_config("/srv"))
    subprocess.run(["docker", "rm", "-f", "ng18-origin"], check=False,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    args = [
        "docker", "run", "--rm", "-d",
        "--name", "ng18-origin",
        "--network", network,
        "-p", f"{UPSTREAM_PORT}:{UPSTREAM_PORT}",
        "-v", f"{cfg}:/etc/nginx/nginx.conf:ro",
        "-v", f"{root}:/srv:ro",
        "nginx:1.27-alpine",
    ]
    cid = subprocess.check_output(args, text=True).strip()
    wait_port(UPSTREAM_PORT)
    return cid


def render_config(name: str, upstream: str, dest: Path) -> Path:
    text = (FIXTURES / name).read_text().replace("__UPSTREAM__", upstream)
    path = dest / name
    path.write_text(text)
    return path


def start_nginx(work: Path, image: str, nginx_bin: str | None, network: str | None = None) -> list:
    names = {
        "nginx-http.conf": 18081,
        "nginx-server.conf": 18082,
        "nginx-location.conf": 18083,
        "nginx-inheritance.conf": 18084,
    }
    if nginx_bin:
        procs = []
        for name, port in names.items():
            path = render_config(name, "127.0.0.1:19090", work)
            proc = subprocess.Popen(
                [nginx_bin, "-c", str(path), "-p", str(work)],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.STDOUT,
            )
            procs.append(proc)
            wait_port(port)
            if name == "nginx-inheritance.conf":
                wait_port(18085)
        return procs

    # One container per config keeps the corpus readable and avoids an include
    # wrapper that would change directive scope.
    containers = []
    upstream_addr = "ng18-origin:19090" if network else "127.0.0.1:19090"
    for name, port in names.items():
        path = render_config(name, upstream_addr, work)
        args = [
            "docker", "run", "--rm", "-d",
            "-p", f"{port}:{port}",
        ]
        if network:
            args += ["--network", network]
        else:
            args += ["--add-host=host.docker.internal:host-gateway"]
        if name == "nginx-inheritance.conf":
            args += ["-p", "18085:18085"]
        args += ["-v", f"{path}:/etc/nginx/nginx.conf:ro", image]
        cid = subprocess.check_output(args, text=True).strip()
        containers.append(cid)
        wait_port(port)
        if name == "nginx-inheritance.conf":
            wait_port(18085)
    return containers


def stop_containers(ids: list) -> None:
    for item in ids:
        if isinstance(item, subprocess.Popen):
            item.terminate()
            try:
                item.wait(timeout=5)
            except subprocess.TimeoutExpired:
                item.kill()
        else:
            subprocess.run(["docker", "rm", "-f", item], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def exyonq_config(global_enabled: bool, routes: list[tuple[str, str, bool | None]]) -> str:
    lines = [
        "config_version = 1",
        "",
        "[[server]]",
        "listen = \"127.0.0.1:0\"",  # replaced by caller
        "routes = [" + ", ".join(f'\"{name}\"' for name, _, _ in routes) + "]",
        "",
    ]
    for name, path, gunzip in routes:
        lines += [
            "[[route]]",
            f"name = \"{name}\"",
            f"match = {{ path = \"{path}\" }}",
            'upstream = "ng18"',
        ]
        if gunzip is not None:
            lines.append(f"gunzip = {str(gunzip).lower()}")
        lines.append("")
    lines += [
        "[[upstream]]",
        'name = "ng18"',
        'target = "http://127.0.0.1:19090"',
        "",
        "[modules.gunzip]",
        f"enabled = {str(global_enabled).lower()}",
        "buffer_bytes = 32768",
        "",
    ]
    return "\n".join(lines)


def start_exyonq(work: Path, binary: str) -> list[subprocess.Popen]:
    specs = {
        18091: (True, [("root", "/", None)]),
        18092: (False, [("root", "/", True)]),
        18093: (False, [("root", "/", True)]),
        18094: (True, [("root", "/", None), ("off", "/off/", False)]),
        18095: (False, [("root", "/", False), ("on", "/on/", True)]),
    }
    procs = []
    for port, (global_enabled, routes) in specs.items():
        cfg = exyonq_config(global_enabled, routes).replace(
            'listen = "127.0.0.1:0"', f'listen = "127.0.0.1:{port}"'
        )
        path = work / f"exyonq-{port}.toml"
        path.write_text(cfg)
        env = dict(os.environ)
        env["EXYONQ_CONTROL_SOCKET"] = f"/tmp/exyonq-ng18-{port}.sock"
        env["EXYONQ_CACHE_PURGE_SOCKET"] = f"/tmp/exyonq-ng18-purge-{port}.sock"
        log = open(work / f"exyonq-{port}.log", "wb")
        proc = subprocess.Popen(
            [binary, "serve", "-c", str(path)],
            stdout=log,
            stderr=subprocess.STDOUT,
            env=env,
        )
        procs.append(proc)
        wait_port(port)
    return procs


def run_matrix(port: int, prefix: str, path_prefix: str = "") -> dict:
    rows = {}
    for path in PATHS:
        request_path = path_prefix + path
        for encoding in ENCODINGS:
            key = f"{prefix}|{path}|{encoding or 'absent'}|GET"
            rows[key] = fetch(port, request_path, encoding, "GET")
            head_key = f"{prefix}|{path}|{encoding or 'absent'}|HEAD"
            rows[head_key] = fetch(port, request_path, encoding, "HEAD")
    return rows


def compare(oracle: dict, actual: dict) -> tuple[int, list[str]]:
    mismatches = []
    for key, want in oracle.items():
        got = actual.get(key)
        if got is None:
            mismatches.append(f"{key}: missing")
            continue
        if want.get("error") or got.get("error"):
            if want.get("error") != got.get("error"):
                mismatches.append(
                    f"{key}: error {want.get('error')!r} != {got.get('error')!r}"
                )
            continue
        if want["status"] != got["status"]:
            mismatches.append(f"{key}: status {want['status']} != {got['status']}")
        we = want["headers"].get("content-encoding")
        ge = got["headers"].get("content-encoding")
        if we != ge:
            mismatches.append(f"{key}: content-encoding {we!r} != {ge!r}")
        if want["decoded_sha256"] != got["decoded_sha256"]:
            mismatches.append(
                f"{key}: decoded body differs ({want['decode_note']} vs {got['decode_note']})"
            )
        if want["headers"].get("vary") != got["headers"].get("vary"):
            mismatches.append(
                f"{key}: vary {want['headers'].get('vary')!r} != {got['headers'].get('vary')!r}"
            )
    return len(mismatches), mismatches


def main() -> int:
    global FIXTURES
    ap = argparse.ArgumentParser()
    ap.add_argument("--nginx-image", default="nginx:1.27-alpine")
    ap.add_argument("--nginx-bin", default=None)
    ap.add_argument("--fixtures", default=str(FIXTURES))
    ap.add_argument("--exyonq-bin", default=None)
    ap.add_argument("--out", default=None)
    ap.add_argument("--oracle-only", action="store_true")
    args = ap.parse_args()

    FIXTURES = Path(args.fixtures)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    out = Path(args.out) if args.out else (
        ROOT / ".exyonq-local/evidence/ng18-gunzip" / stamp
    )
    out.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="ng18-gunzip-"))
    upstream = None
    containers: list = []
    network = None
    exyonq = None
    try:
        if args.nginx_bin:
            upstream = start_origin(work, None, args.nginx_bin)
        else:
            network = f"ng18-net-{stamp.lower()}"
            subprocess.check_output(["docker", "network", "create", network], text=True)
            upstream = start_origin(work, network, None)
        containers = start_nginx(work, args.nginx_image, args.nginx_bin, network)
        oracle = run_matrix(18081, "http")
        oracle.update(run_matrix(18082, "server"))
        oracle.update(run_matrix(18083, "location"))
        oracle.update(run_matrix(18084, "inherit_on"))
        oracle.update(run_matrix(18084, "inherit_off", "/off"))
        oracle.update(run_matrix(18085, "inherit_off_on", "/on"))
        oracle.update(run_matrix(18085, "inherit_off_base"))
        (out / "oracle.json").write_text(json.dumps(oracle, indent=2, sort_keys=True))

        if args.oracle_only or not args.exyonq_bin:
            (out / "README.md").write_text(
                "# NG-18 gunzip differential — oracle only\n\n"
                f"NGINX: {args.nginx_bin or args.nginx_image}\n\n"
                "No ExyonQ binary supplied; this run captures the real NGINX\n"
                "behaviour for the Phase B implementation.\n"
            )
            print(f"oracle: {out / 'oracle.json'}")
            return 0

        exyonq = start_exyonq(work, args.exyonq_bin)
        actual = {}
        actual.update(run_matrix(18091, "http"))
        actual.update(run_matrix(18092, "server"))
        actual.update(run_matrix(18093, "location"))
        actual.update(run_matrix(18094, "inherit_on"))
        actual.update(run_matrix(18094, "inherit_off", "/off"))
        actual.update(run_matrix(18095, "inherit_off_on", "/on"))
        actual.update(run_matrix(18095, "inherit_off_base"))
        (out / "exyonq.json").write_text(json.dumps(actual, indent=2, sort_keys=True))
        mismatches, details = compare(oracle, actual)
        lines = [
            "# NG-18 gunzip differential",
            "",
            f"- NGINX: {args.nginx_bin or args.nginx_image}",
            f"- ExyonQ: {args.exyonq_bin}",
            f"- Rows: {len(oracle)}",
            f"- Mismatches: {mismatches}",
            "",
        ]
        if details:
            lines += ["```"] + details + ["```"]
        (out / "diff.md").write_text("\n".join(lines) + "\n")
        print(f"diff: {mismatches} mismatches -> {out / 'diff.md'}")
        return 1 if mismatches else 0
    finally:
        stop_containers(containers)
        if exyonq:
            for proc in exyonq:
                proc.terminate()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
        if isinstance(upstream, subprocess.Popen):
            upstream.terminate()
        elif isinstance(upstream, str):
            subprocess.run(["docker", "rm", "-f", upstream], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if network:
            subprocess.run(["docker", "network", "rm", network], check=False,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
