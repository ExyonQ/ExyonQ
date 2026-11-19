#!/usr/bin/env bash
# NG-20 real MySQL acceptance over the L4 TCP relay.
#
# Starts two real MySQL 8.4 servers in containers, relays TCP through a real
# `exyonq` process configured with `[[stream]] protocol = "tcp"`, and drives a
# real `mysql` client container through the relay. The protocol handshake and
# `SELECT 1` happen inside the MySQL client, not in a fixture.
#
# Balancing is observable through `@@server_id` (one value per server).
# Stopping one server afterwards exercises connect failover: the relay must
# retry the next selected endpoint and still complete a real session.
#
# Docker with a reachable daemon is required. Without it the script reports
# NOT_EXECUTED and exits 2; it never simulates a database.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/e2e/lib.sh"

IMAGE="${NG20_MYSQL_IMAGE:-mysql:8.4}"
SUFFIX="$$"
NAME_A="exyonq-ng20-mysql-a-${SUFFIX}"
NAME_B="exyonq-ng20-mysql-b-${SUFFIX}"
SERVER_ID_A=4201
SERVER_ID_B=4202

not_executed() {
    echo "RESULT=NOT_EXECUTED NG20_MYSQL_E2E reason=$1"
    exit 2
}

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

if [[ "${NG20_SKIP_BUILD:-0}" != "1" ]]; then
    cargo build -q -p exyonq --bin exyonq --locked
fi
e2e_ensure_bins

WORK="$(mktemp -d "${TMPDIR:-/tmp}/exyonq-ng20-mysql.XXXXXX")"
SERVER_PID=""

cleanup() {
    if [[ -n "$SERVER_PID" ]]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
    docker rm -f "$NAME_A" "$NAME_B" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT

command -v docker >/dev/null 2>&1 || not_executed "docker_not_installed"
docker info >/dev/null 2>&1 || not_executed "docker_daemon_unavailable"

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    if ! docker pull "$IMAGE" >"$WORK/pull.log" 2>&1; then
        tail -n 20 "$WORK/pull.log" >&2 || true
        not_executed "mysql_image_pull_failed"
    fi
fi
IMAGE_ID="$(docker image inspect --format '{{.Id}}' "$IMAGE")"
echo "OBSERVE mysql_image=$IMAGE image_id=$IMAGE_ID docker=$(docker version --format '{{.Server.Version}}')"

# Start one server and wait until a real client can run SELECT 1 on it. The
# published host port is random, so the relay config uses the actual port.
start_mysql() {
    local name="$1" server_id="$2" host_port=""
    docker run -d --rm \
        --name "$name" \
        -p 127.0.0.1::3306 \
        -e MYSQL_ALLOW_EMPTY_PASSWORD=yes \
        -e MYSQL_ROOT_HOST=% \
        "$IMAGE" \
        --server-id="$server_id" \
        >"$WORK/${name}.id" 2>"$WORK/${name}.err" || {
        cat "$WORK/${name}.err" >&2
        return 1
    }
    for _ in $(seq 1 240); do
        if ! docker inspect --format '{{.State.Running}}' "$name" 2>/dev/null | grep -q true; then
            docker logs "$name" >&2 || true
            return 1
        fi
        host_port="$(docker port "$name" 3306/tcp 2>/dev/null | head -n1 | sed 's/.*://')"
        if [[ -n "$host_port" ]] && docker exec "$name" \
            mysql -h 127.0.0.1 -u root --protocol=TCP -N -B -e 'SELECT 1' >/dev/null 2>&1; then
            printf '%s\n' "$host_port"
            return 0
        fi
        sleep 1
    done
    docker logs "$name" >&2 || true
    return 1
}

MYSQL_A_PORT="$(start_mysql "$NAME_A" "$SERVER_ID_A")" \
    || not_executed "mysql_a_start_failed"
echo "OBSERVE mysql_a name=$NAME_A host_port=$MYSQL_A_PORT server_id=$SERVER_ID_A"
MYSQL_B_PORT="$(start_mysql "$NAME_B" "$SERVER_ID_B")" \
    || not_executed "mysql_b_start_failed"
echo "OBSERVE mysql_b name=$NAME_B host_port=$MYSQL_B_PORT server_id=$SERVER_ID_B"

HTTP_PORT="$(e2e_pick_port)"
RELAY_PORT="$(e2e_pick_port)"
CFG="$WORK/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${HTTP_PORT}"
routes = []

[[upstream]]
name = "mysql"
timeout_ms = 5000

[[upstream.endpoints]]
id = "mysql-a"
address = "127.0.0.1"
port = ${MYSQL_A_PORT}

[[upstream.endpoints]]
id = "mysql-b"
address = "127.0.0.1"
port = ${MYSQL_B_PORT}

[[stream]]
name = "mysql-l4"
# 0.0.0.0 so the client container reaches the relay through the Docker host.
listen = "0.0.0.0:${RELAY_PORT}"
protocol = "tcp"
upstream = "mysql"
idle_timeout_ms = 30000
EOF

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
        cat "$WORK/exyonq.log" >&2
        fail "exyonq exited before readiness"
    }
    sleep 0.05
done
[[ "$READY" -eq 1 ]] || {
    cat "$WORK/exyonq.log" >&2
    fail "exyonq HTTP listener did not become ready"
}
grep -q "L4 TCP stream listening" "$WORK/exyonq.log" || {
    cat "$WORK/exyonq.log" >&2
    fail "TCP stream listener was not started"
}
echo "OBSERVE server=pid=$SERVER_PID http=$HTTP_PORT relay=$RELAY_PORT"

# Real mysql client (same image) talking to the relay through the Docker host.
# Linux containers cannot rely on host.docker.internal being routable through
# the bridge on every Docker setup; host networking reaches the relay bound on
# 0.0.0.0 directly. Docker Desktop on macOS still needs host.docker.internal.
if [[ "$(uname -s)" == "Linux" ]]; then
    MYSQL_CLIENT_NET=(--network host)
    MYSQL_CLIENT_HOST=127.0.0.1
else
    MYSQL_CLIENT_NET=(--add-host=host.docker.internal:host-gateway)
    MYSQL_CLIENT_HOST=host.docker.internal
fi
mysql_client() {
    docker run --rm "${MYSQL_CLIENT_NET[@]}" \
        --entrypoint mysql "$IMAGE" \
        -h "$MYSQL_CLIENT_HOST" -P "$RELAY_PORT" -u root --protocol=TCP \
        --connect-timeout=10 -N -B -e "$1"
}

SELECT_ONE="$(mysql_client 'SELECT 1')" || fail "mysql client through relay failed"
[[ "$SELECT_ONE" == "1" ]] || fail "SELECT 1 through relay returned '$SELECT_ONE'"
echo "OBSERVE mysql_select1=$SELECT_ONE"

SERVER_ID_1="$(mysql_client 'SELECT @@server_id')" || fail "server_id probe 1 failed"
SERVER_ID_2="$(mysql_client 'SELECT @@server_id')" || fail "server_id probe 2 failed"
[[ "$SERVER_ID_1" != "$SERVER_ID_2" ]] || fail "two connections hit the same server ($SERVER_ID_1)"
SORTED="$(printf '%s\n%s\n' "$SERVER_ID_1" "$SERVER_ID_2" | sort -n | tr '\n' ',')"
[[ "$SORTED" == "${SERVER_ID_A},${SERVER_ID_B}," ]] \
    || fail "balanced sessions did not reach both servers: $SERVER_ID_1,$SERVER_ID_2"
echo "OBSERVE mysql_balance server_ids=${SERVER_ID_1},${SERVER_ID_2}"

# One session, one handshake, several statements including a version probe.
SESSION="$(mysql_client 'SELECT 1; SELECT @@server_id; SELECT VERSION();')" \
    || fail "multi-statement session failed"
[[ "$(printf '%s\n' "$SESSION" | wc -l | tr -d ' ')" == "3" ]] \
    || fail "unexpected multi-statement output: $SESSION"
[[ "$(printf '%s\n' "$SESSION" | sed -n '1p')" == "1" ]] \
    || fail "SELECT 1 in session returned $(printf '%s\n' "$SESSION" | sed -n '1p')"
echo "OBSERVE mysql_session $(printf '%s\n' "$SESSION" | tr '\n' ' ')"

# Failover: with server A stopped, a new client must still complete a session.
docker stop "$NAME_A" >/dev/null
SERVER_ID_FAIL="$(mysql_client 'SELECT @@server_id')" || fail "failover session failed"
[[ "$SERVER_ID_FAIL" == "$SERVER_ID_B" ]] \
    || fail "failover selected $SERVER_ID_FAIL instead of $SERVER_ID_B"
echo "OBSERVE mysql_failover stopped=$NAME_A server_id=$SERVER_ID_FAIL"

echo "RESULT=PASS NG20_MYSQL_E2E"
