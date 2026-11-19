#!/usr/bin/env bash
# NG-20 real E2E: real exyonq binary, real TCP/UDP sockets, real Redis.
#
# Native Linux is the canonical matrix. A macOS run is real macOS evidence and
# must not be reported as Linux. MySQL and QUIC are optional external services;
# when absent this script reports them as NOT EXECUTED rather than simulating
# them.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/e2e/lib.sh"

if [[ "${1:-}" == "--selftest" ]]; then
    python3 - <<'PY'
payload = b"ping"
assert payload[:1] == b"p"
assert b"A" + payload == b"Aping"
print("SELFTEST_OK NG20 stream payload framing")
PY
    echo "RESULT=PASS NG20_E2E_SELFTEST"
    exit 0
fi

if [[ "${NG20_SKIP_BUILD:-0}" != "1" ]]; then
    cargo build -q -p exyonq --bin exyonq --locked
fi
e2e_ensure_bins

WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-ng20-e2e.XXXXXX")"
SERVER_PID=""
REDIS_PID=""
BACKEND_PID=""

cleanup() {
    for pid in "$SERVER_PID" "$REDIS_PID" "$BACKEND_PID"; do
        [[ -n "$pid" ]] || continue
        kill "$pid" 2>/dev/null || true
    done
    for pid in "$SERVER_PID" "$REDIS_PID" "$BACKEND_PID"; do
        [[ -n "$pid" ]] || continue
        wait "$pid" 2>/dev/null || true
    done
    rm -rf "$WORK"
}
trap cleanup EXIT

USED_PORTS_FILE="$WORK/used-ports"
: >"$USED_PORTS_FILE"
pick_unique_port() {
    local port
    for _ in $(seq 1 100); do
        port="$(e2e_pick_port)"
        if grep -qxF "$port" "$USED_PORTS_FILE" 2>/dev/null; then
            continue
        fi
        printf '%s\n' "$port" >>"$USED_PORTS_FILE"
        printf '%s\n' "$port"
        return 0
    done
    echo "FAIL: could not reserve a unique local port" >&2
    return 1
}

# Real TCP/UDP echo backends. The helper binds all four sockets before it
# reports their ports, so the config uses the actual bound ports.
python3 - "$WORK" >"$WORK/backends.log" 2>&1 <<'PY' &
import json
import socket
import sys
import threading
import time

work = sys.argv[1]

def tcp_server(label):
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("127.0.0.1", 0))
    listener.listen()

    def handle(client):
        with client:
            while True:
                data = client.recv(4096)
                if not data:
                    break
                client.sendall(bytes([label]) + data)

    def accept_loop():
        while True:
            try:
                client, _ = listener.accept()
            except OSError:
                return
            threading.Thread(target=handle, args=(client,), daemon=True).start()

    threading.Thread(target=accept_loop, daemon=True).start()
    return listener.getsockname()[1]

def udp_server(label):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.bind(("127.0.0.1", 0))

    def loop():
        while True:
            try:
                data, peer = sock.recvfrom(2048)
            except OSError:
                return
            sock.sendto(bytes([label]) + data, peer)

    threading.Thread(target=loop, daemon=True).start()
    return sock.getsockname()[1]

ports = {
    "tcp": [tcp_server(ord("A")), tcp_server(ord("B"))],
    "udp": [udp_server(ord("A")), udp_server(ord("B"))],
}
with open(work + "/backends.json", "w", encoding="utf-8") as fh:
    json.dump(ports, fh)
while True:
    time.sleep(60)
PY
BACKEND_PID=$!
for _ in $(seq 1 100); do
    [[ -s "$WORK/backends.json" ]] && break
    kill -0 "$BACKEND_PID" 2>/dev/null || {
        echo "FAIL: backend helper exited" >&2
        cat "$WORK/backends.log" >&2
        exit 1
    }
    sleep 0.05
done
[[ -s "$WORK/backends.json" ]] || {
    echo "FAIL: backend helper did not publish ports" >&2
    cat "$WORK/backends.log" >&2
    exit 1
}

read -r TCP_A TCP_B UDP_A UDP_B < <(
    python3 - "$WORK/backends.json" <<'PY'
import json
import sys
ports = json.load(open(sys.argv[1], encoding="utf-8"))
print(ports["tcp"][0], ports["tcp"][1], ports["udp"][0], ports["udp"][1])
PY
)
printf '%s\n' "$TCP_A" "$TCP_B" "$UDP_A" "$UDP_B" >>"$USED_PORTS_FILE"

HTTP_PORT="$(pick_unique_port)"

REDIS_PORT=""
REDIS_STATE="NOT_EXECUTED"
if command -v redis-server >/dev/null 2>&1 && command -v redis-cli >/dev/null 2>&1; then
    REDIS_PORT="$(pick_unique_port)"
    redis-server \
        --bind 127.0.0.1 \
        --port "$REDIS_PORT" \
        --save '' \
        --appendonly no \
        --protected-mode no \
        >"$WORK/redis.log" 2>&1 &
    REDIS_PID=$!
    REDIS_READY=0
    for _ in $(seq 1 100); do
        if redis-cli -h 127.0.0.1 -p "$REDIS_PORT" ping 2>/dev/null | grep -q '^PONG$'; then
            REDIS_READY=1
            break
        fi
        kill -0 "$REDIS_PID" 2>/dev/null || break
        sleep 0.05
    done
    if [[ "$REDIS_READY" -eq 1 ]]; then
        REDIS_STATE="READY"
    else
        echo "FAIL: redis-server did not become ready" >&2
        cat "$WORK/redis.log" >&2
        exit 1
    fi
else
    echo "NOT_EXECUTED: redis-server/redis-cli not installed"
fi

TCP_PORT="$(pick_unique_port)"
UDP_PORT="$(pick_unique_port)"
REDIS_L4_PORT=""
if [[ -n "$REDIS_PORT" ]]; then
    REDIS_L4_PORT="$(pick_unique_port)"
fi

CFG="$WORK/exyonq.toml"
{
    cat <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${HTTP_PORT}"
routes = []

[[upstream]]
name = "tcp-echo"
timeout_ms = 2000

[[upstream.endpoints]]
id = "tcp-a"
address = "127.0.0.1"
port = ${TCP_A}

[[upstream.endpoints]]
id = "tcp-b"
address = "127.0.0.1"
port = ${TCP_B}

[[upstream]]
name = "udp-echo"
timeout_ms = 2000

[[upstream.endpoints]]
id = "udp-a"
address = "127.0.0.1"
port = ${UDP_A}

[[upstream.endpoints]]
id = "udp-b"
address = "127.0.0.1"
port = ${UDP_B}

[[stream]]
name = "tcp-balanced"
listen = "127.0.0.1:${TCP_PORT}"
protocol = "tcp"
upstream = "tcp-echo"
idle_timeout_ms = 5000

[[stream]]
name = "udp-balanced"
listen = "127.0.0.1:${UDP_PORT}"
protocol = "udp"
upstream = "udp-echo"
idle_timeout_ms = 5000
max_sessions = 16
EOF
    if [[ -n "$REDIS_PORT" ]]; then
        cat <<EOF

[[upstream]]
name = "redis-target"
timeout_ms = 2000
target = "http://127.0.0.1:${REDIS_PORT}"

[[stream]]
name = "redis-l4"
listen = "127.0.0.1:${REDIS_L4_PORT}"
protocol = "tcp"
upstream = "redis-target"
idle_timeout_ms = 5000
EOF
    fi
} >"$CFG"

"$EXYONQ_BIN" validate -c "$CFG" >"$WORK/validate.log" 2>&1
echo "OBSERVE validate=OK config=$CFG"

RUST_LOG=info "$EXYONQ_BIN" serve -c "$CFG" >"$WORK/exyonq.log" 2>&1 &
SERVER_PID=$!

READY=0
for _ in $(seq 1 120); do
    code="$(curl --noproxy '*' --max-time 1 -sS -o /dev/null -w '%{http_code}' \
        "http://127.0.0.1:${HTTP_PORT}/" 2>/dev/null || true)"
    if [[ "$code" =~ ^[1-5][0-9][0-9]$ ]]; then
        READY=1
        break
    fi
    kill -0 "$SERVER_PID" 2>/dev/null || {
        echo "FAIL: exyonq exited before readiness" >&2
        cat "$WORK/exyonq.log" >&2
        exit 1
    }
    sleep 0.05
done
[[ "$READY" -eq 1 ]] || {
    echo "FAIL: exyonq HTTP listener did not become ready" >&2
    cat "$WORK/exyonq.log" >&2
    exit 1
}
grep -q "L4 TCP stream listening" "$WORK/exyonq.log" || {
    echo "FAIL: TCP stream listener was not started" >&2
    cat "$WORK/exyonq.log" >&2
    exit 1
}
grep -q "L4 UDP stream listening" "$WORK/exyonq.log" || {
    echo "FAIL: UDP stream listener was not started" >&2
    cat "$WORK/exyonq.log" >&2
    exit 1
}
echo "OBSERVE server=pid=$SERVER_PID http=$HTTP_PORT tcp=$TCP_PORT udp=$UDP_PORT"

python3 - "$TCP_PORT" <<'PY'
import socket
import sys

port = int(sys.argv[1])
replies = []
for _ in range(4):
    with socket.create_connection(("127.0.0.1", port), timeout=4.0) as sock:
        sock.sendall(b"ping")
        data = b""
        while len(data) < 5:
            chunk = sock.recv(5 - len(data))
            if not chunk:
                break
            data += chunk
    assert data in (b"Aping", b"Bping"), data
    replies.append(data)
assert replies.count(b"Aping") == 2, replies
assert replies.count(b"Bping") == 2, replies
print("OBSERVE tcp_balanced replies=" + ",".join(x.decode() for x in replies))
PY

python3 - "$UDP_PORT" <<'PY'
import socket
import sys

port = int(sys.argv[1])

def one(source):
    source.settimeout(4.0)
    source.sendto(b"udp", ("127.0.0.1", port))
    data, _ = source.recvfrom(64)
    return data

first = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
first.bind(("127.0.0.1", 0))
first_1 = one(first)
first_2 = one(first)
assert first_1 == first_2 == b"Audp", (first_1, first_2)

second = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
second.bind(("127.0.0.1", 0))
second_1 = one(second)
assert second_1 == b"Budp", second_1
print(f"OBSERVE udp_affinity first={first_1.decode()} first_again={first_2.decode()} second={second_1.decode()}")
PY

if [[ "$REDIS_STATE" == "READY" ]]; then
    pong="$(redis-cli -h 127.0.0.1 -p "$REDIS_L4_PORT" ping)"
    [[ "$pong" == "PONG" ]] || {
        echo "FAIL: Redis through L4 did not answer PONG: $pong" >&2
        exit 1
    }
    redis-cli -h 127.0.0.1 -p "$REDIS_L4_PORT" set exyonq:ng20 ok >/dev/null
    value="$(redis-cli -h 127.0.0.1 -p "$REDIS_L4_PORT" get exyonq:ng20)"
    [[ "$value" == "ok" ]] || {
        echo "FAIL: Redis SET/GET through L4 returned $value" >&2
        exit 1
    }
    echo "OBSERVE redis_l4=PONG set_get=ok"
else
    echo "NOT_EXECUTED: Redis L4 acceptance (redis-server unavailable)"
fi

# MySQL and QUIC have dedicated real harnesses; this script covers TCP/UDP
# balancing and Redis only.
echo "SEPARATE_HARNESS: MySQL L4 acceptance -> scripts/e2e/ng20-mysql-e2e.sh"
echo "SEPARATE_HARNESS: QUIC-UDP L4 acceptance -> core/tests/ng20_quic_relay_test.rs"

echo "RESULT=PASS NG20_STREAM_E2E"
