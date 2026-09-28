#!/usr/bin/env python3
"""CAPABILITY_008 = fastcgi — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = fastcgi
  USER_VISIBLE_CONTRACT = Route to FastCGI pool; status mapping; POST
  CONFIG_SURFACE = [[fcgi_pool]]; route.fastcgi
  REAL_EXTERNAL_PEER_TEST = YES (real php-fpm)
  OPEN_DEFECT_IDS = RD-001 (prove/close on current HEAD; historical plan08 closer not Cap008 proof)

Authoritative path: release binary → [[fcgi_pool]]+route.fastcgi → real php-fpm
unix socket → real FastCGI → GET/POST body integrity → status mapping.

EXPLICIT_NON_SCOPE:
  - Cap009 php-fpm product profiles (php/wordpress)
  - Cap007 directory-index primary
  - Cap010+ oci/proxy
  - Cap006 HTTP/3
  - non-product FastCGI peer substitution as acceptance
  - historical plan08 closer as Cap008 proof
  - competitive RPS
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

GET_BODY = b"cap008-fcgi-get-v1"
POST_PREFIX = b"post-body="


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
        if path.is_socket() or path.exists():
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


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: int = 15,
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


def fpm_identity() -> tuple[str, str]:
    if os.geteuid() == 0:
        return "www-data", "www-data"
    return (
        subprocess.check_output(["id", "-un"], text=True).strip(),
        subprocess.check_output(["id", "-gn"], text=True).strip(),
    )


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    result: dict = {
        "FEATURE_ID": "fastcgi",
        "CAPABILITY": "CAPABILITY_008",
        "CAPABILITY_NAME": "fastcgi",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": "Route to FastCGI pool; status mapping; POST",
        "SUPPORTED_BEHAVIOR": "route.fastcgi → [[fcgi_pool]] → real php-fpm; GET/POST integrity; miss/down status mapping",
        "EXPLICIT_NON_SCOPE": [
            "Cap009 php-fpm product profiles",
            "Cap007 directory-index primary",
            "Cap010+ oci/proxy",
            "Cap006 HTTP/3",
            "non-product FastCGI peer substitution as acceptance",
            "historical plan08 closer as Cap008 proof",
            "competitive RPS",
        ],
        "OPEN_DEFECT_CONTEXT": "RD-001",
        "REAL_PHP_FPM_PROCESS": "YES",
        "REAL_FCGI_SOCKET": "YES",
        "CAP008_CROSS_PEER_NOTE": "NO_STUB_FCGI",
        "PLATFORM_NOTE": {
            "LINUX_EVIDENCE_HOST": "YES",
            "RELEASE_BINARY": "YES",
            "CAPABILITY_009_STARTED": "NO",
        },
    }

    if not BINARY.is_file():
        result.update({"FINAL_RESULT": "ENVIRONMENT_BLOCKER", "DETAIL": f"missing {BINARY}"})
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2

    php_fpm = find_php_fpm()
    if not php_fpm:
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "missing php-fpm (required real FastCGI peer)",
                "ENVIRONMENT_DEFECT": "YES",
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 3

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    result["PHP_FPM_BIN"] = php_fpm

    # php-fpm drops privileges. A document root under /root is not traversable
    # by that user, and PHP reports the miss as "File not found."
    tmp = Path(tempfile.mkdtemp(prefix="cap008-fcgi-", dir="/tmp"))
    # php-fpm often runs as www-data; mkdtemp is 0700 — open parents for traversal.
    os.chmod(tmp, 0o755)
    try:
        os.chmod(EV, 0o755)
    except OSError:
        pass
    www = tmp / "www"
    www.mkdir()
    os.chmod(www, 0o755)
    sock = tmp / "php-fpm.sock"
    fpm_log = tmp / "fpm.log"
    fpm_user, fpm_group = fpm_identity()

    (www / "index.php").write_text(
        "<?php\nheader('Content-Type: text/plain');\necho 'cap008-fcgi-get-v1';\n"
    )
    (www / "query.php").write_text(
        "<?php\nheader('Content-Type: text/plain');\necho $_GET['name'] ?? 'missing';\n"
    )
    (www / "post.php").write_text(
        "<?php\nheader('Content-Type: text/plain');\necho 'post-body=' . file_get_contents('php://input');\n"
    )
    # Containment: path escape must not read outside document_root
    outside = tmp / "outside"
    outside.mkdir()
    os.chmod(outside, 0o755)
    (outside / "secret.php").write_text("<?php echo 'OUTSIDE-ROOT-SECRET-CAP008';\n")
    for p in list(www.iterdir()) + [outside / "secret.php"]:
        os.chmod(p, 0o644)
    # Ensure entire EV/tmp chain is world-traversable/readable for www-data.
    subprocess.run(["chmod", "-R", "a+rX", str(tmp)], check=False)

    fpm_cfg = tmp / "php-fpm.conf"
    fpm_cfg.write_text(
        f"""[global]
error_log = {fpm_log}
daemonize = no
[www]
user = {fpm_user}
group = {fpm_group}
listen = {sock}
listen.owner = {fpm_user}
listen.group = {fpm_group}
listen.mode = 0666
pm = static
pm.max_children = 4
clear_env = no
"""
    )

    port = pick_port()
    cfg = tmp / "exyonq.toml"
    cfg.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{port}"
routes = ["php"]
[[route]]
name = "php"
match = {{ path = "/" }}
fastcgi = "php"
[[fcgi_pool]]
name = "php"
address = "{sock}"
document_root = "{www}"
max_concurrency = 8
"""
    )
    result["CONFIG_SHA256"] = sha256_file(cfg)

    fpm = subprocess.Popen(
        [php_fpm, "--nodaemonize", "--fpm-config", str(fpm_cfg)],
        stdout=(tmp / "fpm.out").open("w"),
        stderr=subprocess.STDOUT,
    )
    log = EV / "exyonq-cap008.log"
    proc = None
    try:
        if not wait_sock(sock):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "php-fpm socket missing",
                    "FPM_LOG_TAIL": (fpm_log.read_text(errors="replace") if fpm_log.exists() else "")[
                        -2000:
                    ],
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
        )
        if not wait_listen(port):
            result.update(
                {
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": "exyonq listener not ready",
                    "LOG_TAIL": log.read_text(errors="replace")[-4000:] if log.exists() else "",
                }
            )
            OUT.write_text(json.dumps(result, indent=2) + "\n")
            return 3

        base = f"http://127.0.0.1:{port}"

        # POSITIVE GET + query
        c_get, b_get = curl_req(f"{base}/index.php")
        get_ok = c_get == 200 and b_get == GET_BODY and sha256_bytes(b_get) == sha256_bytes(GET_BODY)
        c_q, b_q = curl_req(f"{base}/query.php?name=cap008")
        query_ok = c_q == 200 and b_q == b"cap008"

        # POSITIVE POST
        post_payload = b"alpha=1&beta=two"
        c_post, b_post = curl_req(
            f"{base}/post.php",
            method="POST",
            data=post_payload,
            headers=["Content-Type: application/x-www-form-urlencoded"],
        )
        want_post = POST_PREFIX + post_payload
        post_ok = c_post == 200 and b_post == want_post and sha256_bytes(b_post) == sha256_bytes(want_post)
        positive = get_ok and query_ok and post_ok

        # NEGATIVE: missing script under FastCGI route — product maps resolve miss → 502
        c_miss, b_miss = curl_req(f"{base}/definitely-missing-cap008.php")
        neg_ok = c_miss == 502 and b"cap008-fcgi-get-v1" not in b_miss and b"OUTSIDE-ROOT-SECRET-CAP008" not in b_miss

        # BOUNDARY: path traversal must not execute outside document_root
        body_path = EV / f"trav-{time.time_ns()}.body"
        trav = subprocess.run(
            [
                "curl",
                "-sS",
                "--path-as-is",
                "--max-time",
                "10",
                "-o",
                str(body_path),
                "-w",
                "%{http_code}",
                f"{base}/../outside/secret.php",
            ],
            capture_output=True,
            text=True,
        )
        b_trav = body_path.read_bytes() if body_path.is_file() else b""
        try:
            body_path.unlink(missing_ok=True)
        except OSError:
            pass
        c_trav = int(trav.stdout.strip()) if (trav.stdout or "").strip().isdigit() else -1
        leak = b"OUTSIDE-ROOT-SECRET-CAP008" in b_trav
        # FastCGI InvalidPath → 502; never 200 with outside bytes
        boundary_ok = (not leak) and c_trav == 502

        # FAILURE: stop FPM → 502
        fpm.send_signal(signal.SIGTERM)
        try:
            fpm.wait(timeout=10)
        except Exception:
            fpm.kill()
        fpm = None
        time.sleep(0.4)
        c_down, _ = curl_req(f"{base}/index.php", timeout=5)
        fail_ok = c_down == 502

        # Restart FPM for concurrency (fresh socket)
        if sock.exists():
            try:
                sock.unlink()
            except OSError:
                pass
        fpm = subprocess.Popen(
            [php_fpm, "--nodaemonize", "--fpm-config", str(fpm_cfg)],
            stdout=(tmp / "fpm.out2").open("w"),
            stderr=subprocess.STDOUT,
        )
        if not wait_sock(sock):
            conc_ok = False
        else:
            time.sleep(0.2)

            def one(_i: int) -> bool:
                code, body = curl_req(f"{base}/index.php")
                return code == 200 and body == GET_BODY

            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as ex:
                conc = list(ex.map(one, range(8)))
            conc_ok = len(conc) == 8 and all(conc)

        overall = (
            positive
            and neg_ok
            and fail_ok
            and boundary_ok
            and conc_ok
            and proc.poll() is None
        )

        # Classify: amd64 www-data + 0700 tmp is harness/env, not product.
        product_defect = "NO"
        harness_defect = "NO"
        if not overall:
            if not positive and c_get == 404 and b"File not found" in (b_get + b_q + b_post):
                harness_defect = "YES"
                product_defect = "NO"
            else:
                product_defect = "YES"

        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_E2E" if overall else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": product_defect if not overall else "NO",
                "HARNESS_DEFECT": harness_defect if not overall else "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "CAP008_POSITIVE_STATUS": "PASS" if positive else "FAIL",
                "CAP008_NEGATIVE_STATUS": "PASS" if neg_ok else "FAIL",
                "CAP008_FAILURE_STATUS": "PASS" if fail_ok else "FAIL",
                "CAP008_BOUNDARY_STATUS": "PASS" if boundary_ok else "FAIL",
                "CAP008_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS" if conc_ok else "FAIL",
                "CAP008_CROSS_CAPABILITY_INVARIANTS": {
                    "FCGI_DOCUMENT_ROOT_ESCAPE_BYTES_REACHABLE": "NO" if boundary_ok else "YES_LEAK",
                    "REAL_PHP_FPM_PEER": "YES",
                    "PRODUCT_FCGI_PEER_ONLY": "YES",
                },
                "BODY_SHA256_STATUS": "PASS" if (get_ok and post_ok) else "FAIL",
                "checks": {
                    "get": {"code": c_get, "ok": get_ok},
                    "query": {"code": c_q, "ok": query_ok, "body": b_q.decode("utf-8", "replace")},
                    "post": {"code": c_post, "ok": post_ok},
                    "missing": {"code": c_miss, "ok": neg_ok},
                    "traversal": {"code": c_trav, "leak": leak, "ok": boundary_ok},
                    "fpm_down_502": {"code": c_down, "ok": fail_ok},
                    "concurrency": {"ok": conc_ok},
                },
            }
        )
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 0 if overall else 1
    finally:
        if proc is not None and proc.poll() is None:
            proc.send_signal(signal.SIGTERM)
            try:
                proc.wait(timeout=10)
            except Exception:
                proc.kill()
        if fpm is not None and fpm.poll() is None:
            fpm.send_signal(signal.SIGTERM)
            try:
                fpm.wait(timeout=10)
            except Exception:
                fpm.kill()


if __name__ == "__main__":
    sys.exit(main())
