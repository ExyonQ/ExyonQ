#!/usr/bin/env python3
"""CAPABILITY_016 = control-http-api - real product E2E.

Real binary, real process, real TCP Control API, real data-plane listener.
No mocks, no synthetic success. Darwin/local runs are iteration only; Linux
closure requires run-cap-016-dual-linux.sh on Netcup amd64 and Oracle arm64.
"""
from __future__ import annotations

import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
TOKEN = "exq_sk_v1_cap016_e2e_token_do_not_log"
TOKEN_HEADER = "X-ExyonQ-Server-Token"
MARKER = b"cap016-control-api-data-plane-marker-v1\n"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


def wait_port(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.1)
    return False


def curl(
    url: str,
    *,
    method: str = "GET",
    token: str | None = TOKEN,
    data: bytes | None = None,
    headers: dict[str, str] | None = None,
    timeout: int = 15,
) -> tuple[int, bytes, dict[str, str], str]:
    tag = f"{time.time_ns()}-{hashlib.sha256((method + url).encode()).hexdigest()[:8]}"
    body_path = EV / f"curl-{tag}.body"
    header_path = EV / f"curl-{tag}.headers"
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-o",
        str(body_path),
        "-D",
        str(header_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
    ]
    if token is not None:
        cmd.extend(["-H", f"{TOKEN_HEADER}: {token}"])
    for k, v in (headers or {}).items():
        cmd.extend(["-H", f"{k}: {v}"])
    data_path = None
    if data is not None:
        data_path = EV / f"curl-{tag}.data"
        data_path.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_path}"])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    raw_headers = header_path.read_text(errors="replace") if header_path.is_file() else ""
    parsed: dict[str, str] = {}
    for line in raw_headers.splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            parsed[k.strip().lower()] = v.strip()
    for p in (body_path, header_path, data_path):
        if p is not None:
            try:
                p.unlink(missing_ok=True)
            except OSError:
                pass
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body, parsed, proc.stderr[-1000:]


def decode_json(body: bytes) -> Any:
    try:
        return json.loads(body.decode("utf-8"))
    except Exception:
        return None


def check(checks: dict[str, Any], name: str, ok: bool, **details: Any) -> None:
    checks[name] = {"ok": bool(ok), **details}


def write_config(cfg: Path, listen_port: int, site_root: Path) -> None:
    cfg.write_text(
        f'''
config_version = 1

[[server]]
listen = "127.0.0.1:{listen_port}"
routes = ["site"]

[[route]]
name = "site"
match = {{ path = "/site/" }}
root = "{site_root}"
'''.lstrip(),
        encoding="utf-8",
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict[str, Any] = {
        "FEATURE_ID": "control-http-api",
        "CAPABILITY": "CAPABILITY_016",
        "CAPABILITY_ID": "016",
        "CAPABILITY_NAME": "control-http-api",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Control HTTP API mutates runtime vhost config and reloads real data plane",
        "REAL_BINARY": "YES",
        "REAL_PROCESS": "YES",
        "REAL_HTTP_CONTROL_API": "YES",
        "REAL_DATA_PLANE_HTTP": "YES",
        "USES_MOCKS": "NO",
        "USES_SMOKE": "NO",
        "TOKEN_VALUE_LOGGED": "NO",
        "BINARY_SHA256": "MISSING",
        "EXYONQ_BINARY_SHA256": "MISSING",
        "FAILED_CHECKS": [],
        "FINAL_RESULT": "FAIL",
        "checks": {},
    }
    checks = result["checks"]

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "FAIL", "ENVIRONMENT_BLOCKER": "YES", "DETAIL": f"missing binary {BINARY}", "FAILED_CHECKS": ["binary_missing"]})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    result["BINARY_SHA256"] = sha256_file(BINARY)
    result["EXYONQ_BINARY_SHA256"] = result["BINARY_SHA256"]

    with tempfile.TemporaryDirectory(prefix="exyonq-cap016-") as td:
        tmp = Path(td)
        listen_port = pick_port()
        api_port = pick_port()
        site_root = tmp / "site-root"
        vhost_root = tmp / "vhost-root"
        site_root.mkdir()
        vhost_root.mkdir()
        (site_root / "index.html").write_bytes(b"cap016-base-site\n")
        (vhost_root / "index.html").write_bytes(MARKER)
        cfg = tmp / "exyonq-cap016.toml"
        ctrl_sock = tmp / "ops-control.sock"
        api_sock = tmp / "http-api.sock"
        write_config(cfg, listen_port, site_root)
        log = EV / "exyonq-cap016-control-api.log"
        env = os.environ.copy()
        env.update(
            {
                "EXYONQ_CONFIG": str(cfg),
                "EXYONQ_CONTROL_SOCKET": str(ctrl_sock),
                "EXYONQ_API_TCP": f"127.0.0.1:{api_port}",
                "EXYONQ_API_SOCKET": str(api_sock),
                "EXYONQ_SERVER_TOKEN": TOKEN,
            }
        )
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        api = f"http://127.0.0.1:{api_port}"
        data = f"http://127.0.0.1:{listen_port}"
        domain = f"cap016-{os.getpid()}.example.test"
        stale_etag = 'W/"rev-0"'
        srv_stopped = False
        try:
            ready = wait_port(api_port) and wait_port(listen_port) and srv.poll() is None
            check(checks, "process_listeners_ready", ready, api_port=api_port, listen_port=listen_port)
            if not ready:
                result["FINAL_RESULT"] = "FAIL"
                result["ENVIRONMENT_BLOCKER"] = "YES"
                result["FAILED_CHECKS"] = ["process_listeners_ready"]
                result["SERVER_LOG_TAIL"] = log.read_text(errors="replace")[-4000:] if log.exists() else ""
                OUT.write_text(json.dumps(result, indent=2) + "\n")
                return 2

            code, body, _, _ = curl(f"{api}/api/v1/health")
            health = decode_json(body)
            check(checks, "health_with_token", code == 200 and health and health.get("status") == "ok", code=code, body=health)

            code, body, _, _ = curl(f"{api}/api/v1/stats")
            stats = decode_json(body)
            check(checks, "stats_generation_revision", code == 200 and isinstance(stats, dict) and "generation" in stats and "revision" in stats, code=code, body=stats)

            code, body, hdrs, _ = curl(f"{api}/api/v1/config")
            config = decode_json(body)
            first_etag = hdrs.get("etag", "")
            check(checks, "config_etag", code == 200 and first_etag.startswith('W/"rev-') and first_etag.endswith('"'), code=code, etag=first_etag, body=config)

            code, body, _, _ = curl(f"{api}/api/v1/health", token=None)
            err = decode_json(body)
            check(checks, "auth_missing_token_401", code == 401 and err and err.get("code") == "EXY-CTRL-0004", code=code, body=err)

            code, body, _, _ = curl(f"{api}/api/v1/health", token="exq_sk_v1_wrong_cap016")
            err = decode_json(body)
            check(checks, "auth_wrong_token_401", code == 401 and err and err.get("code") == "EXY-CTRL-0004", code=code, body=err)

            code, body, _, _ = curl(f"{api}/api/v1/health", token="not_exq_family")
            err = decode_json(body)
            check(checks, "auth_invalid_request_token_401", code == 401 and err and err.get("code") == "EXY-CTRL-0004", code=code, body=err)

            create = json.dumps({"domain": domain, "path": "/", "root": str(vhost_root)}).encode()
            code, body, hdrs, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=create, headers={"Content-Type": "application/json"})
            created = decode_json(body)
            post_etag = hdrs.get("etag", "")
            check(checks, "create_vhost_201", code == 201 and created and created.get("domain") == domain and created.get("root") == str(vhost_root), code=code, etag=post_etag, body=created)

            code, body, _, _ = curl(f"{data}/", token=None, headers={"Host": domain})
            check(checks, "data_plane_created_vhost", code == 200 and MARKER in body, code=code, body_sha256=hashlib.sha256(body).hexdigest())

            code, body, _, _ = curl(f"{data}/", token=None, headers={"Host": f"other-{domain}"})
            check(
                checks,
                "data_plane_wrong_host_isolated",
                code != 200 or MARKER not in body,
                code=code,
                body_sha256=hashlib.sha256(body).hexdigest(),
            )

            code, body, _, _ = curl(f"{data}/site/", token=None, headers={"Host": "127.0.0.1"})
            check(checks, "data_plane_base_site_still_served", code == 200 and b"cap016-base-site" in body, code=code)

            code, body, _, _ = curl(f"{api}/api/v1/config")
            cfg_after = decode_json(body)
            vhosts = (cfg_after or {}).get("vhosts") if isinstance(cfg_after, dict) else None
            created_view = next((v for v in (vhosts or []) if isinstance(v, dict) and v.get("domain") == domain), None)
            check(
                checks,
                "config_reports_created_vhost",
                created_view is not None
                and created_view.get("path") == "/"
                and created_view.get("root") == str(vhost_root),
                body=created_view,
            )

            code, body, _, _ = curl(
                f"{api}/api/v1/config/vhosts",
                method="POST",
                data=create,
                headers={"Content-Type": "application/json", "If-Match": "garbage"},
            )
            err = decode_json(body)
            check(checks, "create_malformed_if_match_412", code == 412, code=code, body=err)

            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=create, headers={"Content-Type": "application/json"})
            err = decode_json(body)
            check(checks, "duplicate_create_409", code == 409 and err and err.get("code") == "EXY-CTRL-0001", code=code, body=err)

            invalid = json.dumps({"domain": "invalid-body.example.test", "path": "/"}).encode()
            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=invalid, headers={"Content-Type": "application/json"})
            err = decode_json(body)
            check(checks, "invalid_body_400", code == 400, code=code, body=err)

            malformed = b'{"domain": "broken.example.test", "root": '
            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=malformed, headers={"Content-Type": "application/json"})
            err = decode_json(body)
            check(checks, "malformed_json_400", code == 400, code=code, body=err)

            listen_change = json.dumps({"domain": "listen-change.example.test", "listen": "127.0.0.1:1", "path": "/", "root": str(vhost_root)}).encode()
            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=listen_change, headers={"Content-Type": "application/json"})
            err = decode_json(body)
            check(checks, "listen_change_attempt_400", code == 400, code=code, body=err)

            oversized = b'{"domain":"big.example.test","path":"/","root":"' + (b"x" * (1024 * 1024 + 32)) + b'"}'
            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts", method="POST", data=oversized, headers={"Content-Type": "application/json"}, timeout=20)
            err = decode_json(body)
            check(checks, "oversized_body_413", code == 413, code=code, body=err)

            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts/{domain}", method="DELETE")
            err = decode_json(body)
            check(checks, "delete_missing_if_match_412", code == 412, code=code, body=err)

            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts/{domain}", method="DELETE", headers={"If-Match": stale_etag})
            err = decode_json(body)
            check(checks, "delete_stale_if_match_412", code == 412, code=code, body=err)

            code, _, hdrs, _ = curl(f"{api}/api/v1/config")
            delete_etag = hdrs.get("etag", "")
            check(checks, "config_etag_after_create", code == 200 and delete_etag.startswith('W/"rev-'), code=code, etag=delete_etag)

            code, body, _, _ = curl(f"{api}/api/v1/config/vhosts/missing-{domain}", method="DELETE", headers={"If-Match": delete_etag})
            err = decode_json(body)
            check(checks, "delete_unknown_domain_404", code == 404, code=code, body=err)

            code, body, hdrs, _ = curl(f"{api}/api/v1/config/vhosts/{domain}", method="DELETE", headers={"If-Match": delete_etag})
            deleted = decode_json(body)
            final_etag = hdrs.get("etag", "")
            check(checks, "delete_vhost_200", code == 200 and deleted and final_etag.startswith('W/"rev-'), code=code, etag=final_etag, body=deleted)

            code, body, _, _ = curl(f"{data}/", token=None, headers={"Host": domain})
            check(
                checks,
                "data_plane_after_delete_not_served",
                code == 404 and MARKER not in body,
                code=code,
                body_sha256=hashlib.sha256(body).hexdigest(),
            )

            check(
                checks,
                "control_socket_independent_of_api_socket",
                str(api_sock) != str(ctrl_sock) and api_sock.name != ctrl_sock.name,
                api_sock=str(api_sock),
                ctrl_sock=str(ctrl_sock),
            )

            # Kill switch: omit Control API env → data plane still serves; API bind absent.
            if srv.poll() is None:
                srv.send_signal(signal.SIGTERM)
                try:
                    srv.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    srv.kill()
                    srv.wait(timeout=5)
            srv_stopped = True

            kill_log = EV / "exyonq-cap016-kill-switch.log"
            kill_env = os.environ.copy()
            kill_env.update(
                {
                    "EXYONQ_CONFIG": str(cfg),
                    "EXYONQ_CONTROL_SOCKET": str(ctrl_sock),
                }
            )
            for key in ("EXYONQ_API_TCP", "EXYONQ_API_SOCKET", "EXYONQ_SERVER_TOKEN"):
                kill_env.pop(key, None)
            kill_srv = subprocess.Popen(
                [str(BINARY), "serve", "--config", str(cfg)],
                stdout=kill_log.open("w"),
                stderr=subprocess.STDOUT,
                cwd=str(WS),
                env=kill_env,
            )
            try:
                kill_ready = wait_port(listen_port) and kill_srv.poll() is None
                check(checks, "kill_switch_dataplane_starts_without_api", kill_ready)
                code, body, _, _ = curl(f"{data}/site/", token=None, headers={"Host": "127.0.0.1"})
                check(
                    checks,
                    "kill_switch_dataplane_serves_without_api",
                    code == 200 and b"cap016-base-site" in body,
                    code=code,
                )
                api_refused = False
                try:
                    with socket.create_connection(("127.0.0.1", api_port), timeout=1.0):
                        api_refused = False
                except OSError:
                    api_refused = True
                check(checks, "kill_switch_api_port_not_bound", api_refused, api_port=api_port)
            finally:
                if kill_srv.poll() is None:
                    kill_srv.send_signal(signal.SIGTERM)
                    try:
                        kill_srv.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        kill_srv.kill()
                        kill_srv.wait(timeout=5)

            failed = [name for name, item in checks.items() if not item.get("ok")]
            result["FAILED_CHECKS"] = failed
            result["FINAL_RESULT"] = "PASS" if not failed else "FAIL"
            result["SERVER_LOG_TAIL"] = log.read_text(errors="replace")[-4000:] if log.exists() else ""
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 0 if not failed else 1
        finally:
            if not srv_stopped and srv.poll() is None:
                srv.send_signal(signal.SIGTERM)
                try:
                    srv.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    srv.kill()
                    srv.wait(timeout=5)


if __name__ == "__main__":
    sys.exit(main())
