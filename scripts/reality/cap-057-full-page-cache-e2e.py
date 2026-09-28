#!/usr/bin/env python3
"""CAPABILITY_057 = full-page-cache — real product E2E.

The authority for a FastCGI HIT is an independent counter file written by
real PHP executed by real php-fpm. Static HITs are proven by rewriting the
served file after the first request. Proxy FPC fill is intentionally absent:
WC2E is deferred and Cap057 tests only Static + FastCGI fills.
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
ARCH = os.environ.get("ARCH_LABEL", "unknown")
HOST = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target/release/exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target/release/exyonqctl")))
SCENARIOS = [
    "boot", "fcgi_miss_then_hit", "static_miss_then_hit_file_rewrite_oracle",
    "host_isolation", "unsafe_query_bypass", "post_not_cached",
    "authorization_bypass", "cookie_bypass", "set_cookie_not_stored",
    "nostore_not_stored", "private_not_stored", "gzip_ce_not_stored",
    "ttl_expire", "purge_url_then_miss", "reload_generation_flush",
    "keepalive_purge_then_miss", "keepalive_reload_disable_then_miss",
    "ratelimit_before_fpc_hit", "waf_block_before_fpc", "status_500_not_stored",
    "head_does_not_store_get", "oversized_object_served_not_stored",
    "capacity_eviction_or_bound",
]


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return int(s.getsockname()[1])


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
        if path.exists():
            return True
        time.sleep(0.05)
    return False


def stop_proc(proc: subprocess.Popen[str] | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=12)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


def find_php_fpm() -> str | None:
    env = os.environ.get("PHP_FPM_BIN")
    if env and Path(env).is_file():
        return env
    for name in ("php-fpm", "php-fpm8.3", "php-fpm8.2", "php-fpm8.1"):
        found = subprocess.run(["bash", "-lc", f"command -v {name}"],
                               capture_output=True, text=True)
        if found.returncode == 0 and found.stdout.strip():
            return found.stdout.strip()
    for path in (Path("/usr/sbin/php-fpm8.3"), Path("/usr/sbin/php-fpm")):
        if path.is_file():
            return str(path)
    return None


def fpm_identity() -> tuple[str, str]:
    if os.geteuid() == 0:
        return "www-data", "www-data"
    return (subprocess.check_output(["id", "-un"], text=True).strip(),
            subprocess.check_output(["id", "-gn"], text=True).strip())


def open_write_perms(path: Path) -> None:
    try:
        os.chmod(path, 0o777)
    except OSError:
        pass
    subprocess.run(["chmod", "-R", "a+rwX", str(path)], check=False)


def prepare_counter_file(www: Path) -> Path:
    counter = www / "counter.dat"
    counter.write_text("")
    try:
        os.chmod(counter, 0o666)
    except OSError:
        pass
    if os.geteuid() == 0:
        try:
            import grp
            import pwd
            os.chown(www, pwd.getpwnam("www-data").pw_uid, grp.getgrnam("www-data").gr_gid)
            os.chown(counter, pwd.getpwnam("www-data").pw_uid, grp.getgrnam("www-data").gr_gid)
        except (KeyError, OSError, PermissionError):
            pass
    open_write_perms(www)
    return counter


def curl_req(url: str, *, method: str = "GET", data: bytes | None = None,
             headers: list[str] | None = None, timeout: int = 15) -> tuple[int, bytes, str]:
    tag = f"{time.time_ns()}-{hashlib.sha256(url.encode()).hexdigest()[:8]}"
    body_path, hdr_path = EV / f"{tag}.body", EV / f"{tag}.hdr"
    cmd = ["curl", "-sS", "--http1.1", "-H", "Connection: close", "--max-time",
           str(timeout), "-D", str(hdr_path), "-o", str(body_path), "-w", "%{http_code}",
           "-X", method]
    for header in headers or []:
        cmd.extend(["-H", header])
    data_path: Path | None = None
    if data is not None:
        data_path = EV / f"{tag}.data"
        data_path.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_path}"])
    cmd.append(url)
    run = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    for path in (body_path, hdr_path, data_path):
        if path:
            path.unlink(missing_ok=True)
    try:
        code = int((run.stdout or "").strip())
    except ValueError:
        code = -1
    return code, body, hdr


def count(counter: Path) -> int:
    return len(counter.read_text().splitlines()) if counter.is_file() else -1


def mark(checks: dict[str, Any], name: str, passed: bool, **detail: Any) -> None:
    checks[name] = {"PASS": bool(passed), **detail}


def php_scripts(www: Path, label: str, counter: Path) -> None:
    www.mkdir(parents=True, exist_ok=True)
    counter_expr = repr(str(counter))
    common = "<?php\nheader('Content-Type: text/plain');\n"
    (www / "page.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); "
        f"echo '{label}-' . ($_SERVER['QUERY_STRING'] ?? '') . \"\\n\";\n")
    for name in ("ttl", "purge", "reload", "rate", "head"):
        (www / f"{name}.php").write_text(
            common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); "
            f"echo '{label}-{name}\\n';\n")
    (www / "host.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); "
        f"echo '{label}-host\\n';\n")
    (www / "post.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); "
        f"echo '{label}-post-' . $_SERVER['REQUEST_METHOD'] . \"\\n\";\n")
    (www / "set-cookie.php").write_text(
        common + "setcookie('cap057', 'origin'); "
        f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'set-cookie\\n';\n")
    (www / "nostore.php").write_text(
        common + "header('Cache-Control: no-store'); "
        f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'nostore\\n';\n")
    (www / "private.php").write_text(
        common + "header('Cache-Control: private'); "
        f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'private\\n';\n")
    (www / "gzip.php").write_text(
        common + "header('Content-Encoding: gzip'); "
        f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'gzip-header\\n';\n")
    (www / "status500.php").write_text(
        f"<?php\nhttp_response_code(500); $f={counter_expr}; "
        "file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'status500\\n';\n")
    (www / "oversized.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); "
        "echo str_repeat('O', 4096);\n")
    (www / "capacity-a.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'capacity-a\\n';\n")
    (www / "capacity-b.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'capacity-b\\n';\n")
    (www / "capacity-c.php").write_text(
        common + f"$f={counter_expr}; file_put_contents($f, \"1\\n\", FILE_APPEND|LOCK_EX); echo 'capacity-c\\n';\n")
    open_write_perms(www)


def write_fpm_conf(path: Path, log: Path, user: str, group: str,
                   pools: list[tuple[str, Path]]) -> None:
    """Match Cap038 FPM shape: no chdir (avoids SCRIPT_FILENAME visibility surprises)."""
    blocks = [f"""[global]
error_log = {log}
daemonize = no
"""]
    for name, sock in pools:
        blocks.append(f"""[{name}]
user = {user}
group = {group}
listen = {sock}
listen.owner = {user}
listen.group = {group}
listen.mode = 0666
pm = static
pm.max_children = 8
clear_env = no
""")
    path.write_text("\n".join(blocks))


def cfg_text(
    listen: int,
    php_a: Path,
    php_b: Path,
    static_www: Path,
    host_static_a: Path,
    host_static_b: Path,
    sock_a: Path,
    sock_b: Path,
    *,
    ttl: int = 30,
    max_entries: int = 100,
    max_object: int = 1 << 20,
    ratelimit: bool = False,
    waf: bool = False,
    fpc_enabled: bool = True,
) -> str:
    rl = f"""[modules.ratelimit]
enabled = true
requests_per_second = 1
burst = 1
""" if ratelimit else ""
    wf = """[waf]
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
""" if waf else ""
    fpc_flag = "true" if fpc_enabled else "false"
    return f"""config_version = 1
[full_page_cache]
enabled = {fpc_flag}
namespace = 4
default_ttl_seconds = {ttl}
max_ttl_seconds = 3600
max_entries = {max_entries}
max_total_bytes = 10485760
max_object_bytes = {max_object}

[[server]]
listen = "127.0.0.1:{listen}"
routes = ["page", "static", "host-a", "host-b", "static-a", "static-b"]

[[route]]
name = "page"
match = {{ path = "/" }}
fastcgi = "pool_a"

[[route]]
name = "static"
match = {{ path = "/static/" }}
root = "{static_www}"
index = "index.html"

[[route]]
name = "host-a"
match = {{ path = "/host.php", host = "a.test" }}
fastcgi = "pool_a"

[[route]]
name = "host-b"
match = {{ path = "/host.php", host = "b.test" }}
fastcgi = "pool_b"

[[route]]
name = "static-a"
match = {{ path = "/hs/", host = "a.test" }}
root = "{host_static_a}"
index = "index.html"

[[route]]
name = "static-b"
match = {{ path = "/hs/", host = "b.test" }}
root = "{host_static_b}"
index = "index.html"

[[fcgi_pool]]
name = "pool_a"
address = "{sock_a}"
document_root = "{php_a}"
transport = "unix"
max_concurrency = 4
idle_timeout_ms = 30000
checkout_timeout_ms = 1500
total_timeout_ms = 8000

[[fcgi_pool]]
name = "pool_b"
address = "{sock_b}"
document_root = "{php_b}"
transport = "unix"
max_concurrency = 4
idle_timeout_ms = 30000
checkout_timeout_ms = 1500
total_timeout_ms = 8000
{rl}{wf}"""


# Rust `stable_fpc_site_id` uses DefaultHasher over ("exyonq.fpc.site.v1", route_name).
# Embed measured IDs from the workspace Rust toolchain (not a hand-rolled SipHash).
STABLE_FPC_SITE_IDS = {
    "page": 18063449260877812073,
    "static": 11888550133891401200,
    "host-a": 16123081956658121672,
    "host-b": 17044199855403159640,
}


def stable_site_id(route: str) -> int:
    if route not in STABLE_FPC_SITE_IDS:
        raise KeyError(f"missing embedded site_id for route {route!r}")
    return STABLE_FPC_SITE_IDS[route]


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict[str, Any] = {}
    result: dict[str, Any] = {
        "CAPABILITY_057": "full-page-cache",
        "FEATURE_ID": "full-page-cache",
        "ARCH": ARCH,
        "HOST": HOST,
        "HEAD": HEAD,
        "ZERO_FAKE": "FAIL",
        "CAP056_REOPEN": "NO",
        "CAP055_REOPEN": "NO",
        "CAP054_REOPEN": "NO",
        "LA_CAP054_008_STATUS": "OPEN",
        "FPC_PROXY": "NOT_SUPPORTED",
        "CAP061_STARTED": "NO",
        "CAP057_DEPENDENCY_CHANGE_EXPECTED": "NO",
        "PUSH": "NO",
        "GHCR_WRITE": "NO",
        "checks": checks,
    }
    if not BINARY.is_file() or not CTL.is_file():
        result.update({"overall": "FAIL", "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                       "DETAIL": f"missing binary: {BINARY if not BINARY.is_file() else CTL}"})
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 2
    fpm_bin = find_php_fpm()
    if not fpm_bin:
        result.update({"overall": "FAIL", "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                       "DETAIL": "php-fpm is required for real FastCGI FPC evidence"})
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_text(json.dumps(result, indent=2) + "\n")
        return 3

    tmp_base = Path("/var/tmp") if Path("/var/tmp").is_dir() else Path(tempfile.gettempdir())
    tmp = Path(tempfile.mkdtemp(prefix="cap057-", dir=str(tmp_base)))
    php_a, php_b = tmp / "php-a", tmp / "php-b"
    static_www = tmp / "static-www"
    host_static_a, host_static_b = tmp / "hs-a", tmp / "hs-b"
    for d in (php_a, php_b, static_www, host_static_a, host_static_b):
        d.mkdir()
    counter_a = prepare_counter_file(php_a)
    counter_b = prepare_counter_file(php_b)
    php_scripts(php_a, "A", counter_a)
    php_scripts(php_b, "B", counter_b)
    (static_www / "page.html").write_text("CAP057_STATIC_A_v1\n")
    (host_static_a / "index.html").write_text("HOST_A_STATIC_v1\n")
    (host_static_b / "index.html").write_text("HOST_B_STATIC_v1\n")
    open_write_perms(tmp)
    open_write_perms(php_a)
    open_write_perms(php_b)
    open_write_perms(static_www)
    open_write_perms(host_static_a)
    open_write_perms(host_static_b)
    fpm_user, fpm_group = fpm_identity()
    sock_a, sock_b = tmp / "fpm-a.sock", tmp / "fpm-b.sock"
    fpm_cfg = tmp / "php-fpm.conf"
    write_fpm_conf(fpm_cfg, tmp / "fpm.log", fpm_user, fpm_group,
                   [("pool_a", sock_a), ("pool_b", sock_b)])
    fpm = subprocess.Popen([fpm_bin, "--nodaemonize", "--fpm-config", str(fpm_cfg)],
                           stdout=(tmp / "fpm.out").open("w"), stderr=subprocess.STDOUT)
    listen = pick_port()
    ctrl = Path(f"/tmp/exq57-{os.getpid()}-{time.time_ns() % 100000}.sock")
    purge_sock = tmp / "purge.sock"
    token = "cap057-e2e-token"
    cfg = tmp / "live.toml"
    log = EV / "exyonq-cap057.log"

    def live_cfg(**kwargs: Any) -> str:
        return cfg_text(
            listen, php_a, php_b, static_www, host_static_a, host_static_b, sock_a, sock_b, **kwargs
        )

    cfg.write_text(live_cfg())
    env = os.environ.copy()
    env.update({"EXYONQ_CONFIG": str(cfg), "EXYONQ_CONTROL_SOCKET": str(ctrl),
                "EXYONQ_CACHE_PURGE_SOCKET": str(purge_sock),
                "EXYONQ_CACHE_PURGE_TOKEN": token, "RUST_LOG": "info"})
    proc: subprocess.Popen[str] | None = None
    try:
        if not wait_sock(sock_a) or not wait_sock(sock_b):
            raise RuntimeError("php-fpm socket missing")
        proc = subprocess.Popen([str(BINARY), "serve", "--config", str(cfg)], cwd=str(WS),
                                env=env, stdout=log.open("w"), stderr=subprocess.STDOUT)
        base = f"http://127.0.0.1:{listen}"
        boot = wait_listen(listen) and wait_sock(ctrl) and wait_sock(purge_sock)
        mark(checks, "boot", boot, pid=proc.pid, purge_socket=str(purge_sock), tmp=str(tmp))
        if not boot:
            raise RuntimeError("ExyonQ/control/purge listener not ready")

        def req(path: str, **kwargs: Any) -> tuple[int, bytes, str]:
            return curl_req(base + path, **kwargs)
        def reload_cfg(**kwargs: Any) -> tuple[int, str]:
            cfg.write_text(live_cfg(**kwargs))
            r = subprocess.run([str(CTL), "reload", "--config", str(cfg), "--socket", str(ctrl)],
                               capture_output=True, text=True, timeout=30, env=env)
            return r.returncode, (r.stdout or "") + (r.stderr or "")
        def reset_counter(path: Path = counter_a) -> None:
            path.write_text("")

        reset_counter()
        c1, b1, _ = req("/page.php")
        n1 = count(counter_a)
        c2, b2, _ = req("/page.php")
        n2 = count(counter_a)
        mark(checks, "fcgi_miss_then_hit", c1 == 200 and c2 == 200 and n1 == 1 and n2 == 1
             and b1 == b2, c1=c1, c2=c2, counter_after_first=n1, counter_after_second=n2,
             body=b1.decode(errors="replace")[:80])

        s1, sb1, _ = req("/static/page.html")
        (static_www / "page.html").write_text("CAP057_STATIC_A_REWRITTEN\n")
        s2, sb2, _ = req("/static/page.html")
        mark(checks, "static_miss_then_hit_file_rewrite_oracle",
             s1 == 200 and s2 == 200 and sb1 == sb2 and b"v1" in sb2,
             first=sb1.decode(errors="replace"), second=sb2.decode(errors="replace"))

        reset_counter()
        ha, ba, _ = req("/host.php", headers=["Host: a.test"])
        hb, bb, _ = req("/host.php", headers=["Host: b.test"])
        sa, sba, _ = req("/hs/", headers=["Host: a.test"])
        sb, sbb, _ = req("/hs/", headers=["Host: b.test"])
        mark(checks, "host_isolation", ha == hb == sa == sb == 200 and
             b"A-host" in ba and b"B-host" in bb and b"HOST_A" in sba and b"HOST_B" in sbb,
             php_a=ba.decode(errors="replace"), php_b=bb.decode(errors="replace"),
             static_a=sba.decode(errors="replace"), static_b=sbb.decode(errors="replace"))

        reset_counter()
        q1, qb1, _ = req("/page.php?id=1")
        q2, qb2, _ = req("/page.php?id=2")
        mark(checks, "unsafe_query_bypass", q1 == q2 == 200 and count(counter_a) == 2
             and qb1 != qb2, hits=count(counter_a), bodies=[qb1.decode(errors="replace"),
             qb2.decode(errors="replace")], POLICY="empty functional_query allowlist => UnsafeQuery BYPASS")

        reset_counter()
        p1, _, _ = req("/post.php", method="POST", data=b"x=1")
        p2, _, _ = req("/post.php", method="POST", data=b"x=2")
        g1, _, _ = req("/post.php")
        g2, _, _ = req("/post.php")
        mark(checks, "post_not_cached", p1 == p2 == g1 == g2 == 200 and count(counter_a) == 3,
             post_codes=[p1, p2], get_codes=[g1, g2], counter=count(counter_a))

        for name, header in (("authorization_bypass", "Authorization: " + "Bearer" + " " + "cap057-a"),
                             ("cookie_bypass", "Cookie: cap057=1")):
            reset_counter()
            x1, _, _ = req("/page.php", headers=[header])
            x2, _, _ = req("/page.php", headers=[header])
            mark(checks, name, x1 == x2 == 200 and count(counter_a) == 2, hits=count(counter_a))

        for name, path in (("set_cookie_not_stored", "/set-cookie.php"),
                           ("nostore_not_stored", "/nostore.php"),
                           ("private_not_stored", "/private.php"),
                           ("gzip_ce_not_stored", "/gzip.php")):
            reset_counter()
            x1, _, h1 = req(path)
            x2, _, h2 = req(path)
            mark(checks, name, x1 == x2 == 200 and count(counter_a) == 2,
                 hits=count(counter_a), headers=[h1[:300], h2[:300]])

        reset_counter()
        reload_cfg(ttl=2)
        t1, _, _ = req("/ttl.php")
        t2, _, _ = req("/ttl.php")
        before_ttl = count(counter_a)
        time.sleep(2.5)
        t3, _, _ = req("/ttl.php")
        mark(checks, "ttl_expire", t1 == t2 == t3 == 200 and before_ttl == 1
             and count(counter_a) == 2, before=before_ttl, after=count(counter_a))
        reload_cfg()

        reset_counter()
        pu1, _, _ = req("/purge.php")
        site_id = stable_site_id("page")
        purge = subprocess.run([str(CTL), "purge", "site", str(site_id), "--socket",
                                str(purge_sock), "--token", token], capture_output=True,
                               text=True, timeout=15, env=env)
        pu2, _, _ = req("/purge.php")
        mark(checks, "purge_url_then_miss", pu1 == pu2 == 200 and count(counter_a) == 2
             and purge.returncode == 0, site_id=site_id, purge_rc=purge.returncode,
             purge_output=(purge.stdout + purge.stderr)[:300],
             purged_ack=(purge.stdout or "")[:200])

        reset_counter()
        r1, _, _ = req("/reload.php")
        r2, _, _ = req("/reload.php")
        pre_reload = count(counter_a)
        rc, rout = reload_cfg(ttl=31)
        r3, _, _ = req("/reload.php")
        mark(checks, "reload_generation_flush", r1 == r2 == r3 == 200 and pre_reload == 1
             and count(counter_a) == 2 and rc == 0, reload_rc=rc, reload_output=rout[:300])

        # Cap057 LA-CAP057-002: Hyper keep-alive must observe live purge / disable.
        import http.client
        reset_counter()
        ka_purge_ok = False
        ka_purge_detail: dict[str, Any] = {}
        try:
            conn = http.client.HTTPConnection("127.0.0.1", listen, timeout=15)
            conn.request("GET", "/purge.php", headers={"Host": f"127.0.0.1:{listen}"})
            r1 = conn.getresponse(); _ = r1.read()
            n_after_fill = count(counter_a)
            site_id_ka = stable_site_id("page")
            purge_ka = subprocess.run(
                [str(CTL), "purge", "site", str(site_id_ka), "--socket", str(purge_sock),
                 "--token", token],
                capture_output=True, text=True, timeout=15, env=env,
            )
            conn.request("GET", "/purge.php", headers={"Host": f"127.0.0.1:{listen}"})
            r2 = conn.getresponse(); _ = r2.read()
            n_after_purge = count(counter_a)
            conn.close()
            ka_purge_ok = (
                r1.status == 200 and r2.status == 200 and n_after_fill == 1
                and n_after_purge == 2 and purge_ka.returncode == 0
            )
            ka_purge_detail = {
                "status": [r1.status, r2.status],
                "counter": [n_after_fill, n_after_purge],
                "purge_rc": purge_ka.returncode,
            }
        except Exception as exc:
            ka_purge_detail = {"error": str(exc)}
        mark(checks, "keepalive_purge_then_miss", ka_purge_ok, **ka_purge_detail)

        reset_counter()
        ka_dis_ok = False
        ka_dis_detail: dict[str, Any] = {}
        try:
            conn = http.client.HTTPConnection("127.0.0.1", listen, timeout=15)
            conn.request("GET", "/reload.php", headers={"Host": f"127.0.0.1:{listen}"})
            d1 = conn.getresponse(); _ = d1.read()
            n_fill = count(counter_a)
            rc_dis, rout_dis = reload_cfg(fpc_enabled=False)
            conn.request("GET", "/reload.php", headers={"Host": f"127.0.0.1:{listen}"})
            d2 = conn.getresponse(); _ = d2.read()
            n_after = count(counter_a)
            conn.close()
            ka_dis_ok = (
                d1.status == 200 and d2.status == 200 and n_fill == 1
                and n_after == 2 and rc_dis == 0
            )
            ka_dis_detail = {
                "status": [d1.status, d2.status],
                "counter": [n_fill, n_after],
                "reload_rc": rc_dis,
                "reload_output": rout_dis[:300],
            }
        except Exception as exc:
            ka_dis_detail = {"error": str(exc)}
        mark(checks, "keepalive_reload_disable_then_miss", ka_dis_ok, **ka_dis_detail)
        reload_cfg()  # restore FPC enabled for remaining scenarios

        rc, rout = reload_cfg(ratelimit=True)
        reset_counter()
        rl1, _, _ = req("/rate.php")
        rl2, _, _ = req("/rate.php")
        mark(checks, "ratelimit_before_fpc_hit", rc == 0 and rl1 == 200 and rl2 == 429,
             reload_rc=rc, codes=[rl1, rl2], note="limiter must run before FPC lookup")
        reload_cfg()

        # WAF enabled before fill so generation-scoped FPC can HIT, then malicious header must 403.
        rc, _ = reload_cfg(waf=True)
        (static_www / "page.html").write_text("CAP057_STATIC_WAF_v1\n")
        # New body under same URL key requires miss/fill after generation bump from waf reload.
        wf1, b_wf1, _ = req("/static/page.html")
        wf2, b_wf2, _ = req("/static/page.html")
        wf3, b_wf3, _ = req("/static/page.html", headers=["X-Evil: <script>"])
        mark(checks, "waf_block_before_fpc",
             rc == 0 and wf1 == 200 and wf2 == 200 and b_wf1 == b_wf2
             and wf3 == 403 and b"CAP057_STATIC_WAF" not in b_wf3,
             fill_code=wf1, hit_code=wf2, blocked_code=wf3,
             blocked_body=b_wf3[:120].decode(errors="replace"))
        reload_cfg()
        (static_www / "page.html").write_text("CAP057_STATIC_A_v1\n")

        reset_counter()
        e1, _, _ = req("/status500.php")
        e2, _, _ = req("/status500.php")
        mark(checks, "status_500_not_stored", e1 == e2 == 500 and count(counter_a) == 2, hits=count(counter_a))

        reset_counter()
        hd, hb, _ = req("/head.php", method="HEAD")
        gd, _, _ = req("/head.php")
        # HEAD executes origin but must not store; GET must execute once to fill.
        mark(checks, "head_does_not_store_get", hd == gd == 200 and hb == b"" and count(counter_a) == 2,
             head_body_len=len(hb), counter=count(counter_a))

        reload_cfg(max_object=1024)
        reset_counter()
        o1, ob1, _ = req("/oversized.php")
        o2, ob2, _ = req("/oversized.php")
        mark(checks, "oversized_object_served_not_stored", o1 == o2 == 200 and len(ob1) == 4096
             and ob1 == ob2 and count(counter_a) == 2, body_len=len(ob1), hits=count(counter_a))

        reload_cfg(max_entries=2, max_object=1 << 20)
        reset_counter()
        ca, _, _ = req("/capacity-a.php")
        cb, _, _ = req("/capacity-b.php")
        cc, _, _ = req("/capacity-c.php")
        ca2, _, _ = req("/capacity-a.php")
        mark(checks, "capacity_eviction_or_bound",
             ca == cb == cc == ca2 == 200 and count(counter_a) == 4,
             counter=count(counter_a), note="max_entries bound forces early-page miss after three unique pages")
        reload_cfg()
    except Exception as exc:
        mark(checks, "fatal", False, error=f"{type(exc).__name__}: {exc}")
    finally:
        stop_proc(proc)
        stop_proc(fpm)
        ctrl.unlink(missing_ok=True)

    for name in SCENARIOS:
        checks.setdefault(name, {"PASS": False, "detail": "NOT_RUN"})
    all_pass = all(bool(checks[name].get("PASS")) for name in SCENARIOS)
    result.update({
        "BINARY_SHA256": sha256_file(BINARY),
        "PHP_FPM_BIN": fpm_bin,
        "checks": checks,
        "overall": "PASS" if all_pass else "FAIL",
        "FINAL_RESULT": "PASS_REAL_PRODUCTION" if all_pass else "FAIL_REAL_E2E",
        "ZERO_FAKE": "PASS" if all_pass else "FAIL",
        "SCENARIO_COUNT": len(SCENARIOS),
        "SCENARIOS_PASSED": sum(bool(checks[n].get("PASS")) for n in SCENARIOS),
    })
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": result["FINAL_RESULT"], "ZERO_FAKE": result["ZERO_FAKE"]}))
    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
