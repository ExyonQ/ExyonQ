#!/usr/bin/env python3
"""CAPABILITY_054 = metrics-prometheus — real product E2E.

ZERO_FAKE chain:
  REAL PRODUCT EVENT → REAL METRIC UPDATE → REAL HTTP SCRAPE → REAL EXPOSED VALUE

Proves Cap054 (does not reopen Cap038/032/031/048/…):
  - metrics disabled → no OpenMetrics on /metrics
  - metrics enabled → scrapeable OpenMetrics (+ EOF)
  - exact request-counter delta (scrape self-accounting documented)
  - status class 2xx/4xx
  - method labels GET/POST
  - response-head duration histogram movement
  - content-type + cache-control + POST→405
  - concurrent scrapes remain valid
  - reload does not reset process-lifetime counters
  - health JSON version reflects package 0.4.4
  - proxy 502 path increments 5xx when upstream down

Cap056/057 cache metrics: NOT closed here.
"""
from __future__ import annotations

import concurrent.futures
import hashlib
import json
import os
import re
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

WS = Path(os.environ.get("WS", ".")).resolve()
OUT = Path(os.environ["OUT_JSON"])
EV = Path(os.environ.get("EV_DIR", str(OUT.parent))).resolve()
ARCH_LABEL = os.environ.get("ARCH_LABEL", "unknown")
HOST_LABEL = os.environ.get("HOST_LABEL", socket.gethostname())
HEAD = os.environ.get("HEAD", "UNKNOWN")
BINARY = Path(os.environ.get("EXYONQ_BIN", str(WS / "target" / "release" / "exyonq")))
CTL = Path(os.environ.get("EXYONQCTL_BIN", str(WS / "target" / "release" / "exyonqctl")))

STATIC_BODY = b"cap054-static-ok\n"
SLOW_BODY = b"cap054-slow-ok\n"


def sha256_file(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def pick_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_listen(port: int, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.05)
    return False


def wait_sock(path: Path, timeout: float = 45.0) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.is_socket():
            return True
        time.sleep(0.05)
    return False


def stop_proc(p):
    if p is None or p.poll() is not None:
        return
    p.send_signal(signal.SIGTERM)
    try:
        p.wait(timeout=10)
    except Exception:
        p.kill()


def curl_req(
    url: str,
    *,
    method: str = "GET",
    data: bytes | None = None,
    timeout: int = 20,
    headers_out: Path | None = None,
    extra_headers: list[str] | None = None,
) -> tuple[int, bytes, str]:
    tag = f"{time.time_ns()}-{threading.get_ident()}"
    body_path = EV / f"curl-{tag}.body"
    hdr_path = headers_out or (EV / f"curl-{tag}.hdr")
    cmd = [
        "curl",
        "-sS",
        "--max-time",
        str(timeout),
        "-D",
        str(hdr_path),
        "-o",
        str(body_path),
        "-w",
        "%{http_code}",
        "-X",
        method,
    ]
    if extra_headers:
        for h in extra_headers:
            cmd.extend(["-H", h])
    data_file = None
    if data is not None:
        data_file = EV / f"curl-{tag}.data"
        data_file.write_bytes(data)
        cmd.extend(["--data-binary", f"@{data_file}"])
    cmd.append(url)
    proc = subprocess.run(cmd, capture_output=True, text=True)
    body = body_path.read_bytes() if body_path.is_file() else b""
    hdr = hdr_path.read_text(errors="replace") if hdr_path.is_file() else ""
    try:
        body_path.unlink(missing_ok=True)
        if data_file is not None:
            data_file.unlink(missing_ok=True)
        if headers_out is None:
            hdr_path.unlink(missing_ok=True)
    except OSError:
        pass
    try:
        code = int((proc.stdout or "").strip() or "0")
    except ValueError:
        code = 0
    return code, body, hdr


def parse_counter(text: str, name: str) -> float | None:
    m = re.search(rf"(?m)^{re.escape(name)} (\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\s*$", text)
    if not m:
        return None
    return float(m.group(1))


def parse_counter_int(text: str, name: str) -> int | None:
    v = parse_counter(text, name)
    return None if v is None else int(v)


def parse_labeled(text: str, name: str, labels: dict[str, str]) -> int | None:
    prefix = f"{name}{{"
    for line in text.splitlines():
        if not line.startswith(prefix):
            continue
        try:
            brace = line.index("{")
            end = line.rindex("}")
            inner = line[brace + 1 : end]
            val = float(line[end + 1 :].strip())
        except ValueError:
            continue
        parsed = {
            k: v.replace(r"\\", "\\").replace(r"\"", '"').replace(r"\n", "\n")
            for k, v in re.findall(r'(\w+)="((?:\\.|[^"\\])*)"', inner)
        }
        if all(parsed.get(k) == v for k, v in labels.items()):
            return int(val)
    return None


def validate_openmetrics(text: str) -> list[str]:
    errs: list[str] = []
    if "# EOF" not in text and not text.rstrip().endswith("# EOF"):
        # OpenMetrics requires EOF marker; our encoder emits "# EOF\n"
        if "# EOF\n" not in text and not text.endswith("# EOF"):
            errs.append("missing_EOF")
    if "exyonq_http_requests_total" not in text:
        errs.append("missing_requests_total")
    # No NaN/Inf samples on numeric lines
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if " NaN" in line or " +Inf" in line or " -Inf" in line:
            # histogram le="+Inf" is a label, not a sample value
            if re.search(r"\} NaN\b|\} \+Inf\b|\} -Inf\b|^\S+ NaN\b|^\S+ \+Inf\b", line):
                errs.append(f"non_finite_sample:{line[:80]}")
    # TYPE before samples for main family
    if "# TYPE exyonq_http_requests_total counter" not in text:
        errs.append("missing_TYPE_requests_total")
    return errs


def make_peer(slow_ms: int = 0):
    class H(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_a):
            pass

        def do_GET(self):
            if self.path.startswith("/slow"):
                if slow_ms:
                    time.sleep(slow_ms / 1000.0)
                body = SLOW_BODY
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif self.path.startswith("/api"):
                body = b"cap054-api-ok"
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            elif self.path.startswith("/err500"):
                body = b"upstream-500"
                self.send_response(500)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            else:
                self.send_response(404)
                self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("Content-Length") or "0")
            _ = self.rfile.read(n) if n else b""
            body = b"cap054-post-ok"
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

    return H


def start_peer(port: int, slow_ms: int = 80):
    httpd = ThreadingHTTPServer(("127.0.0.1", port), make_peer(slow_ms))
    httpd.allow_reuse_address = True
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def cfg_text(
    listen: int,
    root: Path,
    peer: int | None,
    *,
    metrics_enabled: bool,
    metrics_path: str = "/metrics",
    include_proxy: bool = True,
    listen_host: str = "127.0.0.1",
    scrape_bearer_token: str | None = None,
    ratelimit_enabled: bool = False,
    ratelimit_rps: int = 100,
    ratelimit_burst: int = 200,
) -> str:
    routes = '["site"]'
    extra_routes = ""
    upstream = ""
    if include_proxy and peer is not None:
        routes = '["site", "api", "slow"]'
        extra_routes = f"""
[[route]]
name = "api"
match = {{ path = "/api/" }}
upstream = "backend"
[[route]]
name = "slow"
match = {{ path = "/slow/" }}
upstream = "backend"
"""
        upstream = f"""
[[upstream]]
name = "backend"
timeout_ms = 5000
[[upstream.endpoints]]
address = "127.0.0.1"
port = {peer}
weight = 1
priority = 0
admin_state = "enabled"
"""
    bearer = ""
    if scrape_bearer_token is not None:
        bearer = f'\nscrape_bearer_token = "{scrape_bearer_token}"'
    metrics = f"""
[modules.metrics]
enabled = {"true" if metrics_enabled else "false"}
path = "{metrics_path}"
health_path = "/exyonq-metrics-health"{bearer}
"""
    ratelimit = ""
    if ratelimit_enabled:
        ratelimit = f"""
[modules.ratelimit]
enabled = true
requests_per_second = {ratelimit_rps}
burst = {ratelimit_burst}
"""
    return f"""config_version = 1
[[server]]
listen = "{listen_host}:{listen}"
routes = {routes}
[[route]]
name = "site"
match = {{ path = "/site/" }}
root = "{root}"
index = "index.html"
{extra_routes}{upstream}{metrics}{ratelimit}
"""


def main() -> int:
    checks: dict = {}
    ok = True
    tmp = Path(tempfile.mkdtemp(prefix="cap054-", dir=str(EV)))
    www = tmp / "www"
    www.mkdir()
    (www / "index.html").write_bytes(STATIC_BODY)
    (www / "ok.txt").write_bytes(STATIC_BODY)

    peer_port = pick_port()
    httpd = start_peer(peer_port, slow_ms=80)
    listen = pick_port()
    cfg_path = tmp / "live.toml"
    ctrl = Path(f"/tmp/exq54-{os.getpid()}-{time.time_ns() % 100000}.sock")
    if ctrl.exists():
        ctrl.unlink()
    proc = None
    scenarios_pass = 0
    scenarios_total = 0

    def mark(name: str, passed: bool, detail: dict | None = None):
        nonlocal ok, scenarios_pass, scenarios_total
        scenarios_total += 1
        if passed:
            scenarios_pass += 1
        else:
            ok = False
        checks[name] = {"ok": passed, **(detail or {})}

    try:
        if not BINARY.is_file() or not CTL.is_file():
            mark("binaries", False, {"bin": str(BINARY), "ctl": str(CTL)})
            raise RuntimeError("missing binaries")
        mark("binaries", True)

        # --- disabled metrics ---
        cfg_path.write_text(
            cfg_text(listen, www, peer_port, metrics_enabled=False, include_proxy=True)
        )
        env = os.environ.copy()
        env["EXYONQ_CONFIG"] = str(cfg_path)
        env["EXYONQ_CONTROL_SOCKET"] = str(ctrl)
        log = tmp / "serve.log"
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(listen):
            mark("startup_disabled", False, {"log_tail": log.read_text()[-800:]})
            raise RuntimeError("startup failed (disabled)")
        mark("startup_disabled", True, {"pid": proc.pid})

        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        text = body.decode("utf-8", errors="replace")
        # Cap054 + LA-CAP054-008: disabled metrics must not expose any OpenMetrics
        # (not merely Cap054 series names — Linux wire historically had a second authority).
        lower = text.lower()
        hdr_lower = ""
        disabled_ok = (
            "exyonq_http_requests_total" not in text
            and "# TYPE" not in text
            and "# EOF" not in text
            and "openmetrics" not in lower
            and "application/openmetrics-text" not in lower
        )
        mark(
            "metrics_disabled_no_exposition",
            disabled_ok,
            {"http_code": code, "body_prefix": text[:120]},
        )

        # Enable metrics via reload (same listen/root)
        cfg_path.write_text(
            cfg_text(listen, www, peer_port, metrics_enabled=True, include_proxy=True)
        )
        r = subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
        )
        mark(
            "reload_enable_metrics",
            r.returncode == 0,
            {"rc": r.returncode, "out": ((r.stdout or "") + (r.stderr or ""))[:300]},
        )
        time.sleep(0.2)

        # Warm scrape (self-accounting)
        code, body, hdr = curl_req(f"http://127.0.0.1:{listen}/metrics")
        text = body.decode("utf-8", errors="replace")
        om_errs = validate_openmetrics(text)
        ctype_ok = "application/openmetrics-text" in hdr.lower()
        cache_ok = "no-store" in hdr.lower()
        mark(
            "scrape_openmetrics_valid",
            code == 200 and not om_errs and ctype_ok and cache_ok,
            {
                "http_code": code,
                "om_errs": om_errs,
                "content_type_ok": ctype_ok,
                "cache_control_ok": cache_ok,
                "hdr_sample": hdr[:400],
            },
        )

        # Core liveness /health remains WS5 body "ok" (not metrics JSON).
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/health")
        mark(
            "core_liveness_health_ok",
            code == 200 and body == b"ok",
            {"http_code": code, "body": body.decode("utf-8", errors="replace")[:80]},
        )

        # Metrics JSON health on non-colliding default path
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/exyonq-metrics-health")
        health = body.decode("utf-8", errors="replace")
        mark(
            "health_version_0_4_3",
            code == 200 and '"version":"0.4.4"' in health and '"status":"ok"' in health,
            {"http_code": code, "body": health[:200]},
        )

        # POST /metrics → 405
        code, _, hdr = curl_req(f"http://127.0.0.1:{listen}/metrics", method="POST", data=b"x")
        mark(
            "metrics_post_method_not_allowed",
            code == 405,
            {"http_code": code, "allow": "Allow" in hdr or "allow" in hdr.lower()},
        )

        # Exact request counter delta
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        b_text = body.decode("utf-8", errors="replace")
        b_total = parse_counter_int(b_text, "exyonq_http_requests_total")
        b_dur = parse_counter_int(b_text, "exyonq_http_request_duration_milliseconds_count")
        b_sum = parse_counter(b_text, "exyonq_http_request_duration_milliseconds_sum")
        mark("baseline_parse", b_total is not None and b_dur is not None, {"b_total": b_total, "b_dur": b_dur, "b_sum": b_sum})

        N = 7
        for _ in range(N):
            c, body, _ = curl_req(f"http://127.0.0.1:{listen}/site/ok.txt")
            if c != 200 or body != STATIC_BODY:
                mark("dataplane_static", False, {"code": c})
                break
        else:
            mark("dataplane_static", True, {"N": N})

        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        a_text = body.decode("utf-8", errors="replace")
        a_total = parse_counter_int(a_text, "exyonq_http_requests_total")
        a_dur = parse_counter_int(a_text, "exyonq_http_request_duration_milliseconds_count")
        a_sum = parse_counter(a_text, "exyonq_http_request_duration_milliseconds_sum")
        # Baseline scrape completion increments +1 before dataplane; see HELP.
        # delta(body_a - body_b) = N + 1 (baseline scrape's on_response)
        SCRAPE_SELF = 1
        expected = N + SCRAPE_SELF
        delta = None if a_total is None or b_total is None else a_total - b_total
        mark(
            "request_counter_exact_delta",
            delta == expected,
            {
                "b": b_total,
                "a": a_total,
                "delta": delta,
                "expected": expected,
                "N": N,
                "SCRAPE_INCREMENTS_REQUEST_METRICS": True,
                "SCRAPE_SELF_ACCOUNTING": SCRAPE_SELF,
            },
        )
        dur_delta = None if a_dur is None or b_dur is None else a_dur - b_dur
        mark(
            "duration_histogram_movement",
            dur_delta is not None and dur_delta >= expected,
            {"b_dur": b_dur, "a_dur": a_dur, "dur_delta": dur_delta, "expected_min": expected},
        )
        # LA-CAP054-001: static hot path must advance fractional _sum (not round to 0).
        mark(
            "duration_sum_advances_on_static",
            b_sum is not None and a_sum is not None and a_sum > b_sum,
            {"b_sum": b_sum, "a_sum": a_sum},
        )

        # Status: 404
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        before_4xx = parse_counter_int(
            body.decode("utf-8", errors="replace"), "exyonq_http_responses_4xx_total"
        )
        curl_req(f"http://127.0.0.1:{listen}/site/no-such-file-cap054.txt")
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        after_4xx = parse_counter_int(
            body.decode("utf-8", errors="replace"), "exyonq_http_responses_4xx_total"
        )
        # +1 for 404, +1 for baseline scrape in this block? baseline scrape may be 2xx
        # before scrape is 2xx → before_4xx unchanged by scrape; 404 +1; after scrape is 2xx
        mark(
            "status_4xx_delta",
            before_4xx is not None
            and after_4xx is not None
            and after_4xx - before_4xx == 1,
            {"before": before_4xx, "after": after_4xx},
        )

        # Method labels: POST to /api/
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        t0 = body.decode("utf-8", errors="replace")
        post_before = parse_labeled(
            t0,
            "exyonq_http_requests_labeled_total",
            {"method": "POST", "status": "2xx", "route": "_api_"},
        ) or 0
        get_before = parse_labeled(
            t0,
            "exyonq_http_requests_labeled_total",
            {"method": "GET", "status": "2xx", "route": "_api_"},
        ) or 0
        curl_req(f"http://127.0.0.1:{listen}/api/", method="POST", data=b"hello-cap054")
        curl_req(f"http://127.0.0.1:{listen}/api/")
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        t1 = body.decode("utf-8", errors="replace")
        post_after = parse_labeled(
            t1,
            "exyonq_http_requests_labeled_total",
            {"method": "POST", "status": "2xx", "route": "_api_"},
        ) or 0
        get_after = parse_labeled(
            t1,
            "exyonq_http_requests_labeled_total",
            {"method": "GET", "status": "2xx", "route": "_api_"},
        ) or 0
        mark(
            "method_labels_get_post",
            post_after - post_before == 1 and get_after - get_before == 1,
            {
                "post": [post_before, post_after],
                "get": [get_before, get_after],
                "route_label": "_api_",
            },
        )

        # Slow vs fast duration sum movement
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        sum_before = parse_counter(
            body.decode("utf-8", errors="replace"),
            "exyonq_http_request_duration_milliseconds_sum",
        )
        curl_req(f"http://127.0.0.1:{listen}/site/ok.txt")
        curl_req(f"http://127.0.0.1:{listen}/slow/")
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        sum_after = parse_counter(
            body.decode("utf-8", errors="replace"),
            "exyonq_http_request_duration_milliseconds_sum",
        )
        mark(
            "duration_sum_increases_with_slow",
            sum_before is not None
            and sum_after is not None
            and sum_after - sum_before >= 50,
            {"sum_before": sum_before, "sum_after": sum_after, "min_delta_ms": 50},
        )

        # Proxy upstream down → 5xx (connection refused after killing peer temporarily)
        # Use a dedicated dead port upstream via reload
        dead = pick_port()  # unbound
        cfg_path.write_text(
            cfg_text(listen, www, dead, metrics_enabled=True, include_proxy=True)
        )
        r = subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
        )
        mark("reload_dead_upstream", r.returncode == 0, {"rc": r.returncode})
        time.sleep(0.15)
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        before_5xx = parse_counter_int(
            body.decode("utf-8", errors="replace"), "exyonq_http_responses_5xx_total"
        )
        c502, _, _ = curl_req(f"http://127.0.0.1:{listen}/api/")
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        after_5xx = parse_counter_int(
            body.decode("utf-8", errors="replace"), "exyonq_http_responses_5xx_total"
        )
        mark(
            "proxy_failure_5xx_metric",
            c502 in (502, 503, 504)
            and before_5xx is not None
            and after_5xx is not None
            and after_5xx - before_5xx >= 1,
            {"wire_status": c502, "before_5xx": before_5xx, "after_5xx": after_5xx},
        )

        # Restore live peer
        cfg_path.write_text(
            cfg_text(listen, www, peer_port, metrics_enabled=True, include_proxy=True)
        )
        r = subprocess.run(
            [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
            capture_output=True,
            text=True,
        )
        mark("reload_restore_peer", r.returncode == 0, {"rc": r.returncode})
        time.sleep(0.15)

        # Process-lifetime: counters must not reset across NO_OP-ish reload
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        pre_reload_total = parse_counter_int(
            body.decode("utf-8", errors="replace"), "exyonq_http_requests_total"
        )
        # Touch a field that still keeps metrics enabled (path rename back)
        cfg_path.write_text(
            cfg_text(
                listen,
                www,
                peer_port,
                metrics_enabled=True,
                metrics_path="/metrics",
                include_proxy=True,
            )
        )
        for i in range(3):
            r = subprocess.run(
                [str(CTL), "reload", "--config", str(cfg_path), "--socket", str(ctrl)],
                capture_output=True,
                text=True,
            )
            if r.returncode != 0:
                mark("repeated_reload", False, {"i": i, "out": (r.stdout or "")[:200]})
                break
        else:
            mark("repeated_reload", True, {"n": 3})
        time.sleep(0.15)
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        post_text = body.decode("utf-8", errors="replace")
        post_reload_total = parse_counter_int(post_text, "exyonq_http_requests_total")
        om_errs2 = validate_openmetrics(post_text)
        mark(
            "reload_no_counter_reset",
            pre_reload_total is not None
            and post_reload_total is not None
            and post_reload_total >= pre_reload_total
            and not om_errs2,
            {
                "pre": pre_reload_total,
                "post": post_reload_total,
                "om_errs": om_errs2,
            },
        )

        # Concurrent scrapes
        def one_scrape(_):
            c, b, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
            t = b.decode("utf-8", errors="replace")
            return c == 200 and not validate_openmetrics(t)

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as ex:
            results = list(ex.map(one_scrape, range(16)))
        mark(
            "concurrent_scrapes_valid",
            all(results),
            {"ok_count": sum(1 for x in results if x), "n": len(results)},
        )

        # High-cardinality audit: query string must not create distinct route labels
        curl_req(f"http://127.0.0.1:{listen}/site/ok.txt?x=1")
        curl_req(f"http://127.0.0.1:{listen}/site/ok.txt?x=2")
        code, body, _ = curl_req(f"http://127.0.0.1:{listen}/metrics")
        t = body.decode("utf-8", errors="replace")
        bad_qs = 'route="_site_ok.txt?x=' in t or "route=\"_site_ok.txt?x=" in t
        mark(
            "query_string_not_in_route_label",
            not bad_qs,
            {"bad_qs_label_seen": bad_qs},
        )

        # Secret leakage audit on exposition
        secret_hits = []
        for needle in (
            "Authorization",
            "Bearer ",
            "BEGIN PRIVATE",
            "password=",
            "api_key",
        ):
            if needle.lower() in t.lower():
                secret_hits.append(needle)
        mark("no_secret_leakage_in_exposition", not secret_hits, {"hits": secret_hits})

        # --- LA-CAP054-008 (global security; Cap054 stays administratively closed) ---
        # 1) Non-loopback metrics without bearer must fail closed at config validate / start.
        bad_pub = tmp / "la008-public-no-token.toml"
        pub_listen = pick_port()
        bad_pub.write_text(
            cfg_text(
                pub_listen,
                www,
                None,
                metrics_enabled=True,
                include_proxy=False,
                listen_host="0.0.0.0",
            )
        )
        reject = subprocess.run(
            [str(BINARY), "serve", "--config", str(bad_pub)],
            capture_output=True,
            text=True,
            cwd=str(WS),
            timeout=20,
        )
        reject_txt = ((reject.stdout or "") + (reject.stderr or ""))[:600]
        mark(
            "la008_non_loopback_without_token_rejected",
            reject.returncode != 0
            and ("scrape_bearer_token" in reject_txt or "LA-CAP054-008" in reject_txt),
            {"rc": reject.returncode, "out": reject_txt},
        )

        # 2) Non-loopback metrics with bearer: unauthenticated → 401; Bearer → 200.
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        token = "la008-e2e-token"
        pub_listen = pick_port()
        cfg_path.write_text(
            cfg_text(
                pub_listen,
                www,
                peer_port,
                metrics_enabled=True,
                include_proxy=True,
                listen_host="0.0.0.0",
                scrape_bearer_token=token,
            )
        )
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(pub_listen):
            mark("la008_public_startup", False, {"log_tail": log.read_text()[-800:]})
            raise RuntimeError("la008 public startup failed")
        mark("la008_public_startup", True, {"listen": pub_listen})

        code, _, _ = curl_req(f"http://127.0.0.1:{pub_listen}/metrics")
        mark("la008_public_scrape_without_token_401", code == 401, {"http_code": code})

        code, _, _ = curl_req(f"http://127.0.0.1:{pub_listen}/exyonq-metrics-health")
        mark(
            "la008_public_health_without_token_401",
            code == 401,
            {"http_code": code},
        )

        code, body, _ = curl_req(
            f"http://127.0.0.1:{pub_listen}/metrics",
            extra_headers=[f"Authorization: Bearer {token}"],
        )
        t_auth = body.decode("utf-8", errors="replace")
        mark(
            "la008_public_scrape_with_token_200",
            code == 200 and "exyonq_http_requests_total" in t_auth,
            {"http_code": code},
        )

        code, body, _ = curl_req(
            f"http://127.0.0.1:{pub_listen}/exyonq-metrics-health",
            extra_headers=[f"Authorization: Bearer {token}"],
        )
        mark(
            "la008_public_health_with_token_200",
            code == 200 and b'"status":"ok"' in body,
            {"http_code": code},
        )

        # 3) Rate limit precedes metrics short-circuit (burst=1 → second scrape 429).
        stop_proc(proc)
        proc = None
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass
        rl_listen = pick_port()
        cfg_path.write_text(
            cfg_text(
                rl_listen,
                www,
                None,
                metrics_enabled=True,
                include_proxy=False,
                ratelimit_enabled=True,
                ratelimit_rps=1,
                ratelimit_burst=1,
            )
        )
        proc = subprocess.Popen(
            [str(BINARY), "serve", "--config", str(cfg_path)],
            stdout=log.open("w"),
            stderr=subprocess.STDOUT,
            cwd=str(WS),
            env=env,
        )
        if not wait_sock(ctrl) or not wait_listen(rl_listen):
            mark("la008_rl_startup", False, {"log_tail": log.read_text()[-800:]})
            raise RuntimeError("la008 ratelimit startup failed")
        c1, _, _ = curl_req(f"http://127.0.0.1:{rl_listen}/metrics")
        c2, _, _ = curl_req(f"http://127.0.0.1:{rl_listen}/metrics")
        mark(
            "la008_ratelimit_applies_to_metrics_scrape",
            c1 == 200 and c2 == 429,
            {"first": c1, "second": c2},
        )

    except Exception as exc:
        ok = False
        checks["exception"] = {"ok": False, "error": str(exc)}
    finally:
        stop_proc(proc)
        try:
            httpd.shutdown()
        except Exception:
            pass
        if ctrl.exists():
            try:
                ctrl.unlink()
            except OSError:
                pass

    final = "PASS_REAL_PRODUCTION" if ok else "FAIL"
    result = {
        "CAPABILITY_ID": "054",
        "FEATURE_ID": "metrics-prometheus",
        "FINAL_RESULT": final,
        "ARCH_LABEL": ARCH_LABEL,
        "HOST_LABEL": HOST_LABEL,
        "HEAD": HEAD,
        "BINARY": str(BINARY),
        "BINARY_SHA256": sha256_file(BINARY) if BINARY.is_file() else None,
        "SCENARIOS_PASS": scenarios_pass,
        "SCENARIOS_TOTAL": scenarios_total,
        "ZERO_FAKE": "PASS" if ok else "FAIL",
        "SCRAPE_INCREMENTS_REQUEST_METRICS": True,
        "DURATION_SEMANTICS": "response_head_on_request_to_on_response",
        "METRICS_ENDPOINT": "/metrics",
        "METRICS_EXPOSURE_MODEL": "same_listener",
        "CONTENT_TYPE": "application/openmetrics-text; version=1.0.0; charset=utf-8",
        "CHECKS": checks,
        "UTC": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    }
    OUT.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    print(f"FINAL_RESULT={final} scenarios={scenarios_pass}/{scenarios_total}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
