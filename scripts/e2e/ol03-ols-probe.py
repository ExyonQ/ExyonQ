#!/usr/bin/env python3
"""Raw TCP probe for the OL-03 OpenLiteSpeed comparison.

The probe deliberately speaks HTTP/1.1 over raw sockets so a closed connection
cannot be confused with an HTTP error response.  It is used by
`ol03-ols-compare.sh`; it is not a substitute for the product E2E.
"""
import json
import select
import socket
import sys
import time

HOST = sys.argv[1]
PORT = int(sys.argv[2])
SCENARIO = sys.argv[3]
PATH = "/big256.bin"


def now():
    return time.monotonic()


def open_conn(label, t0, close):
    s = socket.socket()
    s.settimeout(3)
    s.connect((HOST, PORT))
    rec = {"label": label, "connected": round(now() - t0, 6), "events": []}
    conn = "close" if close else "keep-alive"
    s.sendall(
        f"GET {PATH} HTTP/1.1\r\nHost: {HOST}:{PORT}\r\nConnection: {conn}\r\n\r\n".encode()
    )
    rec["sent"] = round(now() - t0, 6)
    s.setblocking(False)
    return s, rec


def drain(s, rec, t0, data):
    while True:
        try:
            b = s.recv(65536)
        except BlockingIOError:
            break
        except OSError as exc:
            rec["events"].append(("error", round(now() - t0, 6), str(exc)))
            rec["closed"] = round(now() - t0, 6)
            return
        if not b:
            rec["events"].append(("eof", round(now() - t0, 6)))
            rec["closed"] = round(now() - t0, 6)
            return
        rec["events"].append(("data", round(now() - t0, 6), len(b)))
        data.append(b)


t0 = now()
socks = []
recs = []
data = {}
specs = {
    "single": [("A", 0.0, True)],
    "ab": [("A", 0.0, True), ("B", 0.35, True)],
    "abc": [("A", 0.0, True), ("B", 0.35, True), ("C", 0.7, True)],
    "soft": [("A", 0.0, True), ("B", 2.2, True)],
}[SCENARIO]
for label, delay, close in specs:
    while now() < t0 + delay:
        time.sleep(0.005)
    s, r = open_conn(label, t0, close)
    socks.append((s, r))
    recs.append(r)
    data[label] = []
    drain(s, r, t0, data[label])

limit = 12 if SCENARIO == "single" else 20
while now() < t0 + limit and any("closed" not in r for r in recs):
    live = [s for s, r in socks if "closed" not in r]
    if not live:
        break
    ready, _, _ = select.select(live, [], [], 0.05)
    for s in ready:
        r = next(rr for ss, rr in socks if ss is s)
        drain(s, r, t0, data[r["label"]])

for s, r in socks:
    if "closed" not in r:
        drain(s, r, t0, data[r["label"]])
    try:
        s.close()
    except OSError:
        pass

out = {
    "host": HOST,
    "port": PORT,
    "scenario": SCENARIO,
    "elapsed": round(now() - t0, 6),
    "records": recs,
}
for r in recs:
    raw = b"".join(data[r["label"]])
    r["byte_count"] = len(raw)
    r["data_events"] = sum(1 for e in r["events"] if e[0] == "data")
    r["first_data"] = next((e for e in r["events"] if e[0] == "data"), None)
    if b"\r\n\r\n" in raw:
        h, b = raw.split(b"\r\n\r\n", 1)
        r["header_bytes"] = len(h) + 4
        r["body_bytes"] = len(b)
        r["status_line"] = h.split(b"\r\n", 1)[0].decode("latin1", "replace")
print(json.dumps(out, indent=2))
