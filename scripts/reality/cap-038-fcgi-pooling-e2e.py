#!/usr/bin/env python3
"""CAPABILITY_038 = fcgi-pooling — real product E2E (single capability).

Canonical matrix:
  FEATURE_ID = fcgi-pooling
  USER_VISIBLE_CONTRACT = FastCGI ConnPool reuse, max_connections, idle timeout,
    isolation, reload, drain, shutdown — against real php-fpm
  CONFIG_SURFACE = [[fcgi_pool]] max_connections / idle_timeout_ms /
    checkout_timeout_ms / total_timeout_ms / transport
  REAL_EXTERNAL_PEER_TEST = YES (real php-fpm + AcceptCountingProxy)

ZERO_FAKE terminal chain:
  REAL HTTP CLIENT (curl)
  → REAL EXYONQ release binary
  → REAL FastCGI ConnPool
  → REAL OS socket
  → REAL AcceptCountingProxy (external accept proof)
  → REAL php-fpm
  → REAL PHP

Authoritative path: release binary → [[fcgi_pool]]+route.fastcgi → proxy.sock
→ AcceptCountingProxy → fpm.sock → php-fpm. Cache MUST be disabled.

EXPLICIT_NON_SCOPE:
  - Cap008 fastcgi product close (remains CLOSED; baseline only)
  - Cap009 php-fpm product profiles (remains CLOSED)
  - Cap054 fcgi metrics as acceptance authority
  - smoke / demo / in-process FastCGI peer as acceptance
  - Cap039 Hyper HTTP proxy pool (distinct)
  - competitive RPS
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import select
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

SCENARIO_NAMES = [
    "sequential_reuse",
    "max_connections_capacity",
    "idle_timeout",
    "backend_ab_isolation",
    "reload_backend_change",
    "post_side_effect_once",
    "param_isolation",
    "body_isolation",
    "app_500_reusable",
    "empty_response",
    "large_response",
    "backend_restart",
    "drain_interaction",
    "graceful_shutdown_idle",
    "concurrent_under_cap",
    "tcp_transport",
    "client_disconnect",
    "stderr_warning",
]


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
            time.sleep(0.05)
    return False


def wait_sock(path: Path, timeout: float = 30.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket() or path.exists():
            return True
        time.sleep(0.05)
    return False


def stop_proc(p: subprocess.Popen | None, timeout: float = 10.0) -> None:
    if p is None or p.poll() is not None:
        return
    try:
        p.send_signal(signal.SIGTERM)
        p.wait(timeout=timeout)
    except Exception:
        try:
            p.kill()
        except Exception:
            pass


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


def open_perms(path: Path) -> None:
    try:
        os.chmod(path, 0o755)
    except OSError:
        pass
    subprocess.run(["chmod", "-R", "a+rX", str(path)], check=False)


def open_write_perms(path: Path) -> None:
    """php-fpm (often www-data) must create counter.dat under document_root."""
    open_perms(path)
    try:
        os.chmod(path, 0o777)
    except OSError:
        pass
    subprocess.run(["chmod", "-R", "a+rwX", str(path)], check=False)


def prepare_counter_file(www: Path) -> Path:
    """Create counter.dat inside document_root, owned for php-fpm writes.

    Do NOT use host /tmp: php-fpm under systemd often has PrivateTmp, so FPM's
    /tmp is not the harness /tmp.
    """
    counter = www / "counter.dat"
    counter.write_text("")
    os.chmod(counter, 0o666)
    if os.geteuid() == 0:
        try:
            import grp
            import pwd

            uid = pwd.getpwnam("www-data").pw_uid
            gid = grp.getgrnam("www-data").gr_gid
            os.chown(www, uid, gid)
            os.chown(counter, uid, gid)
        except (KeyError, OSError, PermissionError):
            pass
    open_write_perms(www)
    return counter


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    headers: list[str] | None = None,
    timeout: float = 15.0,
) -> tuple[int, bytes]:
    tag = f"{time.time_ns()}-{threading.get_ident()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
    body_path = EV / f"curl-{tag}.body"
    cmd = [
        "curl",
        "-sS",
        "--http1.1",
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


def _pipe_bidirectional(a: socket.socket, b: socket.socket) -> None:
    a.setblocking(False)
    b.setblocking(False)
    sockets = [a, b]
    try:
        while True:
            r, _, x = select.select(sockets, [], sockets, 60.0)
            if x or not r:
                break
            for src in r:
                dst = b if src is a else a
                try:
                    data = src.recv(65536)
                except OSError:
                    return
                if not data:
                    return
                try:
                    dst.sendall(data)
                except OSError:
                    return
    finally:
        for s in (a, b):
            try:
                s.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            try:
                s.close()
            except OSError:
                pass


class UnixAcceptCountingProxy:
    """Unix listen sock → dial upstream unix sock; accept_count on each accept."""

    def __init__(self, listen_path: Path, upstream_path: Path):
        self.listen_path = listen_path
        self.upstream_path = upstream_path
        self.accept_count = 0
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._srv: socket.socket | None = None

    def start(self) -> None:
        if self.listen_path.exists():
            try:
                self.listen_path.unlink()
            except OSError:
                pass
        srv = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(str(self.listen_path))
        try:
            os.chmod(self.listen_path, 0o666)
        except OSError:
            pass
        srv.listen(128)
        srv.settimeout(0.5)
        self._srv = srv
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def _loop(self) -> None:
        assert self._srv is not None
        while not self._stop.is_set():
            try:
                client, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                if self._stop.is_set():
                    break
                continue
            with self._lock:
                self.accept_count += 1
            try:
                upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                upstream.connect(str(self.upstream_path))
            except OSError:
                try:
                    client.close()
                except OSError:
                    pass
                continue
            threading.Thread(
                target=_pipe_bidirectional, args=(client, upstream), daemon=True
            ).start()

    def stop(self) -> None:
        self._stop.set()
        if self._srv is not None:
            try:
                self._srv.close()
            except OSError:
                pass
        if self._thread is not None:
            self._thread.join(timeout=2.0)
        if self.listen_path.exists():
            try:
                self.listen_path.unlink()
            except OSError:
                pass


class TcpAcceptCountingProxy:
    """TCP listen → dial upstream TCP; accept_count on each accept."""

    def __init__(self, listen_host: str, listen_port: int, up_host: str, up_port: int):
        self.listen_host = listen_host
        self.listen_port = listen_port
        self.up_host = up_host
        self.up_port = up_port
        self.accept_count = 0
        self._lock = threading.Lock()
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._srv: socket.socket | None = None

    def start(self) -> None:
        srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind((self.listen_host, self.listen_port))
        srv.listen(128)
        srv.settimeout(0.5)
        self._srv = srv
        self._thread = threading.Thread(target=self._loop, daemon=True)
        self._thread.start()

    def _loop(self) -> None:
        assert self._srv is not None
        while not self._stop.is_set():
            try:
                client, _ = self._srv.accept()
            except socket.timeout:
                continue
            except OSError:
                if self._stop.is_set():
                    break
                continue
            with self._lock:
                self.accept_count += 1
            try:
                upstream = socket.create_connection((self.up_host, self.up_port), timeout=5.0)
            except OSError:
                try:
                    client.close()
                except OSError:
                    pass
                continue
            threading.Thread(
                target=_pipe_bidirectional, args=(client, upstream), daemon=True
            ).start()

    def stop(self) -> None:
        self._stop.set()
        if self._srv is not None:
            try:
                self._srv.close()
            except OSError:
                pass
        if self._thread is not None:
            self._thread.join(timeout=2.0)


def write_php_scripts(www: Path, identity: str, *, counter_path: Path | None = None) -> None:
    www.mkdir(parents=True, exist_ok=True)
    (www / "ping.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "echo bin2hex(random_bytes(16));\n"
    )
    (www / "sleep.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "$ms = isset($_GET['ms']) ? (int)$_GET['ms'] : 1000;\n"
        "if ($ms < 0) { $ms = 0; }\n"
        "if ($ms > 60000) { $ms = 60000; }\n"
        "usleep($ms * 1000);\n"
        "echo 'slept-' . $ms;\n"
    )
    (www / "identity.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        f"echo 'IDENTITY={identity}';\n"
    )
    # Prefer absolute path under /tmp so www-data can append regardless of
    # document_root parent mode (mkdtemp is often 0700 until chmod).
    if counter_path is not None:
        counter_expr = f"'{counter_path}'"
    else:
        counter_expr = "__DIR__ . '/counter.dat'"
    (www / "counter.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        f"$f = {counter_expr};\n"
        # PHP single-quoted '1\\n' is literal backslash-n; use double quotes.
        '$n = file_put_contents($f, "1\\n", FILE_APPEND | LOCK_EX);\n'
        "echo ($n === false) ? 'append-fail' : 'appended';\n"
    )
    (www / "echo_q.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "echo getenv('QUERY_STRING') !== false ? getenv('QUERY_STRING') : ($_SERVER['QUERY_STRING'] ?? '');\n"
    )
    (www / "echo_body.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "echo file_get_contents('php://input');\n"
    )
    (www / "err500.php").write_text(
        "<?php\n"
        "header('Cache-Control: no-store');\n"
        "http_response_code(500);\n"
        "echo 'fail';\n"
    )
    (www / "empty.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "http_response_code(200);\n"
    )
    (www / "large.php").write_text(
        "<?php\n"
        "header('Content-Type: application/octet-stream');\n"
        "header('Cache-Control: no-store');\n"
        "echo str_repeat('L', 200 * 1024);\n"
    )
    (www / "warn.php").write_text(
        "<?php\n"
        "header('Content-Type: text/plain');\n"
        "header('Cache-Control: no-store');\n"
        "@trigger_error('cap038-warning', E_USER_WARNING);\n"
        "echo 'warn-ok';\n"
    )
    open_perms(www)


def write_fpm_conf(
    conf: Path,
    *,
    log: Path,
    listen: str,
    user: str,
    group: str,
    max_children: int = 8,
    tcp: bool = False,
) -> None:
    listen_extra = ""
    if tcp:
        listen_extra = "listen.allowed_clients = 127.0.0.1\n"
    else:
        listen_extra = (
            f"listen.owner = {user}\n"
            f"listen.group = {group}\n"
            "listen.mode = 0666\n"
        )
    conf.write_text(
        f"""[global]
error_log = {log}
daemonize = no
[www]
user = {user}
group = {group}
listen = {listen}
{listen_extra}pm = static
pm.max_children = {max_children}
clear_env = no
"""
    )


def write_exyonq_cfg(
    path: Path,
    *,
    listen: int,
    pools: list[dict],
    routes: list[dict],
) -> None:
    """pools: name, address, document_root, transport, max_concurrency, max_connections,
    idle_timeout_ms, checkout_timeout_ms, total_timeout_ms
    routes: name, path, fastcgi
    """
    lines = [
        "config_version = 1",
        "",
        "[[server]]",
        f'listen = "127.0.0.1:{listen}"',
        "routes = [" + ", ".join(f'"{r["name"]}"' for r in routes) + "]",
        "",
    ]
    for r in routes:
        lines += [
            "[[route]]",
            f'name = "{r["name"]}"',
            f'match = {{ path = "{r["path"]}" }}',
            f'fastcgi = "{r["fastcgi"]}"',
            "",
        ]
    for p in pools:
        block = [
            "[[fcgi_pool]]",
            f'name = "{p["name"]}"',
            f'address = "{p["address"]}"',
            f'document_root = "{p["document_root"]}"',
            f'transport = "{p.get("transport", "unix")}"',
            f'max_concurrency = {p.get("max_concurrency", 4)}',
        ]
        if p.get("max_connections") is not None:
            block.append(f'max_connections = {p["max_connections"]}')
        block += [
            f'idle_timeout_ms = {p.get("idle_timeout_ms", 30000)}',
            f'checkout_timeout_ms = {p.get("checkout_timeout_ms", 1500)}',
            f'total_timeout_ms = {p.get("total_timeout_ms", 8000)}',
            "",
        ]
        lines += block
    path.write_text("\n".join(lines))


def start_exyonq(cfg: Path, log: Path, ctrl: Path) -> subprocess.Popen:
    env = os.environ.copy()
    env["EXYONQ_CONFIG"] = str(cfg)
    env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
    if ctrl.exists():
        try:
            ctrl.unlink()
        except OSError:
            pass
    return subprocess.Popen(
        [str(BINARY), "serve", "--config", str(cfg)],
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
        cwd=str(WS),
        env=env,
    )


def ctl_cmd(ctrl: Path, op: str, *, cfg: Path | None = None) -> tuple[int, str]:
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
    if cfg is not None:
        env["EXYONQ_CONFIG"] = str(cfg)
    if op == "reload":
        if cfg is None:
            raise ValueError("reload requires cfg")
        cmd = [str(CTL), "reload", "--config", str(cfg), "--socket", str(ctrl)]
    else:
        cmd = [str(CTL), op, "--socket", str(ctrl)]
    proc = subprocess.run(cmd, capture_output=True, text=True, env=env)
    return proc.returncode, (proc.stdout or "") + (proc.stderr or "")


def write_result(result: dict) -> None:
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(result, indent=2, default=str) + "\n")


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    result: dict = {
        "FEATURE_ID": "fcgi-pooling",
        "CAPABILITY": "CAPABILITY_038",
        "CAPABILITY_NAME": "fcgi-pooling",
        "CAP038_INITIAL_ASSESSMENT": "IMPLEMENTATION_ALREADY_EXISTS_NEEDS_REALITY_CLOSE",
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HOSTNAME": socket.gethostname(),
        "UNAME_M": os.uname().machine,
        "KERNEL": f"{os.uname().sysname} {os.uname().release}",
        "HEAD": HEAD,
        "EXYONQ_BINARY": str(BINARY),
        "EXYONQCTL_BINARY": str(CTL),
        "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
        "PRODUCT_CONTRACT": (
            "FastCGI ConnPool: reuse, max_connections, idle timeout, isolation, "
            "reload, drain, shutdown against real php-fpm"
        ),
        "FASTCGI_KEEP_CONN": "YES",
        "FCGI_POOL_KEY": "pool_id:u32",
        "FCGI_MULTIPLEXING": "UNSUPPORTED",
        "FASTCGI_TCP": "SUPPORTED",
        "CACHE_DISABLED": "YES",
        "REAL_PHP_FPM_PROCESS": "YES",
        "REAL_ACCEPT_COUNT_PROOF": "YES",
        "USES_SMOKE": "NO",
        "EXPLICIT_NON_SCOPE": [
            "Cap008 fastcgi remains CLOSED (baseline only)",
            "Cap009 php-fpm profiles remain CLOSED",
            "Cap054 fcgi metrics not required for Cap038 close",
            "smoke / demo / in-process FastCGI peer as acceptance",
            "Cap039 Hyper HTTP proxy pool (distinct)",
            "competitive RPS",
        ],
        "CAP008_REOPEN": "NO",
        "CAP009_REOPEN": "NO",
        "CAPABILITY_054_STARTED": "NO",
    }

    if not BINARY.is_file():
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": f"missing {BINARY}",
                "ZERO_FAKE": "FAIL",
                "PRODUCT_DEFECT": "NO",
                "checks": checks,
            }
        )
        write_result(result)
        return 2

    if not CTL.is_file():
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": f"missing {CTL}",
                "ZERO_FAKE": "FAIL",
                "PRODUCT_DEFECT": "NO",
                "checks": checks,
            }
        )
        write_result(result)
        return 2

    php_fpm = find_php_fpm()
    if not php_fpm:
        result.update(
            {
                "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                "DETAIL": "missing php-fpm (required real FastCGI peer; Darwin expected blocker)",
                "ENVIRONMENT_DEFECT": "YES",
                "ZERO_FAKE": "FAIL",
                "PRODUCT_DEFECT": "NO",
                "checks": checks,
            }
        )
        write_result(result)
        return 3

    result["EXYONQ_BINARY_SHA256"] = sha256_file(BINARY)
    result["PHP_FPM_BIN"] = php_fpm

    tmp = Path(tempfile.mkdtemp(prefix="cap038-fcgi-pool-", dir="/tmp"))
    open_perms(tmp)
    try:
        open_perms(EV)
    except OSError:
        pass

    fpm_user, fpm_group = fpm_identity()
    www = tmp / "www"
    www_a = tmp / "www-a"
    www_b = tmp / "www-b"
    write_php_scripts(www, "PRIMARY")
    write_php_scripts(www_a, "A")
    write_php_scripts(www_b, "B")
    open_perms(tmp)
    open_write_perms(www)
    open_write_perms(www_a)
    open_write_perms(www_b)
    counter_file = prepare_counter_file(www)

    fpm_sock = tmp / "fpm.sock"
    fpm_sock_a = tmp / "fpm-a.sock"
    fpm_sock_b = tmp / "fpm-b.sock"
    proxy_sock = tmp / "proxy.sock"
    proxy_sock_a = tmp / "proxy-a.sock"
    proxy_sock_b = tmp / "proxy-b.sock"
    ctrl = Path(f"/tmp/exq38-{os.getpid()}-{time.time_ns() % 100000}.sock")

    fpm_procs: list[subprocess.Popen] = []
    proxies: list = []
    proc: subprocess.Popen | None = None
    listen = pick_port()
    cfg_path = tmp / "live.toml"
    log = EV / "exyonq-cap038.log"

    def start_fpm_unix(sock: Path, conf_name: str, children: int = 8) -> subprocess.Popen:
        if sock.exists():
            try:
                sock.unlink()
            except OSError:
                pass
        conf = tmp / conf_name
        flog = tmp / (conf_name + ".log")
        write_fpm_conf(
            conf, log=flog, listen=str(sock), user=fpm_user, group=fpm_group, max_children=children
        )
        p = subprocess.Popen(
            [php_fpm, "--nodaemonize", "--fpm-config", str(conf)],
            stdout=(tmp / (conf_name + ".out")).open("w"),
            stderr=subprocess.STDOUT,
        )
        if not wait_sock(sock, timeout=20.0):
            raise RuntimeError(f"php-fpm socket missing: {sock}")
        return p

    def start_proxy(listen_path: Path, upstream: Path) -> UnixAcceptCountingProxy:
        px = UnixAcceptCountingProxy(listen_path, upstream)
        px.start()
        if not wait_sock(listen_path, timeout=5.0):
            raise RuntimeError(f"proxy sock missing: {listen_path}")
        return px

    def restart_exyonq(pools: list[dict], routes: list[dict] | None = None) -> None:
        nonlocal proc, listen
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        time.sleep(0.15)
        listen = pick_port()
        if routes is None:
            routes = [{"name": "php", "path": "/", "fastcgi": pools[0]["name"]}]
        write_exyonq_cfg(cfg_path, listen=listen, pools=pools, routes=routes)
        proc = start_exyonq(cfg_path, log, ctrl)
        if not wait_listen(listen, timeout=45.0) or not wait_sock(ctrl, timeout=20.0):
            raise RuntimeError("exyonq failed to start")

    def base_url() -> str:
        return f"http://127.0.0.1:{listen}"

    def default_pool(**kw) -> dict:
        d = {
            "name": "php",
            "address": str(proxy_sock),
            "document_root": str(www),
            "transport": "unix",
            "max_concurrency": 4,
            "max_connections": 2,
            "idle_timeout_ms": 30000,
            "checkout_timeout_ms": 1500,
            "total_timeout_ms": 8000,
        }
        d.update(kw)
        return d

    try:
        # --- bootstrap primary fpm + proxy ---
        fpm0 = start_fpm_unix(fpm_sock, "fpm.conf", children=8)
        fpm_procs.append(fpm0)
        px0 = start_proxy(proxy_sock, fpm_sock)
        proxies.append(px0)

        # ============================================================
        # 1. sequential_reuse
        # ============================================================
        px0.accept_count = 0
        restart_exyonq([default_pool(max_connections=2, idle_timeout_ms=30000)])
        tokens: list[str] = []
        codes: list[int] = []
        for i in range(12):
            code, body = curl_req(f"{base_url()}/ping.php?i={i}")
            codes.append(code)
            tokens.append(body.decode("utf-8", "replace").strip())
        accepts = px0.accept_count
        seq_ok = (
            all(c == 200 for c in codes)
            and len(set(tokens)) == 12
            and all(len(t) == 32 for t in tokens)
            and accepts <= 3
            and accepts < 12
            and accepts >= 1
        )
        checks["sequential_reuse"] = {
            "ok": seq_ok,
            "request_count": 12,
            "accept_count": accepts,
            "distinct_tokens": len(set(tokens)),
            "CACHE_OFF": "YES",
            "sample_tokens": tokens[:3],
        }

        # ============================================================
        # 2. max_connections_capacity
        # ============================================================
        stop_proc(proc)
        proc = None
        px0.accept_count = 0
        restart_exyonq(
            [
                default_pool(
                    max_connections=1,
                    max_concurrency=4,
                    checkout_timeout_ms=800,
                    total_timeout_ms=8000,
                )
            ]
        )
        hold_result: dict = {"code": -1, "body": b""}
        second_result: dict = {"code": -1, "body": b"", "accepts_during": -1}

        def hold_sleep():
            c, b = curl_req(f"{base_url()}/sleep.php?ms=2500", timeout=10.0)
            hold_result["code"] = c
            hold_result["body"] = b

        t_hold = threading.Thread(target=hold_sleep, daemon=True)
        t_hold.start()
        # Wait until the hold request has opened a real backend connection.
        deadline_hold = time.time() + 3.0
        while time.time() < deadline_hold and px0.accept_count < 1:
            time.sleep(0.05)
        accepts_during = px0.accept_count
        second_result["accepts_during"] = accepts_during

        def second_req():
            c, b = curl_req(f"{base_url()}/ping.php", timeout=8.0)
            second_result["code"] = c
            second_result["body"] = b

        t2 = threading.Thread(target=second_req, daemon=True)
        t2.start()
        t2.join(timeout=10.0)
        t_hold.join(timeout=12.0)
        accepts_after = px0.accept_count
        # Prefer: no second accept while first held (accepts stay 1).
        # Concurrent second: 503 OK, or 200 after first completes with accepts==1 also OK.
        # FAIL if accepts>1 while max_connections=1.
        second_code = second_result["code"]
        cap_ok = (
            hold_result["code"] == 200
            and accepts_during == 1
            and accepts_after == 1
            and (
                second_code == 503
                or second_code == 200
            )
        )
        checks["max_connections_capacity"] = {
            "ok": cap_ok,
            "hold_code": hold_result["code"],
            "second_code": second_code,
            "accepts_during_hold": accepts_during,
            "accepts_after": accepts_after,
            "note": "503 PoolBusy OR wait-then-200 with accepts==1; FAIL if accepts>1",
        }

        # ============================================================
        # 3. idle_timeout
        # ============================================================
        stop_proc(proc)
        proc = None
        px0.accept_count = 0
        restart_exyonq(
            [
                default_pool(
                    max_connections=2,
                    idle_timeout_ms=800,
                    checkout_timeout_ms=1500,
                )
            ]
        )
        c1, _ = curl_req(f"{base_url()}/ping.php")
        a1 = px0.accept_count
        time.sleep(0.3)
        c2, _ = curl_req(f"{base_url()}/ping.php")
        a2 = px0.accept_count
        time.sleep(1.5)
        c3, _ = curl_req(f"{base_url()}/ping.php")
        a3 = px0.accept_count
        idle_ok = (
            c1 == 200
            and c2 == 200
            and c3 == 200
            and a1 >= 1
            and a2 == a1
            and a3 > a2
        )
        checks["idle_timeout"] = {
            "ok": idle_ok,
            "accepts": [a1, a2, a3],
            "codes": [c1, c2, c3],
            "note": "0.3s reuse (accepts unchanged); 1.5s after idle_timeout → new accept",
        }

        # ============================================================
        # 4. backend_ab_isolation
        # ============================================================
        stop_proc(proc)
        proc = None
        for p in fpm_procs:
            stop_proc(p)
        fpm_procs.clear()
        for px in proxies:
            px.stop()
        proxies.clear()

        fpm_a = start_fpm_unix(fpm_sock_a, "fpm-a.conf", children=4)
        fpm_b = start_fpm_unix(fpm_sock_b, "fpm-b.conf", children=4)
        fpm_procs.extend([fpm_a, fpm_b])
        px_a = start_proxy(proxy_sock_a, fpm_sock_a)
        px_b = start_proxy(proxy_sock_b, fpm_sock_b)
        proxies.extend([px_a, px_b])
        # SCRIPT_FILENAME joins full URI under document_root → need /a/ and /b/ trees.
        (www_a / "a").mkdir(exist_ok=True)
        (www_b / "b").mkdir(exist_ok=True)
        for name in (
            "ping.php",
            "identity.php",
            "sleep.php",
            "echo_q.php",
            "echo_body.php",
            "err500.php",
            "empty.php",
            "large.php",
            "warn.php",
            "counter.php",
        ):
            src_a = www_a / name
            src_b = www_b / name
            if src_a.is_file():
                (www_a / "a" / name).write_bytes(src_a.read_bytes())
            if src_b.is_file():
                (www_b / "b" / name).write_bytes(src_b.read_bytes())
        open_perms(www_a)
        open_perms(www_b)
        restart_exyonq(
            [
                default_pool(
                    name="pool_a",
                    address=str(proxy_sock_a),
                    document_root=str(www_a),
                    max_connections=2,
                ),
                default_pool(
                    name="pool_b",
                    address=str(proxy_sock_b),
                    document_root=str(www_b),
                    max_connections=2,
                ),
            ],
            routes=[
                {"name": "ra", "path": "/a/", "fastcgi": "pool_a"},
                {"name": "rb", "path": "/b/", "fastcgi": "pool_b"},
            ],
        )
        px_a.accept_count = 0
        px_b.accept_count = 0
        ab_bodies: list[tuple[str, str]] = []
        ab_ok_flags = []
        for i in range(8):
            if i % 2 == 0:
                code, body = curl_req(f"{base_url()}/a/identity.php")
                text = body.decode("utf-8", "replace")
                ab_bodies.append(("A", text))
                ab_ok_flags.append(code == 200 and "IDENTITY=A" in text and "IDENTITY=B" not in text)
            else:
                code, body = curl_req(f"{base_url()}/b/identity.php")
                text = body.decode("utf-8", "replace")
                ab_bodies.append(("B", text))
                ab_ok_flags.append(code == 200 and "IDENTITY=B" in text and "IDENTITY=A" not in text)
        ab_ok = (
            all(ab_ok_flags)
            and px_a.accept_count >= 1
            and px_b.accept_count >= 1
            and px_a.accept_count < 8
            and px_b.accept_count < 8
        )
        checks["backend_ab_isolation"] = {
            "ok": ab_ok,
            "accepts_a": px_a.accept_count,
            "accepts_b": px_b.accept_count,
            "bodies": ab_bodies[:4],
        }

        # ============================================================
        # 5. reload_backend_change
        # ============================================================
        # Start pointing at A; reload to B.
        stop_proc(proc)
        proc = None
        px_a.accept_count = 0
        px_b.accept_count = 0
        restart_exyonq(
            [
                default_pool(
                    name="php",
                    address=str(proxy_sock_a),
                    document_root=str(www_a),
                    max_connections=2,
                )
            ]
        )
        c_pre, body_pre = curl_req(f"{base_url()}/identity.php")
        accepts_a_pre = px_a.accept_count
        accepts_b_pre = px_b.accept_count
        # rewrite config in place (daemon-bound EXYONQ_CONFIG path)
        write_exyonq_cfg(
            cfg_path,
            listen=listen,
            pools=[
                default_pool(
                    name="php",
                    address=str(proxy_sock_b),
                    document_root=str(www_b),
                    max_connections=2,
                )
            ],
            routes=[{"name": "php", "path": "/", "fastcgi": "php"}],
        )
        rc_rel, out_rel = ctl_cmd(ctrl, "reload", cfg=cfg_path)
        time.sleep(0.3)
        post_bodies = []
        for i in range(4):
            code, body = curl_req(f"{base_url()}/identity.php?r={i}")
            post_bodies.append((code, body.decode("utf-8", "replace")))
        accepts_a_post = px_a.accept_count
        accepts_b_post = px_b.accept_count
        pre_identity = body_pre.decode("utf-8", "replace")
        reload_ok = (
            c_pre == 200
            and "IDENTITY=A" in pre_identity
            and rc_rel == 0
            and all(c == 200 and "IDENTITY=B" in t for c, t in post_bodies)
            and all("IDENTITY=A" not in t for _, t in post_bodies)
            and accepts_b_post > accepts_b_pre
            and accepts_a_post == accepts_a_pre  # A must not serve new accepts after reload
        )
        checks["reload_backend_change"] = {
            "ok": reload_ok,
            "reload_rc": rc_rel,
            "reload_out": out_rel.splitlines()[0] if out_rel else "",
            "pre_identity": pre_identity,
            "post_bodies": post_bodies,
            "accepts_a_pre": accepts_a_pre,
            "accepts_a_post": accepts_a_post,
            "accepts_b_pre": accepts_b_pre,
            "accepts_b_post": accepts_b_post,
        }

        # ============================================================
        # Switch back to primary single fpm/proxy for remaining scenarios
        # ============================================================
        stop_proc(proc)
        proc = None
        for p in fpm_procs:
            stop_proc(p)
        fpm_procs.clear()
        for px in proxies:
            px.stop()
        proxies.clear()

        fpm0 = start_fpm_unix(fpm_sock, "fpm.conf", children=8)
        fpm_procs.append(fpm0)
        px0 = start_proxy(proxy_sock, fpm_sock)
        proxies.append(px0)
        # Reset counter inside document_root (not host /tmp — FPM PrivateTmp).
        counter_file = prepare_counter_file(www)

        restart_exyonq([default_pool(max_connections=2)])

        # ============================================================
        # 6. post_side_effect_once
        # ============================================================
        post_codes = []
        post_bodies = []
        for i in range(5):
            code, body = curl_req(
                f"{base_url()}/counter.php",
                method="POST",
                data=b"x=1",
                headers=["Content-Type: application/x-www-form-urlencoded"],
            )
            post_codes.append(code)
            post_bodies.append(body.decode("utf-8", "replace"))
        lines = counter_file.read_text().splitlines() if counter_file.exists() else []
        post_ok = (
            all(c == 200 for c in post_codes)
            and all(b == "appended" for b in post_bodies)
            and len(lines) == 5
        )
        checks["post_side_effect_once"] = {
            "ok": post_ok,
            "post_codes": post_codes,
            "post_bodies": post_bodies,
            "counter_path": str(counter_file),
            "counter_lines": len(lines),
            "note": "5 POSTs → exactly 5 lines (no replay); counter in document_root",
        }

        # ============================================================
        # 7. param_isolation
        # ============================================================
        param_ok_flags = []
        param_samples = []
        for i, q in enumerate(["alpha=1", "beta=two", "alpha=1", "beta=two"]):
            code, body = curl_req(f"{base_url()}/echo_q.php?{q}")
            text = body.decode("utf-8", "replace")
            param_samples.append(text)
            param_ok_flags.append(code == 200 and text == q)
        checks["param_isolation"] = {
            "ok": all(param_ok_flags),
            "samples": param_samples,
        }

        # ============================================================
        # 8. body_isolation
        # ============================================================
        body_ok_flags = []
        body_samples = []
        for payload in (b"BODY-ONE-AAA", b"BODY-TWO-BBB"):
            code, body = curl_req(
                f"{base_url()}/echo_body.php",
                method="POST",
                data=payload,
                headers=["Content-Type: application/octet-stream"],
            )
            body_samples.append(body.decode("utf-8", "replace"))
            body_ok_flags.append(code == 200 and body == payload)
        checks["body_isolation"] = {
            "ok": all(body_ok_flags),
            "samples": body_samples,
        }

        # ============================================================
        # 9. app_500_reusable
        # ============================================================
        accepts_before_500 = px0.accept_count
        c500, b500 = curl_req(f"{base_url()}/err500.php")
        a_after_500 = px0.accept_count
        c200, b200 = curl_req(f"{base_url()}/ping.php")
        a_after_ok = px0.accept_count
        # Idle pool may already exist: zero new accepts after reset is OK (reuse).
        app500_ok = (
            c500 == 500
            and b"fail" in b500
            and c200 == 200
            and len(b200.strip()) == 32
            and a_after_ok <= accepts_before_500 + 2
        )
        checks["app_500_reusable"] = {
            "ok": app500_ok,
            "code_500": c500,
            "code_next": c200,
            "accepts_after_500": a_after_500,
            "accepts_after_ok": a_after_ok,
        }

        # ============================================================
        # 10. empty_response
        # ============================================================
        c_empty, b_empty = curl_req(f"{base_url()}/empty.php")
        c_next, b_next = curl_req(f"{base_url()}/ping.php")
        empty_ok = c_empty == 200 and b_empty == b"" and c_next == 200 and len(b_next.strip()) == 32
        checks["empty_response"] = {
            "ok": empty_ok,
            "empty_code": c_empty,
            "empty_len": len(b_empty),
            "next_code": c_next,
        }

        # ============================================================
        # 11. large_response
        # ============================================================
        c_large, b_large = curl_req(f"{base_url()}/large.php", timeout=30.0)
        want_sha = sha256_bytes(b"L" * (200 * 1024))
        got_sha = sha256_bytes(b_large)
        c_after_large, _ = curl_req(f"{base_url()}/ping.php")
        large_ok = (
            c_large == 200
            and len(b_large) == 200 * 1024
            and got_sha == want_sha
            and c_after_large == 200
        )
        checks["large_response"] = {
            "ok": large_ok,
            "len": len(b_large),
            "sha256": got_sha,
            "want_sha256": want_sha,
            "next_code": c_after_large,
        }

        # ============================================================
        # 12. backend_restart
        # ============================================================
        c_ok, _ = curl_req(f"{base_url()}/ping.php")
        stop_proc(fpm_procs[0])
        fpm_procs.clear()
        time.sleep(0.3)
        # may briefly 502
        _c_down, _ = curl_req(f"{base_url()}/ping.php", timeout=3.0)
        if fpm_sock.exists():
            try:
                fpm_sock.unlink()
            except OSError:
                pass
        fpm0 = start_fpm_unix(fpm_sock, "fpm.conf", children=8)
        fpm_procs.append(fpm0)
        recovered = False
        last_code = -1
        t0 = time.time()
        while time.time() - t0 < 15.0:
            last_code, body = curl_req(f"{base_url()}/ping.php", timeout=3.0)
            if last_code == 200 and len(body.strip()) == 32:
                recovered = True
                break
            time.sleep(0.25)
        restart_ok = c_ok == 200 and recovered and (proc is not None and proc.poll() is None)
        checks["backend_restart"] = {
            "ok": restart_ok,
            "pre_code": c_ok,
            "recovered": recovered,
            "last_code": last_code,
            "note": "accepts may increase after FPM restart — OK",
        }

        # ============================================================
        # 13. drain_interaction
        # ============================================================
        # Fresh process (not already draining)
        stop_proc(proc)
        proc = None
        px0.accept_count = 0
        restart_exyonq([default_pool(max_connections=2)])
        c_pre_d, _ = curl_req(f"{base_url()}/ping.php")
        rc_d, out_d = ctl_cmd(ctrl, "drain")
        time.sleep(0.15)
        c_drain, b_drain = curl_req(f"{base_url()}/ping.php", timeout=5.0)
        alive = proc is not None and proc.poll() is None
        drain_ok = (
            c_pre_d == 200
            and rc_d == 0
            and c_drain == 503
            and "drain" in b_drain.decode("utf-8", "replace").lower()
            and alive
        )
        checks["drain_interaction"] = {
            "ok": drain_ok,
            "drain_rc": rc_d,
            "product_code_after_drain": c_drain,
            "body_snippet": b_drain[:120].decode("utf-8", "replace"),
            "process_alive": alive,
        }

        # ============================================================
        # 14. graceful_shutdown_idle
        # ============================================================
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        restart_exyonq([default_pool(max_connections=2, idle_timeout_ms=30000)])
        c_idle, _ = curl_req(f"{base_url()}/ping.php")
        assert proc is not None
        t_term = time.time()
        proc.send_signal(signal.SIGTERM)
        try:
            proc.wait(timeout=35.0)
            exit_rc = proc.returncode
            exited = True
        except subprocess.TimeoutExpired:
            proc.kill()
            exit_rc = -1
            exited = False
        elapsed = time.time() - t_term
        shutdown_ok = c_idle == 200 and exited and elapsed <= 35.0
        checks["graceful_shutdown_idle"] = {
            "ok": shutdown_ok,
            "exited": exited,
            "elapsed_s": round(elapsed, 3),
            "exit_rc": exit_rc,
            "note": "idle pool must not block SIGTERM forever",
        }
        proc = None

        # ============================================================
        # 15. concurrent_under_cap
        # ============================================================
        restart_exyonq(
            [
                default_pool(
                    max_connections=3,
                    max_concurrency=6,
                    checkout_timeout_ms=3000,
                    total_timeout_ms=8000,
                )
            ]
        )
        px0.accept_count = 0
        time.sleep(0.1)

        def fast_get(i: int) -> tuple[int, bytes]:
            return curl_req(f"{base_url()}/ping.php?c={i}", timeout=10.0)

        with concurrent.futures.ThreadPoolExecutor(max_workers=6) as ex:
            conc = list(ex.map(fast_get, range(6)))
        conc_accepts = px0.accept_count
        conc_ok = (
            len(conc) == 6
            and all(c == 200 for c, _ in conc)
            and conc_accepts <= 3
            and conc_accepts >= 1
        )
        checks["concurrent_under_cap"] = {
            "ok": conc_ok,
            "codes": [c for c, _ in conc],
            "accept_count": conc_accepts,
            "max_connections": 3,
        }

        # ============================================================
        # 16. tcp_transport
        # ============================================================
        stop_proc(proc)
        proc = None
        for p in fpm_procs:
            stop_proc(p)
        fpm_procs.clear()
        for px in proxies:
            px.stop()
        proxies.clear()

        tcp_fpm_port = pick_port()
        tcp_proxy_port = pick_port()
        tcp_conf = tmp / "fpm-tcp.conf"
        tcp_log = tmp / "fpm-tcp.log"
        write_fpm_conf(
            tcp_conf,
            log=tcp_log,
            listen=f"127.0.0.1:{tcp_fpm_port}",
            user=fpm_user,
            group=fpm_group,
            max_children=4,
            tcp=True,
        )
        fpm_tcp = subprocess.Popen(
            [php_fpm, "--nodaemonize", "--fpm-config", str(tcp_conf)],
            stdout=(tmp / "fpm-tcp.out").open("w"),
            stderr=subprocess.STDOUT,
        )
        fpm_procs.append(fpm_tcp)
        if not wait_listen(tcp_fpm_port, timeout=20.0):
            checks["tcp_transport"] = {
                "ok": False,
                "FASTCGI_TCP_TEST": "FAIL_FPM_TCP_LISTEN",
                "note": "php-fpm TCP listen failed",
            }
        else:
            tcp_proxy = TcpAcceptCountingProxy(
                "127.0.0.1", tcp_proxy_port, "127.0.0.1", tcp_fpm_port
            )
            tcp_proxy.start()
            proxies.append(tcp_proxy)
            time.sleep(0.1)
            restart_exyonq(
                [
                    default_pool(
                        name="php",
                        address=f"127.0.0.1:{tcp_proxy_port}",
                        document_root=str(www),
                        transport="tcp",
                        max_connections=2,
                        idle_timeout_ms=30000,
                    )
                ]
            )
            tcp_proxy.accept_count = 0
            tcp_tokens = []
            tcp_codes = []
            for i in range(8):
                code, body = curl_req(f"{base_url()}/ping.php?tcp={i}")
                tcp_codes.append(code)
                tcp_tokens.append(body.decode("utf-8", "replace").strip())
            tcp_accepts = tcp_proxy.accept_count
            tcp_ok = (
                all(c == 200 for c in tcp_codes)
                and len(set(tcp_tokens)) == 8
                and tcp_accepts <= 3
                and tcp_accepts < 8
                and tcp_accepts >= 1
            )
            checks["tcp_transport"] = {
                "ok": tcp_ok,
                "FASTCGI_TCP_TEST": "PASS" if tcp_ok else "FAIL",
                "request_count": 8,
                "accept_count": tcp_accepts,
                "note": "TCP AcceptCountingProxy → php-fpm TCP; unix remains primary",
            }
            result["FASTCGI_TCP"] = "SUPPORTED"

        # Restore unix fpm/proxy for remaining scenarios
        stop_proc(proc)
        proc = None
        for p in fpm_procs:
            stop_proc(p)
        fpm_procs.clear()
        for px in proxies:
            px.stop()
        proxies.clear()
        fpm0 = start_fpm_unix(fpm_sock, "fpm.conf", children=8)
        fpm_procs.append(fpm0)
        px0 = start_proxy(proxy_sock, fpm_sock)
        proxies.append(px0)
        restart_exyonq([default_pool(max_connections=2, max_concurrency=4)])

        # ============================================================
        # 17. client_disconnect
        # ============================================================
        # Abort early; product may drain-or-discard. Wait for PHP sleep to end so
        # capacity accounting is observable without racing a still-running FCGI.
        def abort_sleep():
            subprocess.run(
                [
                    "curl",
                    "-sS",
                    "--max-time",
                    "0.3",
                    f"{base_url()}/sleep.php?ms=1200",
                ],
                capture_output=True,
            )

        t_abort = threading.Thread(target=abort_sleep, daemon=True)
        t_abort.start()
        t_abort.join(timeout=5.0)
        time.sleep(1.5)
        c_after_abort, b_after_abort = curl_req(f"{base_url()}/ping.php", timeout=5.0)

        # Prove capacity after cleanup: concurrent sleep + get under max_connections=2
        hold2: dict = {"code": -1}

        def hold2_fn():
            c, _ = curl_req(f"{base_url()}/sleep.php?ms=800", timeout=5.0)
            hold2["code"] = c

        th = threading.Thread(target=hold2_fn, daemon=True)
        th.start()
        deadline_h2 = time.time() + 2.0
        while time.time() < deadline_h2 and px0.accept_count < 1:
            time.sleep(0.05)
        c_conc, _ = curl_req(f"{base_url()}/ping.php", timeout=5.0)
        th.join(timeout=6.0)
        disc_ok = (
            c_after_abort == 200
            and len(b_after_abort.strip()) == 32
            and c_conc == 200
            and hold2["code"] == 200
            and (proc is not None and proc.poll() is None)
        )
        checks["client_disconnect"] = {
            "ok": disc_ok,
            "after_abort_code": c_after_abort,
            "concurrent_get_code": c_conc,
            "hold_code": hold2["code"],
            "note": "curl --max-time 0.3 abort; wait PHP end; subsequent GET + concurrent capacity OK",
        }

        # ============================================================
        # 18. stderr_warning
        # ============================================================
        c_warn, b_warn = curl_req(f"{base_url()}/warn.php")
        c_after_warn, b_after_warn = curl_req(f"{base_url()}/ping.php")
        warn_ok = (
            c_warn == 200
            and b"warn-ok" in b_warn
            and c_after_warn == 200
            and len(b_after_warn.strip()) == 32
        )
        checks["stderr_warning"] = {
            "ok": warn_ok,
            "warn_code": c_warn,
            "warn_body": b_warn[:80].decode("utf-8", "replace"),
            "next_code": c_after_warn,
            "note": "do not require STDERR discard; next request must succeed",
        }

        # --- finalize ---
        missing = [n for n in SCENARIO_NAMES if n not in checks]
        for n in missing:
            checks[n] = {"ok": False, "detail": "NOT_RUN"}

        all_ok = all(bool(checks[n].get("ok")) for n in SCENARIO_NAMES) and not missing
        zero_fake = (
            "PASS"
            if all_ok
            and result.get("REAL_PHP_FPM_PROCESS") == "YES"
            and result.get("REAL_ACCEPT_COUNT_PROOF") == "YES"
            and result.get("CACHE_DISABLED") == "YES"
            else "FAIL"
        )
        result.update(
            {
                "FINAL_RESULT": "PASS_REAL_PRODUCTION" if all_ok else "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO" if all_ok else "YES",
                "HARNESS_DEFECT": "NO",
                "ENVIRONMENT_BLOCKER": "NO",
                "ZERO_FAKE": zero_fake,
                "CACHE_DISABLED": "YES",
                "checks": checks,
                "SCENARIO_COUNT": len(SCENARIO_NAMES),
                "SCENARIOS_PASSED": sum(1 for n in SCENARIO_NAMES if checks[n].get("ok")),
            }
        )
        write_result(result)
        return 0 if all_ok else 1

    except Exception as exc:
        import traceback

        result.update(
            {
                "FINAL_RESULT": "FAIL_REAL_E2E",
                "PRODUCT_DEFECT": "NO",
                "HARNESS_DEFECT": "YES",
                "ZERO_FAKE": "FAIL",
                "DETAIL": f"harness exception: {exc!r}",
                "TRACEBACK": traceback.format_exc()[-4000:],
                "checks": checks,
            }
        )
        write_result(result)
        return 1
    finally:
        stop_proc(proc, timeout=15.0)
        for p in fpm_procs:
            stop_proc(p, timeout=10.0)
        for px in proxies:
            try:
                px.stop()
            except Exception:
                pass
        for s in (
            fpm_sock,
            fpm_sock_a,
            fpm_sock_b,
            proxy_sock,
            proxy_sock_a,
            proxy_sock_b,
            ctrl,
        ):
            if s.exists():
                try:
                    s.unlink()
                except OSError:
                    pass


if __name__ == "__main__":
    sys.exit(main())
