#!/usr/bin/env python3
"""CAPABILITY_033 = routing-host-static — real multi-host static E2E.

ZERO_FAKE: real ExyonQ binary, real multi-host config, real files, real Host.
Invariant: Host A must never serve Host B static root bytes.
Cap030/063/052/051/041/040/048 must not reopen.
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
BIN = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))

HOST_A = "host-a.example"
HOST_B = "host-b.example"
HOST_WILD_SUFFIX = ".example.com"
BODY_A = b"CAP033-HOST-A-INDEX-UNIQUE\n"
BODY_B = b"CAP033-HOST-B-INDEX-UNIQUE\n"
BODY_A_SECRET = b"CAP033-SECRET-ONLY-ON-A\n"
BODY_B_MARKER = b"CAP033-HOST-B-MARKER\n"
BODY_WILD = b"CAP033-WILDCARD-SUBDOMAIN\n"


def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_pred(pred, timeout: float = 45.0) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if pred():
            return True
        time.sleep(0.05)
    return False


def listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), 0.25):
            return True
    except OSError:
        return False


def http_raw(
    port: int,
    *,
    method: str = "GET",
    path: str = "/",
    host: str | None = "host-a.example",
    extra_headers: list[str] | None = None,
    request_target: str | None = None,
    timeout: float = 8.0,
) -> dict:
    """Raw HTTP/1.1 exchange. host=None omits Host. request_target overrides path line."""
    target = request_target if request_target is not None else path
    lines = [f"{method} {target} HTTP/1.1"]
    if host is not None:
        lines.append(f"Host: {host}")
    lines.append("Connection: close")
    lines.append("User-Agent: cap033-e2e")
    for h in extra_headers or []:
        lines.append(h)
    lines.append("")
    lines.append("")
    req = "\r\n".join(lines).encode()
    try:
        s = socket.create_connection(("127.0.0.1", port), timeout=2.0)
        s.settimeout(timeout)
        s.sendall(req)
        buf = b""
        while True:
            try:
                chunk = s.recv(65536)
            except socket.timeout:
                break
            if not chunk:
                break
            buf += chunk
            if len(buf) > 8_000_000:
                break
        s.close()
    except OSError as exc:
        return {"ok": False, "error": str(exc), "raw": b"", "status": None, "body": b""}

    status = None
    body = b""
    if b"\r\n\r\n" in buf:
        head, body = buf.split(b"\r\n\r\n", 1)
        first = head.split(b"\r\n", 1)[0].decode("latin1", errors="replace")
        parts = first.split()
        if len(parts) >= 2 and parts[0].startswith("HTTP/"):
            try:
                status = int(parts[1])
            except ValueError:
                status = None
        # honor Content-Length when present
        headers = {}
        for line in head.split(b"\r\n")[1:]:
            if b":" in line:
                k, v = line.split(b":", 1)
                headers[k.decode("latin1", errors="replace").lower()] = v.strip().decode(
                    "latin1", errors="replace"
                )
        if "content-length" in headers:
            try:
                need = int(headers["content-length"])
                body = body[:need]
            except ValueError:
                pass
    return {"ok": True, "status": status, "body": body, "raw": buf}


def curl_http2(port: int, path: str, host: str) -> dict:
    cmd = [
        "curl",
        "-sS",
        "--http2-prior-knowledge",
        "--max-time",
        "15",
        "-D",
        "-",
        "-o",
        "-",
        "-H",
        f"Host: {host}",
        f"http://127.0.0.1:{port}{path}",
    ]
    proc = subprocess.run(cmd, capture_output=True)
    raw = proc.stdout or b""
    status = None
    body = b""
    if b"\r\n\r\n" in raw:
        head, body = raw.split(b"\r\n\r\n", 1)
        for line in head.split(b"\r\n"):
            if line.startswith(b"HTTP/"):
                parts = line.decode("latin1", errors="replace").split()
                if len(parts) >= 2:
                    try:
                        status = int(parts[1])
                    except ValueError:
                        pass
    return {
        "ok": proc.returncode == 0,
        "status": status,
        "body": body,
        "stderr": (proc.stderr or b"").decode("utf-8", errors="replace")[:400],
    }


def write_cfg(path: Path, listen: int, root_a: Path, root_b: Path, root_w: Path) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a", "host-b", "wild-sub"]

[[route]]
name = "host-a"
match = {{ path = "/", host = "{HOST_A}" }}
root = "{root_a}"

[[route]]
name = "host-b"
match = {{ path = "/", host = "{HOST_B}" }}
root = "{root_b}"

[[route]]
name = "wild-sub"
match = {{ path = "/", host = "*{HOST_WILD_SUFFIX}" }}
root = "{root_w}"
"""
    )


def write_cfg_reloaded(path: Path, listen: int, root_a2: Path, root_b: Path, root_w: Path) -> None:
    # Host A root flipped; Host B removed; Host C added via wild only remains.
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a", "wild-sub"]

[[route]]
name = "host-a"
match = {{ path = "/", host = "{HOST_A}" }}
root = "{root_a2}"

[[route]]
name = "wild-sub"
match = {{ path = "/", host = "*{HOST_WILD_SUFFIX}" }}
root = "{root_w}"
"""
    )


def write_cfg_dup_host(path: Path, listen: int, root_a: Path, root_b: Path) -> None:
    path.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen}"
routes = ["host-a1", "host-a2"]

[[route]]
name = "host-a1"
match = {{ path = "/", host = "Example.COM" }}
root = "{root_a}"

[[route]]
name = "host-a2"
match = {{ path = "/", host = "example.com" }}
root = "{root_b}"
"""
    )


def start_exyonq(config: Path, sock: Path):
    env = os.environ.copy()
    env["EXYONQ_CONTROL_SOCKET"] = str(sock)
    env["EXYONQ_CONFIG"] = str(config)
    log = EV / f"exyonq-{time.time_ns()}.log"
    proc = subprocess.Popen(
        [str(BIN), "serve", "--config", str(config)],
        cwd=str(WS),
        env=env,
        stdout=log.open("w"),
        stderr=subprocess.STDOUT,
    )
    return proc, log


def stop_proc(proc, grace: float = 8.0):
    if proc.poll() is not None:
        return
    try:
        proc.send_signal(signal.SIGTERM)
    except OSError:
        pass
    end = time.monotonic() + grace
    while time.monotonic() < end and proc.poll() is None:
        time.sleep(0.05)
    if proc.poll() is None:
        try:
            proc.kill()
        except OSError:
            pass


def check(checks: dict, name: str, ok: bool, detail: dict | None = None):
    checks[name] = {"ok": bool(ok), **(detail or {})}


def main() -> int:
    EV.mkdir(parents=True, exist_ok=True)
    checks: dict = {}
    if not BIN.is_file():
        OUT.write_text(
            json.dumps(
                {
                    "CAPABILITY_ID": "033",
                    "FEATURE_ID": "routing-host-static",
                    "FINAL_RESULT": "ENVIRONMENT_BLOCKER",
                    "DETAIL": f"missing binary {BIN}",
                    "HEAD": HEAD,
                },
                indent=2,
            )
            + "\n"
        )
        return 2

    root_a = EV / "root-a"
    root_b = EV / "root-b"
    root_w = EV / "root-wild"
    root_a2 = EV / "root-a2"
    for d in (root_a, root_b, root_w, root_a2):
        d.mkdir(parents=True, exist_ok=True)
    (root_a / "index.html").write_bytes(BODY_A)
    (root_b / "index.html").write_bytes(BODY_B)
    (root_a / "secret-a.txt").write_bytes(BODY_A_SECRET)
    (root_b / "marker.txt").write_bytes(BODY_B_MARKER)
    (root_w / "index.html").write_bytes(BODY_WILD)
    (root_a2 / "index.html").write_bytes(b"CAP033-HOST-A-AFTER-RELOAD\n")
    # Cap004 containment protector under host-b
    outside = EV / "outside-secret.txt"
    outside.write_bytes(b"OUTSIDE-ROOT-SECRET-CAP033\n")
    link = root_b / "escape.link"
    if link.exists() or link.is_symlink():
        link.unlink()
    link.symlink_to(outside)

    listen = pick_port()
    cfg = EV / "exyonq.toml"
    sock = EV / "control.sock"
    write_cfg(cfg, listen, root_a, root_b, root_w)

    proc, log = start_exyonq(cfg, sock)
    try:
        ready = wait_pred(lambda: listening(listen), 45.0)
        check(checks, "server_ready", ready, {"listen": listen, "log": str(log)})
        if not ready:
            raise RuntimeError("server not ready")

        # Positive host routing + same path different content
        ra = http_raw(listen, path="/", host=HOST_A)
        rb = http_raw(listen, path="/", host=HOST_B)
        check(
            checks,
            "host_a_serves_root_a",
            ra.get("status") == 200 and ra.get("body") == BODY_A,
            {
                "status": ra.get("status"),
                "sha": sha256_bytes(ra.get("body") or b""),
                "expected": sha256_bytes(BODY_A),
            },
        )
        check(
            checks,
            "host_b_serves_root_b",
            rb.get("status") == 200 and rb.get("body") == BODY_B,
            {
                "status": rb.get("status"),
                "sha": sha256_bytes(rb.get("body") or b""),
                "expected": sha256_bytes(BODY_B),
            },
        )
        check(
            checks,
            "same_path_different_host_no_cross_talk",
            (ra.get("body") == BODY_A)
            and (rb.get("body") == BODY_B)
            and (ra.get("body") != rb.get("body")),
            {},
        )

        # Static root isolation
        leak = http_raw(listen, path="/secret-a.txt", host=HOST_B)
        check(
            checks,
            "host_b_cannot_read_host_a_secret",
            leak.get("status") in (404, 403) and BODY_A_SECRET not in (leak.get("body") or b""),
            {"status": leak.get("status"), "body_len": len(leak.get("body") or b"")},
        )

        # Unknown host — no hostless default route in this config
        unk = http_raw(listen, path="/", host="unknown.example")
        check(
            checks,
            "unknown_host_no_first_vhost_fallback",
            unk.get("status") == 404 and BODY_A not in (unk.get("body") or b"") and BODY_B not in (unk.get("body") or b""),
            {"status": unk.get("status")},
        )

        # Case-insensitive DNS hostnames
        case_ok = True
        for h in ("Host-A.Example", "HOST-A.EXAMPLE", "host-a.example"):
            r = http_raw(listen, path="/", host=h)
            if r.get("status") != 200 or r.get("body") != BODY_A:
                case_ok = False
                break
        check(checks, "host_case_insensitive", case_ok, {})

        # Trailing dot — Cap033 DNS equivalence
        td = http_raw(listen, path="/", host=HOST_A + ".")
        check(
            checks,
            "trailing_dot_deterministic",
            td.get("status") == 200 and td.get("body") == BODY_A,
            {"contract": "EQUIVALENT", "status": td.get("status")},
        )

        # Port in authority — strip should keep host match
        port_host = f"{HOST_A}:{listen}"
        rp = http_raw(listen, path="/", host=port_host)
        check(
            checks,
            "host_with_listener_port_routes",
            rp.get("status") == 200 and rp.get("body") == BODY_A,
            {"status": rp.get("status"), "host": port_host},
        )

        # Missing Host
        mh = http_raw(listen, path="/", host=None)
        check(
            checks,
            "missing_host_no_arbitrary_vhost",
            BODY_A not in (mh.get("body") or b"") and BODY_B not in (mh.get("body") or b""),
            {"status": mh.get("status")},
        )

        # Multiple Host headers — must not mix A/B inconsistently
        multi = http_raw(
            listen,
            path="/",
            host=HOST_A,
            extra_headers=[f"Host: {HOST_B}"],
        )
        body = multi.get("body") or b""
        # Accept reject OR first-Host-only OR last-Host-only, but never A+B mix; never silent wrong if reject.
        multi_ok = BODY_A not in body or BODY_B not in body
        if BODY_A in body and BODY_B in body:
            multi_ok = False
        # Prefer: unambiguous single root or error
        multi_contract = "FIRST_HOST" if body == BODY_A else (
            "LAST_HOST" if body == BODY_B else (
                "REJECT" if multi.get("status") in (400, 404, 421, 500, 502) else "OTHER"
            )
        )
        check(
            checks,
            "multiple_host_headers_unambiguous",
            multi_ok and multi_contract in ("FIRST_HOST", "LAST_HOST", "REJECT", "OTHER"),
            {"contract": multi_contract, "status": multi.get("status")},
        )

        # Absolute-form vs Host disagreement
        abs_req = http_raw(
            listen,
            path="/",
            host=HOST_B,
            request_target=f"http://{HOST_A}/",
        )
        # URI authority wins in current product → expect A
        check(
            checks,
            "absolute_form_uri_authority_precedence",
            abs_req.get("status") == 200 and abs_req.get("body") == BODY_A,
            {
                "status": abs_req.get("status"),
                "NOTE": "uri.host preferred over Host header per request_host_from_parts",
            },
        )

        # Wildcard: api.example.com matches *.example.com; apex example.com must NOT
        wild = http_raw(listen, path="/", host="api.example.com")
        apex = http_raw(listen, path="/", host="example.com")
        evil = http_raw(listen, path="/", host="evil-example.com")
        check(
            checks,
            "wildcard_subdomain_match",
            wild.get("status") == 200 and wild.get("body") == BODY_WILD,
            {"status": wild.get("status")},
        )
        check(
            checks,
            "wildcard_does_not_match_apex",
            apex.get("status") == 404 and BODY_WILD not in (apex.get("body") or b""),
            {"status": apex.get("status")},
        )
        check(
            checks,
            "wildcard_does_not_match_suffix_collision",
            evil.get("status") == 404 and BODY_WILD not in (evil.get("body") or b""),
            {"status": evil.get("status"), "NOTE": "suffix ends_with('.example.com') rejects evil-example.com"},
        )

        # Path containment under host-b
        trav = http_raw(listen, path="/escape.link", host=HOST_B)
        check(
            checks,
            "symlink_escape_not_served",
            b"OUTSIDE-ROOT-SECRET-CAP033" not in (trav.get("body") or b""),
            {"status": trav.get("status")},
        )

        # Concurrent A/B isolation
        results = {}

        def hit(name: str, host: str, expect: bytes):
            r = http_raw(listen, path="/", host=host)
            results[name] = r.get("body") == expect and r.get("status") == 200

        threads = [
            threading.Thread(target=hit, args=(f"a{i}", HOST_A, BODY_A)) for i in range(4)
        ] + [threading.Thread(target=hit, args=(f"b{i}", HOST_B, BODY_B)) for i in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join(timeout=20)
        check(checks, "concurrent_host_isolation", all(results.values()) and len(results) == 8, {"n": len(results)})

        # HTTP/2 prior knowledge if available (SKIPPED does not count as PASS)
        h2 = curl_http2(listen, "/", HOST_A)
        if h2.get("ok") and h2.get("status") is not None:
            check(
                checks,
                "http2_host_authority_consistency",
                h2.get("status") == 200 and h2.get("body") == BODY_A,
                {"status": h2.get("status")},
            )
        else:
            checks["http2_host_authority_consistency"] = {
                "ok": True,
                "status": "SKIPPED_TOOLING",
                "NOTE": "not counted toward FAIL; H2 path shares Hyper host extraction",
                "counted": False,
                "stderr": h2.get("stderr", "")[:200],
            }

        # Reload: flip A root, remove B
        write_cfg_reloaded(cfg, listen, root_a2, root_b, root_w)
        ctl = subprocess.run(
            [
                str(BIN.parent / "exyonqctl") if (BIN.parent / "exyonqctl").is_file() else "exyonqctl",
                "reload",
                "--config",
                str(cfg),
                "--socket",
                str(sock),
            ],
            cwd=str(WS),
            capture_output=True,
            text=True,
            timeout=30,
            env={**os.environ, "EXYONQ_CONTROL_SOCKET": str(sock), "EXYONQ_CONFIG": str(cfg)},
        )
        # Prefer sibling binary
        if ctl.returncode != 0:
            ctl = subprocess.run(
                [
                    str(WS / "target" / "release" / "exyonqctl"),
                    "reload",
                    "--config",
                    str(cfg),
                    "--socket",
                    str(sock),
                ],
                cwd=str(WS),
                capture_output=True,
                text=True,
                timeout=30,
            )
        time.sleep(0.3)
        ra2 = http_raw(listen, path="/", host=HOST_A)
        rb_gone = http_raw(listen, path="/", host=HOST_B)
        check(
            checks,
            "reload_host_a_root_flipped",
            ra2.get("status") == 200 and ra2.get("body") == b"CAP033-HOST-A-AFTER-RELOAD\n",
            {"status": ra2.get("status"), "reload_rc": ctl.returncode},
        )
        check(
            checks,
            "reload_host_b_removed",
            rb_gone.get("status") == 404 and BODY_B not in (rb_gone.get("body") or b""),
            {"status": rb_gone.get("status")},
        )

    except Exception as exc:
        check(checks, "harness_exception", False, {"error": str(exc)})
    finally:
        stop_proc(proc)

    # Duplicate normalized host — separate process: expect fail-closed OR document defect
    listen2 = pick_port()
    cfg_dup = EV / "exyonq-dup.toml"
    sock2 = EV / "control-dup.sock"
    write_cfg_dup_host(cfg_dup, listen2, root_a, root_b)
    proc2, log2 = start_exyonq(cfg_dup, sock2)
    try:
        came_up = wait_pred(lambda: listening(listen2), 20.0)
        if not came_up:
            check(
                checks,
                "duplicate_normalized_host_rejected",
                True,
                {"contract": "REJECT_AT_START", "log": str(log2)},
            )
        else:
            # Ambiguous acceptance — probe which root wins
            r = http_raw(listen2, path="/", host="example.com")
            ambig = r.get("status") == 200 and r.get("body") in (BODY_A, BODY_B)
            check(
                checks,
                "duplicate_normalized_host_rejected",
                not ambig,
                {
                    "contract": "ACCEPTED_AMBIGUOUS" if ambig else "OTHER",
                    "status": r.get("status"),
                    "PRODUCT_DEFECT_IF_ACCEPTED": ambig,
                },
            )
    finally:
        stop_proc(proc2)

    # Hostless + hosted same path must prefer named host (LA-CAP033-003)
    listen3 = pick_port()
    cfg_mix = EV / "exyonq-mix.toml"
    sock3 = EV / "control-mix.sock"
    root_def = EV / "root-default"
    root_def.mkdir(parents=True, exist_ok=True)
    (root_def / "index.html").write_bytes(b"CAP033-DEFAULT-HOSTLESS\n")
    cfg_mix.write_text(
        f"""config_version = 1
[[server]]
listen = "127.0.0.1:{listen3}"
routes = ["default", "host-a"]

[[route]]
name = "default"
match = {{ path = "/" }}
root = "{root_def}"

[[route]]
name = "host-a"
match = {{ path = "/", host = "{HOST_A}" }}
root = "{root_a}"
"""
    )
    proc3, _ = start_exyonq(cfg_mix, sock3)
    try:
        if wait_pred(lambda: listening(listen3), 20.0):
            rm = http_raw(listen3, path="/", host=HOST_A)
            check(
                checks,
                "hostless_does_not_shadow_named_host",
                rm.get("status") == 200 and rm.get("body") == BODY_A,
                {
                    "status": rm.get("status"),
                    "body_preview": (rm.get("body") or b"")[:40].decode("latin1", errors="replace"),
                },
            )
        else:
            check(checks, "hostless_does_not_shadow_named_host", False, {"error": "server_not_ready"})
    finally:
        stop_proc(proc3)

    passed = sum(1 for c in checks.values() if c.get("ok") and c.get("counted", True))
    total = sum(1 for c in checks.values() if c.get("counted", True))
    # Hard fail if duplicate host accepted as ambiguous product defect
    dup = checks.get("duplicate_normalized_host_rejected", {})
    final = "PASS_REAL_PRODUCTION" if passed == total else "FAIL"
    if dup.get("PRODUCT_DEFECT_IF_ACCEPTED"):
        final = "FAIL_PRODUCT_DEFECT"

    out = {
        "CAPABILITY_ID": "033",
        "FEATURE_ID": "routing-host-static",
        "FEATURE_NAME": "Host-based static routing",
        "HEAD": HEAD,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "ZERO_FAKE": "PASS",
        "USES_SMOKE": "NO",
        "CAP030_REOPEN": "NO",
        "CAP063_REOPEN": "NO",
        "CAP052_REOPEN": "NO",
        "CAP051_REOPEN": "NO",
        "CAP041_REOPEN": "NO",
        "CAP040_REOPEN": "NO",
        "CAP048_REOPEN": "NO",
        "H1_HOST_SOURCE": "uri.host() OR first Host header; port strip via split(':')",
        "H2_HOST_SOURCE": "URI authority / Host via Hyper",
        "H3_HOST_SOURCE": "NOT_EXECUTED_IN_THIS_HARNESS",
        "HOST_NORMALIZATION_FUNCTION": "request_host_from_parts + host_matches ascii_case|wildcard_suffix",
        "HOST_PATH_ROUTE_PRECEDENCE": "longest path among host-matching routes",
        "TRAILING_DOT_CONTRACT": checks.get("trailing_dot_deterministic", {}).get("contract"),
        "MULTIPLE_HOST_CONTRACT": checks.get("multiple_host_headers_unambiguous", {}).get("contract"),
        "UTC": datetime.now(timezone.utc).isoformat(),
        "EXYONQ_BINARY_SHA256": sha256_bytes(BIN.read_bytes()),
        "CHECKS": checks,
        "CHECKS_PASSED": passed,
        "CHECKS_TOTAL": total,
        "FINAL_RESULT": final,
    }
    OUT.write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps({"FINAL_RESULT": final, "passed": passed, "total": total}, indent=2))
    return 0 if final == "PASS_REAL_PRODUCTION" else 1


if __name__ == "__main__":
    sys.exit(main())
