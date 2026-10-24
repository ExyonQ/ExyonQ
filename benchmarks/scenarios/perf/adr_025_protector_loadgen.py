#!/usr/bin/env python3
"""ADR-025 protector matrix loadgen — diagnostic only, not publishable."""
from __future__ import annotations

import argparse
import asyncio
import json
import ssl
import subprocess
import sys
import threading
import time
from typing import Any, Callable

READ_CHUNK_TIMEOUT_S = 5.0
REQUEST_TIMEOUT_DEFAULT_S = 30.0
REQUEST_TIMEOUT_LARGE_S = 120.0
LARGE_BODY_THRESHOLD = 65536

SCENARIOS: dict[str, dict[str, Any]] = {
    "p1": {"path": "/site/1k.bin", "body": 1024, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p2": {"path": "/site/64k.bin", "body": 65536, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p3": {"path": "/site/1m.bin", "body": 1048576, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p4": {"path": "/api/", "body": 1024, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p5": {"path": "/api/", "body": 1024, "conns": 100, "port": None, "https": False, "close": True, "sse": False},
    "p6": {"path": "/api/", "body": 1024, "conns": 200, "port": None, "https": False, "close": False, "sse": False},
    "p7": {"path": "/site/routes/route050.bin", "body": 512, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p8": {"path": "/api/stream", "body": 0, "conns": 100, "port": None, "https": False, "close": False, "sse": True},
    "p9": {"path": "/site/64k.bin", "body": 65536, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p10": {"path": "/api/", "body": 1024, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p11": {"path": "/site/1k.bin", "body": 1024, "conns": 100, "port": "tls", "https": True, "close": False, "sse": False},
    "p12": {"path": "/api/", "body": 1024, "conns": 100, "port": None, "https": False, "close": False, "sse": False},
    "p13": {"path": "/site/1k.bin", "body": 1024, "conns": 100, "port": "http3", "https": True, "close": False, "sse": False, "http3": True},
}


class HarnessInvalidError(RuntimeError):
    """Loader/harness issue — not a protector epoll vs blocking signal."""

    def __init__(self, message: str, *, code: str = "HARNESS-INVALID") -> None:
        super().__init__(message)
        self.code = code


class ServerResponseError(RuntimeError):
    """Unexpected HTTP status or body from server under test."""


def pct(vals: list[float], p: float) -> float | None:
    if not vals:
        return None
    vals = sorted(vals)
    return vals[int(round((p / 100.0) * (len(vals) - 1)))]


def request_timeout_s(expect_body: int) -> float:
    if expect_body > LARGE_BODY_THRESHOLD:
        return REQUEST_TIMEOUT_LARGE_S
    return REQUEST_TIMEOUT_DEFAULT_S


def parse_status_and_headers(header_block: bytes) -> tuple[str, dict[str, str]]:
    text = header_block.decode("latin-1", "replace")
    lines = text.split("\r\n")
    status = lines[0] if lines else ""
    headers: dict[str, str] = {}
    for line in lines[1:]:
        if ":" in line:
            key, value = line.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    return status, headers


async def read_with_timeout(reader: asyncio.StreamReader, limit: int, timeout_s: float) -> bytes:
    try:
        return await asyncio.wait_for(reader.read(limit), timeout=timeout_s)
    except asyncio.TimeoutError as exc:
        raise HarnessInvalidError(f"read timeout after {timeout_s}s", code="INVALID-LOADGEN") from exc


async def read_http_response(
    reader: asyncio.StreamReader,
    expect_body: int,
    *,
    sse: bool = False,
    close: bool = False,
) -> tuple[str, bytes]:
    """Read one HTTP response with per-read and whole-request timeouts."""
    deadline = time.time() + request_timeout_s(expect_body)
    buf = b""

    while time.time() < deadline:
        remaining = max(0.1, deadline - time.time())
        chunk_timeout = min(READ_CHUNK_TIMEOUT_S, remaining)
        chunk = await read_with_timeout(reader, 65536, chunk_timeout)
        if not chunk:
            break
        buf += chunk
        he = buf.find(b"\r\n\r\n")
        if he < 0:
            continue

        header_block = buf[: he + 4]
        status, headers = parse_status_and_headers(header_block)

        if sse:
            content_type = headers.get("content-type", "").lower()
            if status.startswith("HTTP/1.1 200") and "text/event-stream" in content_type:
                return status, b""
            raise ServerResponseError(
                f"sse status/content-type unexpected: {status[:60]!r} "
                f"content-type={content_type!r}"
            )

        if not status.startswith("HTTP/1.1 200"):
            body_preview = buf[he + 4 : he + 4 + 64]
            raise ServerResponseError(
                f"status {status[:60]!r} body_preview={body_preview!r}"
            )

        if expect_body == 0:
            return status, b""

        body_start = he + 4
        content_length = headers.get("content-length")
        if content_length is not None:
            try:
                cl = int(content_length)
            except ValueError as exc:
                raise HarnessInvalidError(f"invalid Content-Length: {content_length!r}") from exc
            while len(buf) - body_start < cl:
                remaining = max(0.1, deadline - time.time())
                if remaining <= 0:
                    raise HarnessInvalidError(
                        f"incomplete body: have {len(buf) - body_start} need {cl}",
                        code="INVALID-LOADGEN",
                    )
                chunk = await read_with_timeout(
                    reader, cl - (len(buf) - body_start), min(READ_CHUNK_TIMEOUT_S, remaining)
                )
                if not chunk:
                    if close and len(buf) - body_start >= cl:
                        break
                    raise HarnessInvalidError(
                        f"short body: have {len(buf) - body_start} need {cl}",
                        code="INVALID-LOADGEN",
                    )
                buf += chunk
            body = buf[body_start : body_start + cl]
            if len(body) != expect_body:
                raise ServerResponseError(f"body {len(body)} != expected {expect_body}")
            return status, body

        while len(buf) - body_start < expect_body:
            remaining = max(0.1, deadline - time.time())
            if remaining <= 0:
                raise HarnessInvalidError(
                    f"incomplete body without Content-Length: have {len(buf) - body_start}",
                    code="INVALID-LOADGEN",
                )
            chunk = await read_with_timeout(
                reader, expect_body - (len(buf) - body_start), min(READ_CHUNK_TIMEOUT_S, remaining)
            )
            if not chunk:
                raise HarnessInvalidError(
                    f"short body without Content-Length: have {len(buf) - body_start}",
                    code="INVALID-LOADGEN",
                )
            buf += chunk
        body = buf[body_start : body_start + expect_body]
        return status, body

    raise HarnessInvalidError(f"request timeout len={len(buf)}", code="INVALID-LOADGEN")


def classify_error(exc: Exception) -> str:
    if isinstance(exc, HarnessInvalidError):
        return exc.code
    if isinstance(exc, ServerResponseError):
        return "SERVER-WATCH"
    return "REQUEST-FAIL"


async def open_conn(
    host: str,
    port: int,
    *,
    https: bool,
    ssl_ctx: ssl.SSLContext | None,
) -> tuple[asyncio.StreamReader, asyncio.StreamWriter]:
    kw: dict[str, Any] = {}
    if https:
        assert ssl_ctx is not None
        kw["ssl"] = ssl_ctx
    return await asyncio.wait_for(asyncio.open_connection(host, port, **kw), timeout=30)


async def worker_loop(
    cfg: dict[str, Any],
    *,
    host: str,
    port: int,
    path: str,
    expect: int,
    measure_end: float,
    ssl_ctx: ssl.SSLContext | None,
    stats: dict[str, Any],
    lock: threading.Lock,
    should_stop: Callable[[], bool] | None = None,
) -> None:
    req_hdr = f"GET {path} HTTP/1.1\r\nHost: localhost\r\n"
    if cfg["close"]:
        req_hdr += "Connection: close\r\n"
    else:
        req_hdr += "Connection: keep-alive\r\n"
    req = (req_hdr + "\r\n").encode()
    session: tuple[asyncio.StreamReader, asyncio.StreamWriter] | None = None

    while time.time() < measure_end:
        if should_stop is not None and should_stop():
            break
        try:
            if cfg["close"] or session is None:
                if session is not None:
                    try:
                        session[1].close()
                        await session[1].wait_closed()
                    except Exception:
                        pass
                session = await open_conn(host, port, https=cfg["https"], ssl_ctx=ssl_ctx)
            t0 = time.perf_counter()
            session[1].write(req)
            await session[1].drain()
            _status, body = await read_http_response(
                session[0], expect, sse=cfg["sse"], close=cfg["close"]
            )
            if expect > 0 and len(body) != expect:
                raise ServerResponseError(f"body {len(body)} != {expect}")
            dt_ms = (time.perf_counter() - t0) * 1000.0
            with lock:
                stats["ok"] += 1
                if len(stats["lat_ms"]) < 100000:
                    stats["lat_ms"].append(dt_ms)
            if cfg["close"] or cfg["sse"]:
                try:
                    session[1].close()
                    await session[1].wait_closed()
                except Exception:
                    pass
                session = None
        except Exception as exc:
            code = classify_error(exc)
            with lock:
                stats["fail"] += 1
                stats["error_codes"][code] = stats["error_codes"].get(code, 0) + 1
                if len(stats["errors"]) < 15:
                    stats["errors"].append(f"{code}: {exc}")
            if session is not None:
                try:
                    session[1].close()
                    await session[1].wait_closed()
                except Exception:
                    pass
                session = None
            if cfg["close"] or cfg["sse"]:
                continue
            return

    if session is not None:
        try:
            session[1].close()
            await session[1].wait_closed()
        except Exception:
            pass


def run_http3_load(host: str, port: int, path: str, duration_s: int) -> tuple[int, int, float, list[float]]:
    url = f"https://{host}:{port}{path}"
    end = time.time() + duration_s
    ok = fail = 0
    lat: list[float] = []
    while time.time() < end:
        t0 = time.perf_counter()
        try:
            proc = subprocess.run(
                ["curl", "--http3-only", "-sk", "-o", "/dev/null", "-w", "%{http_code}", url],
                capture_output=True,
                text=True,
                timeout=30,
            )
            code = (proc.stdout or "").strip()
            if code == "200":
                ok += 1
                lat.append((time.perf_counter() - t0) * 1000.0)
            else:
                fail += 1
        except Exception:
            fail += 1
    elapsed = max(duration_s, 0.001)
    return ok, fail, elapsed, lat


def summarize_harness(stats: dict[str, Any]) -> str | None:
    codes = stats.get("error_codes") or {}
    if not stats.get("fail", 0):
        return None
    if stats.get("ok", 0) == 0 and codes:
        non_protector = ("HARNESS-INVALID", "INVALID-LOADGEN", "REQUEST-FAIL")
        if all(c in non_protector for c in codes):
            return "HARNESS-INVALID"
        if all(c in ("HARNESS-INVALID", "INVALID-LOADGEN") for c in codes):
            return "HARNESS-INVALID"
        if all(c == "SERVER-WATCH" for c in codes):
            return "SERVER-WATCH"
        if "HARNESS-INVALID" in codes or "INVALID-LOADGEN" in codes:
            return "HARNESS-INVALID"
    return None


async def main_async(
    sid: str,
    out_path: str,
    host_port: int,
    tls_port: int,
    http3_port: int,
    warmup: int,
    measure: int,
) -> dict[str, Any]:
    cfg = SCENARIOS[sid]
    port_key = cfg.get("port")
    if port_key == "tls":
        port = tls_port
    elif port_key == "http3":
        port = http3_port
    else:
        port = host_port
    path = cfg["path"]
    expect = cfg["body"]
    conns = cfg["conns"]
    host = "127.0.0.1"

    if cfg.get("http3"):
        ok, fail, elapsed, lat = run_http3_load(host, port, path, measure)
        out = {
            "scenario": sid,
            "path": path,
            "http3": True,
            "conns": conns,
            "warmup_s": 0,
            "measure_s": measure,
            "ok": ok,
            "fail": fail,
            "success_rate": ok / max(ok + fail, 1),
            "rps": ok / elapsed,
            "p50_ms": pct(lat, 50),
            "p95_ms": pct(lat, 95),
            "p99_ms": pct(lat, 99),
            "elapsed_s": elapsed,
            "sample_errors": [],
            "harness_classification": None,
        }
        with open(out_path, "w", encoding="utf-8") as fh:
            json.dump(out, fh, indent=2)
        return out

    ssl_ctx = ssl.create_default_context()
    ssl_ctx.check_hostname = False
    ssl_ctx.verify_mode = ssl.CERT_NONE

    lock = threading.Lock()
    stats: dict[str, Any] = {"ok": 0, "fail": 0, "lat_ms": [], "errors": [], "error_codes": {}}

    warm_end = time.time() + warmup
    warm_tasks = [
        asyncio.create_task(
            worker_loop(
                cfg,
                host=host,
                port=port,
                path=path,
                expect=expect,
                measure_end=warm_end,
                ssl_ctx=ssl_ctx,
                stats=stats,
                lock=lock,
            )
        )
        for _ in range(conns)
    ]
    await asyncio.gather(*warm_tasks, return_exceptions=True)

    with lock:
        stats["ok"] = 0
        stats["fail"] = 0
        stats["lat_ms"] = []
        stats["errors"] = []
        stats["error_codes"] = {}

    measure_end = time.time() + measure
    t0 = time.time()
    measure_tasks = [
        asyncio.create_task(
            worker_loop(
                cfg,
                host=host,
                port=port,
                path=path,
                expect=expect,
                measure_end=measure_end,
                ssl_ctx=ssl_ctx,
                stats=stats,
                lock=lock,
            )
        )
        for _ in range(conns)
    ]
    await asyncio.gather(*measure_tasks, return_exceptions=True)
    elapsed = max(time.time() - t0, 0.001)

    harness_class = summarize_harness(stats)
    out = {
        "scenario": sid,
        "path": path,
        "expect_body": expect,
        "conns": conns,
        "warmup_s": warmup,
        "measure_s": measure,
        "ok": stats["ok"],
        "fail": stats["fail"],
        "success_rate": stats["ok"] / max(stats["ok"] + stats["fail"], 1),
        "rps": stats["ok"] / elapsed,
        "p50_ms": pct(stats["lat_ms"], 50),
        "p95_ms": pct(stats["lat_ms"], 95),
        "p99_ms": pct(stats["lat_ms"], 99),
        "elapsed_s": elapsed,
        "sample_errors": stats["errors"][:10],
        "error_codes": stats["error_codes"],
        "harness_classification": harness_class,
    }
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(out, fh, indent=2)
    return out


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="ADR-025 protector matrix loadgen")
    parser.add_argument("scenario")
    parser.add_argument("out_json")
    parser.add_argument("host_port", type=int)
    parser.add_argument("tls_port", type=int)
    parser.add_argument("http3_port", type=int)
    parser.add_argument("warmup", type=int)
    parser.add_argument("measure", type=int)
    args = parser.parse_args(argv)
    out = asyncio.run(
        main_async(
            args.scenario,
            args.out_json,
            args.host_port,
            args.tls_port,
            args.http3_port,
            args.warmup,
            args.measure,
        )
    )
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
