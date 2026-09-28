#!/usr/bin/env python3
"""Phase 8 independent raw-socket HTTP/1.1 attacker. Stdlib only. Not product code."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import socket
import time
from pathlib import Path


def split_listen(listen: str) -> tuple[str, int]:
    host, port = listen.rsplit(":", 1)
    return host, int(port)


def recv_until_idle(sock: socket.socket, timeout: float = 1.5, max_bytes: int = 1_048_576) -> bytes:
    sock.settimeout(timeout)
    buf = bytearray()
    deadline = time.time() + timeout
    while time.time() < deadline and len(buf) < max_bytes:
        try:
            chunk = sock.recv(65536)
        except socket.timeout:
            break
        if not chunk:
            break
        buf.extend(chunk)
        if b"\r\n\r\n" in buf:
            # keep reading a little for body
            sock.settimeout(0.25)
    return bytes(buf)


def parse_http(raw: bytes) -> dict:
    if not raw:
        return {"status": None, "headers": {}, "body": b"", "raw_len": 0, "wire_sha256": hashlib.sha256(raw).hexdigest()}
    head, _, body = raw.partition(b"\r\n\r\n")
    lines = head.split(b"\r\n")
    status = None
    if lines:
        parts = lines[0].split()
        if len(parts) >= 2 and parts[1].isdigit():
            status = int(parts[1])
    headers = {}
    for line in lines[1:]:
        if b":" not in line:
            continue
        k, v = line.split(b":", 1)
        headers[k.decode("latin1", "replace").lower()] = v.strip().decode("latin1", "replace")
    return {
        "status": status,
        "headers": headers,
        "body": body,
        "raw_len": len(raw),
        "wire_sha256": hashlib.sha256(raw).hexdigest(),
        "body_sha256": hashlib.sha256(body).hexdigest(),
        "body_text": body.decode("utf-8", "replace")[:400],
    }


def exchange(listen: str, payload: bytes, timeout: float = 2.0, linger: bool = False) -> dict:
    host, port = split_listen(listen)
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect((host, port))
        s.sendall(payload)
        if linger:
            time.sleep(0.15)
        raw = recv_until_idle(s, timeout=timeout)
        return parse_http(raw)
    except OSError as e:
        return {"status": None, "error": str(e), "raw_len": 0, "body": b"", "headers": {}}
    finally:
        try:
            s.close()
        except OSError:
            pass


def exchange_two(listen: str, first: bytes, second: bytes | None, timeout: float = 2.0) -> dict:
    host, port = split_listen(listen)
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(timeout)
    try:
        s.connect((host, port))
        s.sendall(first)
        first_raw = recv_until_idle(s, timeout=timeout)
        second_raw = b""
        if second is not None:
            try:
                s.sendall(second)
                second_raw = recv_until_idle(s, timeout=timeout)
            except OSError:
                pass
        return {"first": parse_http(first_raw), "second": parse_http(second_raw), "conn_alive_after_first": bool(first_raw)}
    except OSError as e:
        return {"first": {"status": None, "error": str(e)}, "second": {"status": None}}
    finally:
        try:
            s.close()
        except OSError:
            pass


def req(method: str, path: str, headers: list[tuple[str, str]], body: bytes = b"") -> bytes:
    lines = [f"{method} {path} HTTP/1.1"]
    for k, v in headers:
        lines.append(f"{k}: {v}")
    head = ("\r\n".join(lines) + "\r\n\r\n").encode("latin1")
    return head + body


def dump(outdir: Path, name: str, obj: dict) -> None:
    serial = json.loads(json.dumps(obj, default=lambda o: o.decode("latin1", "replace") if isinstance(o, (bytes, bytearray)) else o))
    (outdir / f"{name}.json").write_text(json.dumps(serial, indent=2) + "\n")


def run_framing(listen: str, outdir: Path) -> dict:
    cases = {}
    cases["dup_cl_same"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Content-Length", "4"), ("Content-Length", "4")], b"AAAA"),
    )
    cases["dup_cl_diff"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Content-Length", "4"), ("Content-Length", "8")], b"AAAABBBB"),
    )
    cases["cl_te"] = exchange(
        listen,
        req(
            "POST",
            "/api/echo",
            [("Host", "p8.local"), ("Content-Length", "4"), ("Transfer-Encoding", "chunked")],
            b"AAAA",
        ),
    )
    cases["te_only"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Transfer-Encoding", "chunked")], b"4\r\nAAAA\r\n0\r\n\r\n"),
    )
    cases["te_malformed"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Transfer-Encoding", "chunked, identity")]),
    )
    cases["obs_fold"] = exchange(
        listen,
        b"GET /static/hello.txt HTTP/1.1\r\nHost: p8.local\r\nX-Fold: start\r\n continued\r\n\r\n",
    )
    cases["bare_lf"] = exchange(listen, b"GET /static/hello.txt HTTP/1.1\nHost: p8.local\n\n")
    cases["missing_host"] = exchange(listen, b"GET /static/hello.txt HTTP/1.1\r\n\r\n")
    cases["http10_no_host"] = exchange(listen, b"GET /static/hello.txt HTTP/1.0\r\n\r\n")
    cases["premature_eof"] = exchange(listen, b"POST /api/echo HTTP/1.1\r\nHost: p8.local\r\nContent-Length: 100\r\n\r\nshort")
    cases["body_longer_than_cl"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Content-Length", "4")], b"AAAABBBBEXTRA"),
    )
    cases["body_shorter_timeout"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Content-Length", "20")], b"short"),
        timeout=1.0,
    )
    cases["chunked_malformed"] = exchange(
        listen,
        req("POST", "/api/echo", [("Host", "p8.local"), ("Transfer-Encoding", "chunked")], b"ZZ\r\nnope"),
    )
    dump(outdir, "framing", cases)
    return cases


def run_desync(listen: str, outdir: Path) -> dict:
    # Ambiguous framing then a second GET on the same connection.
    smuggle = (
        b"POST /api/echo HTTP/1.1\r\nHost: p8.local\r\nContent-Length: 4\r\n"
        b"Transfer-Encoding: chunked\r\n\r\nAAAA"
        b"GET /static/hello.txt HTTP/1.1\r\nHost: p8.local\r\n\r\n"
    )
    dual_cl = (
        b"POST /api/echo HTTP/1.1\r\nHost: p8.local\r\nContent-Length: 0\r\nContent-Length: 44\r\n\r\n"
        b"GET /static/hello.txt HTTP/1.1\r\nHost: p8.local\r\n\r\n"
    )
    extra_after_cl = req("POST", "/api/echo", [("Host", "p8.local"), ("Content-Length", "4")], b"AAAA") + req(
        "GET", "/static/hello.txt", [("Host", "p8.local")]
    )
    cases = {
        "cl_te_then_get": exchange(listen, smuggle),
        "dup_cl_then_get": exchange(listen, dual_cl),
        "pipeline_get_get": exchange(
            listen,
            req("GET", "/static/hello.txt", [("Host", "p8.local"), ("Connection", "keep-alive")])
            + req("GET", "/api/item?q=pipe2", [("Host", "p8.local")]),
        ),
        "reject_then_second": exchange_two(
            listen,
            req("POST", "/api/echo", [("Host", "p8.local"), ("Transfer-Encoding", "chunked")]),
            req("GET", "/api/item?q=after-reject", [("Host", "p8.local")]),
        ),
        "extra_after_exact_cl": exchange(listen, extra_after_cl),
    }
    dump(outdir, "desync", cases)
    return cases


def run_host(listen: str, outdir: Path) -> dict:
    cases = {}
    cases["dup_host"] = exchange(
        listen,
        b"GET /host-static/hello.txt HTTP/1.1\r\nHost: p8.example\r\nHost: evil.example\r\n\r\n",
    )
    cases["missing_host_11"] = exchange(listen, b"GET /static/hello.txt HTTP/1.1\r\nConnection: close\r\n\r\n")
    cases["absolute_form"] = exchange(
        listen,
        b"GET http://evil.example/static/hello.txt HTTP/1.1\r\nHost: p8.example\r\n\r\n",
    )
    cases["host_port"] = exchange(listen, req("GET", "/host-static/hello.txt", [("Host", "p8.example:80")]))
    cases["host_case"] = exchange(listen, req("GET", "/host-static/hello.txt", [("Host", "P8.EXAMPLE")]))
    cases["trailing_dot"] = exchange(listen, req("GET", "/host-static/hello.txt", [("Host", "p8.example.")]))
    cases["wildcard"] = exchange(listen, req("GET", "/host-wild/hello.txt", [("Host", "a.p8.test")]))
    cases["wildcard_boundary"] = exchange(listen, req("GET", "/host-wild/hello.txt", [("Host", "p8.test.evil")]))
    cases["hostless_fallback"] = exchange(listen, req("GET", "/static/hello.txt", [("Host", "unknown.invalid")]))
    cases["userinfo_like"] = exchange(
        listen,
        b"GET /static/hello.txt HTTP/1.1\r\nHost: p8.example@evil\r\n\r\n",
    )
    dump(outdir, "host", cases)
    return cases


def run_path(listen: str, outdir: Path) -> dict:
    paths = [
        ("slash2", "/static//hello.txt"),
        ("slash3", "/static///hello.txt"),
        ("dot", "/static/./hello.txt"),
        ("dotdot", "/static/../hello.txt"),
        ("dotdot_enc", "/static/%2e%2e/hello.txt"),
        ("enc_slash", "/static/hello%2ftxt"),
        ("enc_backslash", "/static/hello%5ctxt"),
        ("backslash", "/static/hello\\txt"),
        ("long", "/static/" + ("a" * 3000)),
        ("empty_seg", "/static//subdir//index.html"),
        ("prefix_api_as_static", "/static/api/item"),
        ("php_on_static", "/static/secret.php"),
        ("enc_php", "/static/secret%2ephp"),
        ("fcgi_dotdot", "/app/../static/hello.txt"),
        ("fcgi_enc_dotdot", "/app/%2e%2e/secret"),
        ("proxy_abs", "/api/http://evil/"),
        ("proxy_enc_auth", "/api/%2f%2fevil.example/"),
    ]
    cases = {}
    for name, path in paths:
        cases[name] = exchange(listen, req("GET", path, [("Host", "p8.local")]))
        cases[name]["path"] = path
    dump(outdir, "path", cases)
    return cases


def run_keepalive(listen: str, outdir: Path) -> dict:
    seqs = [
        ("static_static", ["/static/hello.txt", "/static/hello.txt"]),
        ("static_proxy", ["/static/hello.txt", "/api/item?q=ka1"]),
        ("static_fcgi", ["/static/hello.txt", "/app/?m=ka1"]),
        ("proxy_static", ["/api/item?q=ka2", "/static/hello.txt"]),
        ("proxy_fcgi", ["/api/item?q=ka3", "/app/?m=ka2"]),
        ("fcgi_static", ["/app/?m=ka3", "/static/hello.txt"]),
        ("fcgi_proxy", ["/app/?m=ka4", "/api/item?q=ka4"]),
        ("fcgi_fcgi", ["/app/?m=ka5", "/app/?m=ka6"]),
    ]
    host, port = split_listen(listen)
    out = {}
    for name, paths in seqs:
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(3)
        bodies = []
        try:
            s.connect((host, port))
            for p in paths:
                s.sendall(req("GET", p, [("Host", "p8.local"), ("Connection", "keep-alive")]))
                bodies.append(parse_http(recv_until_idle(s, timeout=2.0)))
            out[name] = {"ok": True, "responses": bodies}
        except OSError as e:
            out[name] = {"ok": False, "error": str(e), "partial": bodies}
        finally:
            s.close()
    dump(outdir, "keepalive", out)
    return out


def run_pipeline_contract(listen: str, outdir: Path) -> dict:
    payload = req("GET", "/static/hello.txt", [("Host", "p8.local")]) + req(
        "GET", "/api/item?q=pipeline", [("Host", "p8.local")]
    )
    r = exchange(listen, payload)
    dump(outdir, "pipeline", r)
    return r


def classify_framing(cases: dict) -> dict:
    fail_closed = 0
    unexpected_200 = []
    for k, v in cases.items():
        st = v.get("status") if isinstance(v, dict) else None
        if st in (400, 408, 411, 413, 417, 501, None) or (isinstance(st, int) and st >= 400):
            fail_closed += 1
        elif k in (
            "dup_cl_same",
            "dup_cl_diff",
            "cl_te",
            "te_only",
            "te_malformed",
            "bare_lf",
            "missing_host",
            "chunked_malformed",
        ) and st == 200:
            unexpected_200.append(k)
    return {"fail_closed_or_error_count": fail_closed, "unexpected_success": unexpected_200}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--listen", required=True)
    ap.add_argument("--outdir", required=True)
    ap.add_argument("--cmd", required=True)
    args = ap.parse_args()
    outdir = Path(args.outdir)
    outdir.mkdir(parents=True, exist_ok=True)
    cmd = args.cmd
    if cmd == "framing":
        cases = run_framing(args.listen, outdir)
        summary = classify_framing(cases)
        (outdir / "framing_summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary))
    elif cmd == "desync":
        cases = run_desync(args.listen, outdir)
        print(json.dumps({k: (v.get("status") if isinstance(v, dict) else v) for k, v in cases.items()}, default=str))
    elif cmd == "host":
        run_host(args.listen, outdir)
        print("host_ok")
    elif cmd == "path":
        run_path(args.listen, outdir)
        print("path_ok")
    elif cmd == "keepalive":
        run_keepalive(args.listen, outdir)
        print("keepalive_ok")
    elif cmd == "pipeline":
        run_pipeline_contract(args.listen, outdir)
        print("pipeline_ok")
    else:
        raise SystemExit(f"unknown cmd {cmd}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
