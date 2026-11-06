#!/usr/bin/env bash
# Real WebSocket E2E through an already-running ExyonQ proxy.
#
# Required environment:
#   HA_WS_URL       Home Assistant WebSocket URL through ExyonQ
#                   (for example ws://127.0.0.1:18080/api/websocket)
#   N8N_BASE_URL    n8n HTTP base URL through ExyonQ
#                   (for example http://127.0.0.1:18081)
#   N8N_COOKIE_FILE Netscape cookie jar with an authenticated n8n session.
#                   The file is read locally and never printed.
#   N8N_ORIGIN      Optional Origin accepted by n8n (default http://127.0.0.1).
#                   n8n rejects the proxy port when its editor origin is the
#                   bare loopback host, so this must match the n8n deployment.
#
# This script does not start Home Assistant, n8n or ExyonQ. It verifies the
# real applications and their real WebSocket traffic through the proxy; it is
# not a synthetic echo test.
set -euo pipefail

: "${HA_WS_URL:?HA_WS_URL is required}"
: "${N8N_BASE_URL:?N8N_BASE_URL is required}"
: "${N8N_COOKIE_FILE:?N8N_COOKIE_FILE is required}"

command -v python3 >/dev/null 2>&1 || {
  echo "FAIL: python3 is required"
  exit 2
}
python3 -c 'import websockets' >/dev/null 2>&1 || {
  echo "FAIL: the python websockets package is required"
  exit 2
}

python3 - <<'PY'
import asyncio
import http.client
import json
import os
import pathlib
import urllib.parse

import websockets

HA_WS_URL = os.environ["HA_WS_URL"]
N8N_BASE_URL = os.environ["N8N_BASE_URL"]
COOKIE_FILE = pathlib.Path(os.environ["N8N_COOKIE_FILE"])
PUSH_REF = f"exyonq-e2e-{os.getpid()}"


def load_n8n_cookie() -> str:
    if not COOKIE_FILE.is_file():
        raise SystemExit(f"FAIL: cookie jar not found: {COOKIE_FILE}")
    for raw in COOKIE_FILE.read_text(encoding="utf-8").splitlines():
        line = raw
        if line.startswith("#HttpOnly_"):
            line = line[len("#HttpOnly_") :]
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) >= 7 and fields[5] == "n8n-auth":
            return f"n8n-auth={fields[6]}"
    raise SystemExit("FAIL: n8n-auth cookie not found in the cookie jar")


COOKIE = load_n8n_cookie()
BASE = urllib.parse.urlsplit(N8N_BASE_URL)
if BASE.scheme not in ("http", "https") or not BASE.hostname:
    raise SystemExit(f"FAIL: unsupported N8N_BASE_URL: {N8N_BASE_URL}")
ORIGIN = os.environ.get("N8N_ORIGIN", "http://127.0.0.1")
WS_BASE = ("wss" if BASE.scheme == "https" else "ws") + "://" + BASE.netloc


def n8n_request(method: str, path: str, body=None, push: bool = False):
    conn_cls = http.client.HTTPSConnection if BASE.scheme == "https" else http.client.HTTPConnection
    conn = conn_cls(BASE.hostname, BASE.port, timeout=20)
    headers = {
        "Cookie": COOKIE,
        "Origin": ORIGIN,
        "User-Agent": "exyonq-ws-real-apps-e2e/0.9.1",
    }
    if push:
        headers["push-ref"] = PUSH_REF
    if body is not None:
        headers["Content-Type"] = "application/json"
    try:
        conn.request(method, BASE.path.rstrip("/") + path, body=body, headers=headers)
        response = conn.getresponse()
        return response.status, response.read()
    finally:
        conn.close()


def delete_workflow(workflow_id: str) -> None:
    for method, path in (
        ("POST", f"/rest/workflows/{workflow_id}/archive"),
        ("DELETE", f"/rest/workflows/{workflow_id}"),
    ):
        try:
            status, _ = n8n_request(method, path, body="{}" if method == "POST" else None)
            print(f"N8N_CLEANUP {method} {path} STATUS {status}")
        except Exception as exc:  # cleanup must not hide the product assertion
            print(f"N8N_CLEANUP_WARNING {type(exc).__name__}")


async def check_home_assistant() -> None:
    async with websockets.connect(HA_WS_URL, open_timeout=8) as ws:
        first = await asyncio.wait_for(ws.recv(), 8)
        print("HA_FIRST", first[:300])
        if json.loads(first).get("type") != "auth_required":
            raise AssertionError("Home Assistant did not request authentication")
        await ws.send(json.dumps({"type": "auth", "access_token": "invalid-e2e"}))
        second = await asyncio.wait_for(ws.recv(), 8)
        print("HA_SECOND", second[:300])
        if json.loads(second).get("type") != "auth_invalid":
            raise AssertionError("Home Assistant did not reject the invalid token")
    print("HA_WS_BIDIRECTIONAL_PASS")


async def check_n8n() -> None:
    workflow = {
        "name": f"ExyonQ WS real apps {os.getpid()}",
        "nodes": [
            {
                "parameters": {},
                "id": "11111111-1111-4111-8111-111111111112",
                "name": "NoOp",
                "type": "n8n-nodes-base.noOp",
                "typeVersion": 1,
                "position": [0, 0],
            }
        ],
        "connections": {},
        "settings": {},
        "active": False,
    }
    status, body = n8n_request("POST", "/rest/workflows", json.dumps(workflow))
    print("N8N_CREATE_STATUS", status)
    if status != 200:
        raise AssertionError(f"n8n workflow creation failed: {body[:500]!r}")
    workflow_id = json.loads(body)["data"]["id"]
    print("N8N_WORKFLOW_ID", workflow_id)
    try:
        ws_url = f"{WS_BASE}/rest/push?pushRef={urllib.parse.quote(PUSH_REF)}"
        async with websockets.connect(
            ws_url,
            origin=ORIGIN,
            additional_headers={
                "Cookie": COOKIE,
                "User-Agent": "exyonq-ws-real-apps-e2e/0.9.1",
            },
            open_timeout=5,
            ping_interval=None,
        ) as ws:
            print("N8N_WS_OPEN")
            status, body = n8n_request(
                "POST",
                f"/rest/workflows/{workflow_id}/run",
                json.dumps(
                    {"destinationNode": {"nodeName": "NoOp", "mode": "inclusive"}}
                ),
                push=True,
            )
            print("N8N_RUN_STATUS", status, "N8N_RUN_BODY", body[:500])
            if status != 200:
                raise AssertionError(f"n8n workflow run failed: {body[:500]!r}")
            frames = []
            while True:
                message = await asyncio.wait_for(ws.recv(), 10)
                text = str(message)
                frames.append(text)
                print("N8N_FRAME", repr(message)[:1000])
                if '"executionFinished"' in text:
                    break
                if len(frames) >= 12:
                    raise AssertionError("too many frames without executionFinished")
            joined = "\n".join(frames)
            for expected in (
                "executionStarted",
                "nodeExecuteBefore",
                "nodeExecuteAfter",
                "executionFinished",
            ):
                if expected not in joined:
                    raise AssertionError(f"missing n8n frame: {expected}")
            print("N8N_FRAME_COUNT", len(frames))
            print("N8N_WS_APP_FRAME_PASS")
    finally:
        delete_workflow(workflow_id)


async def main() -> None:
    await check_home_assistant()
    await check_n8n()


asyncio.run(main())
PY

echo "RESULT=PASS WS_REAL_APPS ha+n8n"
