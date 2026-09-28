#!/usr/bin/env python3
"""CAPABILITY_009 = php-fpm — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = php-fpm
  USER_VISIBLE_CONTRACT = PHP-FPM unix/tcp via fcgi pools; product profiles php/wordpress
  CONFIG_SURFACE = fcgi_pool.address/transport; product profile

Cap008 FastCGI baseline is CLOSED_VERIFIED_REAL_PRODUCTION — necessary but not sufficient.
Cap009 proves additional product semantics:
  - product profile `php` (exyonqctl profile render)
  - product profile `wordpress` (static wp-includes/wp-content + FCGI app)
  - real php-fpm over unix transport
  - real php-fpm over tcp transport
  - SCRIPT_FILENAME / DOCUMENT_ROOT / PATH_INFO integrity under document_root
  - Cap008 containment invariant preserved

EXPLICIT_NON_SCOPE:
  - Cap008 generic FastCGI-only close as Cap009 proof
  - Cap010 oci-container
  - Cap007 directory-index primary
  - Cap006 HTTP/3
  - competitive RPS
  - historical plan08 closer as Cap009 proof
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

MARKER = b"cap009-php-profile-v1"
WP_FRONT = b"cap009-wp-front-v1"
WP_INC = b"cap009-wp-includes-static-v1"
WP_CONTENT = b"cap009-wp-content-static-v1"
OUTSIDE = b"OUTSIDE-ROOT-SECRET-CAP009"


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


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


def wait_sock(path: Path, timeout: float = 30.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return True
        time.sleep(0.1)
    return False


def find_php_fpm() -> str | None:
    env = os.environ.get("PHP_FPM_BIN")
    if env and Path(env).is_file():
        return env
    for c in ("php-fpm", "php-fpm8.3", "php-fpm8.2", "php-fpm8.1"):
        p = subprocess.run(["bash", "-lc", f"command -v {c}"], capture_output=True, text=True)
        if p.returncode == 0 and p.stdout.strip():
            return p.stdout.strip()
    for p in (Path("/usr/sbin/php-fpm8.3"), Path("/usr/sbin/php-fpm")):
        if p.is_file():
            return str(p)
    return None


def fpm_identity() -> tuple[str, str]:
    if os.geteuid() == 0:
        return "www-data", "www-data"
    return (
        subprocess.check_output(["id", "-un"], text=True).strip(),
        subprocess.check_output(["id", "-gn"], text=True).strip(),
    )


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: int = 15,
    path_as_is: bool = False,
) -> tuple[int, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
    body_path = EV / f"curl-{tag}.body"
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
    if path_as_is:
        cmd.append("--path-as-is")
    if headers:
        for h in headers:
            cmd.extend(["-H", h])
    if data is not None:
        data_file = EV / f"curl-{tag}.data"
        data_file.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_file}"])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    try:
        body_path.unlink(missing_ok=True)
        if data is not None:
            (EV / f"curl-{tag}.data").unlink(missing_ok=True)
    except OSError:
        pass
    code_s = (proc.stdout or "").strip()
    code = int(code_s) if code_s.isdigit() else -1
    return code, body


def open_perms(path: Path) -> None:
    os.chmod(path, 0o755)
    subprocess.run(["chmod", "-R", "a+rX", str(path)], check=False)


def render_profile(
    ctl: Path,
    profile: str,
    *,
    listen: str,
    document_root: Path,
    fpm_address: str,
    fpm_transport: str,
    out_toml: Path,
) -> None:
    cmd = [
        str(ctl),
        "config",
        "profile",
        "render",
        profile,
        "--listen",
        listen,
        "--document-root",
        str(document_root),
        "--fpm-address",
        fpm_address,
        "--fpm-transport",
        fpm_transport,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, cwd=str(WS))
    if proc.returncode != 0:
        raise RuntimeError(f"profile render {profile} failed: {proc.stderr[-2000:]}")
    out_toml.write_text(proc.stdout)
    text = out_toml.read_text()
    # render_product_profile_toml uses Debug quotes: transport = "unix"|"tcp"
    if f'transport = "{fpm_transport}"' not in text:
        raise RuntimeError(f"rendered profile missing transport={fpm_transport}")
    if f'ProductProfile = {profile}' not in text and f"# ProductProfile = {profile}" not in text:
        raise RuntimeError(f"rendered profile missing ProductProfile = {profile}")


def start_fpm_unix(php_fpm: str, sock: Path, conf: Path, log: Path, user: str, group: str) -> subprocess.Popen:
    conf.write_text(
        f"""[global]
error_log = {log}
daemonize = no
[www]
user = {user}
group = {group}
listen = {sock}
listen.owner = {user}
listen.group = {group}
listen.mode = 0666
pm = static
pm.max_children = 4
clear_env = no
"""
    )
    if sock.exists():
        sock.unlink()
    return subprocess.Popen(
        [php_fpm, "--nodaemonize", "--fpm-config", str(conf)],
        stdout=(log.parent / (log.name + ".out")).open("w"),
        stderr=subprocess.STDOUT,
    )


def start_fpm_tcp(
    php_fpm: str, hostport: str, conf: Path, log: Path, user: str, group: str
) -> subprocess.Popen:
    conf.write_text(
        f"""[global]
error_log = {log}
daemonize = no
[www]
user = {user}
group = {group}
listen = {hostport}
listen.allowed_clients = 127.0.0.1
pm = static
pm.max_children = 4
clear_env = no
"""
    )
    return subprocess.Popen(
        [php_fpm, "--nodaemonize", "--fpm-config", str(conf)],
        stdout=(log.parent / (log.name + ".out")).open("w"),
        stderr=subprocess.STDOUT,
    )


def stop_proc(p: subprocess.Popen | None) -> None:
    if p is None:
        return
    if p.poll() is None:
        p.send_signal(signal.SIGTERM)
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "php-fpm",
        "CAPABILITY": "CAPABILITY_009",
        "CAPABILITY_NAME": "php-fpm",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQCTL_BINARY": str(CTL),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "PHP-FPM unix/tcp via fcgi pools; product profiles php/wordpress",
        "SUPPORTED_BEHAVIOR": "php+wordpress product profiles; unix+tcp php-fpm; SCRIPT_FILENAME/DOCUMENT_ROOT/PATH_INFO; Cap008 containment preserved",
        "EXPLICIT_NON_SCOPE": [
            "Cap008 generic FastCGI-only close as Cap009 proof",
            "Cap010 oci-container",
            "Cap007 directory-index primary",
            "Cap006 HTTP/3",
            "competitive RPS",
            "historical plan08 closer as Cap009 proof",
        ],
        "CAP008_FASTCGI_BASELINE": "CLOSED_VERIFIED_REAL_PRODUCTION",
        "OPEN_DEFECT_CONTEXT": "RD-001",
        "REAL_PHP_FPM_PROCESS": "YES",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_010_STARTED": "NO",
        },
    }

    if not BINARY.is_file() or not CTL.is_file():
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": f"missing binary exyonq={BINARY.is_file()} ctl={CTL.is_file()}",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    php_fpm = find_php_fpm()
    if not php_fpm:
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "missing php-fpm",
                "ENVIRONMENT_DEFECT": "YES",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 3

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    result["EXYONQCTL_BINARY_SHA256"] = sha256_file(CTL)
    result["PHP_FPM_BIN"] = php_fpm

    tmp = Path(tempfile.mkdtemp(prefix="cap009-phpfpm-", dir="/tmp"))
    try:
        os.chmod(EV, 0o755)
    except OSError:
        pass
    open_perms(tmp)
    user, group = fpm_identity()

    www = tmp / "www"
    www.mkdir()
    (www / "wp-includes").mkdir()
    (www / "wp-content").mkdir()
    # PHP that proves Cap009 mapping semantics
    (www / "index.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "echo 'cap009-php-profile-v1\\n';\n"
        "echo 'DR=' . ($_SERVER['DOCUMENT_ROOT'] ?? '') . \"\\n\";\n"
        "echo 'SF=' . ($_SERVER['SCRIPT_FILENAME'] ?? '') . \"\\n\";\n"
        "echo 'SN=' . ($_SERVER['SCRIPT_NAME'] ?? '') . \"\\n\";\n"
        "echo 'PI=' . ($_SERVER['PATH_INFO'] ?? '') . \"\\n\";\n"
        "if ($_SERVER['REQUEST_METHOD'] === 'POST') {\n"
        "  echo 'POST=' . file_get_contents('php://input') . \"\\n\";\n"
        "}\n"
    )
    (www / "wp-includes" / "x.js").write_bytes(WP_INC)
    (www / "wp-content" / "x.css").write_bytes(WP_CONTENT)
    (www / "wp-front.php").write_bytes(WP_FRONT)  # unused unless routed
    outside = tmp / "outside"
    outside.mkdir()
    (outside / "secret.php").write_text("<?php echo 'OUTSIDE-ROOT-SECRET-CAP009';\n")
    open_perms(www)
    open_perms(outside)

    checks: dict = {}
    fpm = None
    srv = None
    try:
        # --- A: php profile + unix ---
        sock = tmp / "php-fpm.sock"
        fpm = start_fpm_unix(php_fpm, sock, tmp / "fpm-unix.conf", tmp / "fpm-unix.log", user, group)
        if not wait_sock(sock):
            raise RuntimeError("unix php-fpm socket missing")
        port_u = pick_port()
        cfg_u = tmp / "php-unix.toml"
        render_profile(
            CTL,
            "php",
            listen=f"127.0.0.1:{port_u}",
            document_root=www,
            fpm_address=str(sock),
            fpm_transport="unix",
            out_toml=cfg_u,
        )
        log_u = EV / "exyonq-cap009-unix.log"
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_u)],
            stdout=log_u.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(port_u):
            raise RuntimeError("unix listener not ready")
        base_u = f"http://127.0.0.1:{port_u}"

        c_get, b_get = curl_req(f"{base_u}/index.php")
        www_canon = str(www.resolve())
        get_ok = (
            c_get == 200
            and MARKER in b_get
            and f"DR={www_canon}".encode() in b_get
            and f"SF={www_canon}/index.php".encode() in b_get
            and b"SN=/index.php" in b_get
        )
        c_pi, b_pi = curl_req(f"{base_u}/index.php/extra/path")
        path_info_ok = c_pi == 200 and MARKER in b_pi and b"PI=/extra/path" in b_pi
        post_payload = b"k=v&n=1"
        c_post, b_post = curl_req(
            f"{base_u}/index.php",
            method="POST",
            data=post_payload,
            headers=["Content-Type: application/x-www-form-urlencoded"],
        )
        post_ok = c_post == 200 and MARKER in b_post and b"POST=" + post_payload in b_post

        c_miss, _ = curl_req(f"{base_u}/missing-cap009.php")
        neg_ok = c_miss == 502

        c_trav, b_trav = curl_req(f"{base_u}/../outside/secret.php", path_as_is=True)
        boundary_ok = c_trav == 502 and OUTSIDE not in b_trav

        checks["php_unix"] = {
            "get_mapping": {"code": c_get, "ok": get_ok},
            "path_info": {"code": c_pi, "ok": path_info_ok},
            "post": {"code": c_post, "ok": post_ok},
            "miss_502": {"code": c_miss, "ok": neg_ok},
            "traversal_502": {"code": c_trav, "ok": boundary_ok, "leak": OUTSIDE in b_trav},
            "transport": "unix",
            "profile": "php",
        }
        php_unix_ok = get_ok and path_info_ok and post_ok and neg_ok and boundary_ok

        # concurrency on unix php profile
        def one(_i: int) -> bool:
            code, body = curl_req(f"{base_u}/index.php")
            return code == 200 and MARKER in body

        with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
            conc = list(ex.map(one, range(8)))
        conc_ok = all(conc)
        checks["concurrency"] = {"ok": conc_ok, "n": len(conc)}

        # FPM down failure on unix profile
        stop_proc(fpm)
        fpm = None
        time.sleep(0.4)
        c_down, _ = curl_req(f"{base_u}/index.php", timeout=5)
        fail_ok = c_down == 502
        checks["fpm_down_502"] = {"code": c_down, "ok": fail_ok}

        stop_proc(srv)
        srv = None

        # --- B: php profile + tcp ---
        tcp_port = pick_port()
        fpm = start_fpm_tcp(
            php_fpm, f"127.0.0.1:{tcp_port}", tmp / "fpm-tcp.conf", tmp / "fpm-tcp.log", user, group
        )
        time.sleep(0.5)
        # wait for TCP listen
        if not wait_listen(tcp_port, timeout=20):
            raise RuntimeError("tcp php-fpm not listening")
        port_t = pick_port()
        cfg_t = tmp / "php-tcp.toml"
        render_profile(
            CTL,
            "php",
            listen=f"127.0.0.1:{port_t}",
            document_root=www,
            fpm_address=f"127.0.0.1:{tcp_port}",
            fpm_transport="tcp",
            out_toml=cfg_t,
        )
        log_t = EV / "exyonq-cap009-tcp.log"
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_t)],
            stdout=log_t.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(port_t):
            raise RuntimeError("tcp listener not ready")
        c_tcp, b_tcp = curl_req(f"http://127.0.0.1:{port_t}/index.php")
        tcp_ok = c_tcp == 200 and MARKER in b_tcp and f"DR={www_canon}".encode() in b_tcp
        checks["php_tcp"] = {"code": c_tcp, "ok": tcp_ok, "transport": "tcp", "profile": "php"}
        stop_proc(srv)
        srv = None
        stop_proc(fpm)
        fpm = None

        # --- C: wordpress profile (unix) ---
        sock2 = tmp / "php-fpm-wp.sock"
        fpm = start_fpm_unix(php_fpm, sock2, tmp / "fpm-wp.conf", tmp / "fpm-wp.log", user, group)
        if not wait_sock(sock2):
            raise RuntimeError("wordpress php-fpm socket missing")
        # wordpress front uses index.php via FCGI for app paths; also copy front marker into index for /
        # Keep Cap009 wordpress fixture: static assets + FCGI index.php
        port_w = pick_port()
        cfg_w = tmp / "wordpress.toml"
        render_profile(
            CTL,
            "wordpress",
            listen=f"127.0.0.1:{port_w}",
            document_root=www,
            fpm_address=str(sock2),
            fpm_transport="unix",
            out_toml=cfg_w,
        )
        # Ensure rendered wordpress has static routes
        wp_toml = cfg_w.read_text()
        if "wp-includes" not in wp_toml or "wordpress" not in wp_toml:
            raise RuntimeError("wordpress profile missing expected routes")
        log_w = EV / "exyonq-cap009-wp.log"
        srv = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_w)],
            stdout=log_w.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(port_w):
            raise RuntimeError("wordpress listener not ready")
        base_w = f"http://127.0.0.1:{port_w}"
        c_inc, b_inc = curl_req(f"{base_w}/wp-includes/x.js")
        c_ct, b_ct = curl_req(f"{base_w}/wp-content/x.css")
        c_app, b_app = curl_req(f"{base_w}/index.php")
        wp_static_ok = c_inc == 200 and b_inc == WP_INC and c_ct == 200 and b_ct == WP_CONTENT
        wp_app_ok = c_app == 200 and MARKER in b_app and f"SF={www_canon}/index.php".encode() in b_app
        checks["wordpress"] = {
            "static_includes": {"code": c_inc, "ok": c_inc == 200 and b_inc == WP_INC},
            "static_content": {"code": c_ct, "ok": c_ct == 200 and b_ct == WP_CONTENT},
            "fcgi_app": {"code": c_app, "ok": wp_app_ok},
            "profile": "wordpress",
            "ok": wp_static_ok and wp_app_ok,
        }
        wordpress_ok = wp_static_ok and wp_app_ok

        positive = php_unix_ok and tcp_ok and wordpress_ok
        # wordpress-phase server must still be alive after checks
        overall = positive and fail_ok and conc_ok and (srv is not None and srv.poll() is None)

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if overall else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP009_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP009_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP009_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP009_BOUNDARY_STATUS": "PASS" if boundary_ok else "FAIL",
                "CAP009_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if conc_ok else "FAIL",
                "CAP009_CROSS_CAPABILITY_INVARIANTS": {
                    "FCGI_DOCUMENT_ROOT_ESCAPE_BYTES_REACHABLE": "NO" if boundary_ok else "YES_LEAK",
                    "CAP008_FASTCGI_BASELINE": "CLOSED_VERIFIED_REAL_PRODUCTION",
                    "SCRIPT_FILENAME_UNDER_DOCUMENT_ROOT": "YES" if get_ok else "NO",
                    "PHP_PROFILE_UNIX": "PASS" if php_unix_ok else "FAIL",
                    "PHP_PROFILE_TCP": "PASS" if tcp_ok else "FAIL",
                    "WORDPRESS_PROFILE": "PASS" if wordpress_ok else "FAIL",
                },
                "BODY_SHA256_STATUS": "PASS"
                if (get_ok and wp_static_ok and sha256_bytes(b_inc) == sha256_bytes(WP_INC))
                else "FAIL",
                "checks": checks,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    except Exception as e:
        result.update(
            {
                "FINAL_RESULT": "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "UNKNOWN",
                "HARNESS_DEFECT": "YES",
                "DETAIL": repr(e),
                "checks": checks,
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 1
    finally:
        stop_proc(srv)
        stop_proc(fpm)


if __name__ == "__main__":
    sys.exit(main())
