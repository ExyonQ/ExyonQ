#!/usr/bin/env bash
# AP P1 / F6 memory-concurrency: real Linux server, real /proc RSS, real /metrics.
#
# The server is started with EXYONQ_SYNC_ACCEPT=1 so the configured accept
# workers are real OS threads, not a substituted counter. Linux exposes one
# resident set per process: every worker series must therefore carry the same
# VmRSS and scope="shared", while the series count must equal the configured
# worker count. No fixture supplies either value.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/e2e/lib.sh"
e2e_require_linux
e2e_ensure_bins

WORKERS="${WORKER_MEMORY_WORKERS:-4}"
[[ "$WORKERS" =~ ^[1-9][0-9]*$ ]] || {
    echo "WORKER_MEMORY_WORKERS must be a positive integer" >&2
    exit 2
}
[[ "$WORKERS" -le 16 ]] || {
    echo "WORKER_MEMORY_WORKERS must be <= 16 (server clamp)" >&2
    exit 2
}

WORK="$(mktemp -d /tmp/exyonq-worker-memory.XXXXXX)"
PORT="$(e2e_pick_port)"
DOCROOT="$WORK/public"
mkdir -p "$DOCROOT"
printf 'worker-memory-body' >"$DOCROOT/small.txt"
python3 -c "open('$DOCROOT/load.bin','wb').write(bytes([0x41])*(64*1024))"
EXY_PID=""
cleanup() {
    if [[ -n "$EXY_PID" ]]; then
        kill "$EXY_PID" 2>/dev/null || true
        for _ in $(seq 1 20); do
            kill -0 "$EXY_PID" 2>/dev/null || break
            sleep 0.05
        done
        kill -9 "$EXY_PID" 2>/dev/null || true
        wait "$EXY_PID" 2>/dev/null || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

CFG="$WORK/exyonq.toml"
cat >"$CFG" <<EOF
config_version = 1

[modules.metrics]
enabled = true
path = "/metrics"
health_path = "/exyonq-metrics-health"

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["assets"]

[[route]]
name = "assets"
match = { path = "/assets" }
root = "$DOCROOT"
index = "index.html"
EOF

"$EXYONQ_BIN" validate -c "$CFG"
RUST_LOG=error \
    EXYONQ_ACCEPT_WORKERS="$WORKERS" \
    EXYONQ_SYNC_ACCEPT=1 \
    EXYONQ_EPOLL_STATIC=0 \
    EXYONQ_CONTROL_SOCKET="$WORK/control.sock" \
    "$EXYONQ_BIN" serve -c "$CFG" >"$WORK/exyonq.log" 2>&1 &
EXY_PID=$!

ready=0
for _ in $(seq 1 200); do
    if curl --max-time 1 -fsS -o /dev/null "http://127.0.0.1:$PORT/assets/small.txt" 2>/dev/null; then
        ready=1
        break
    fi
    kill -0 "$EXY_PID" 2>/dev/null || {
        echo "server exited before readiness" >&2
        cat "$WORK/exyonq.log" >&2
        exit 1
    }
    sleep 0.05
done
[[ "$ready" -eq 1 ]] || {
    echo "server did not become ready" >&2
    cat "$WORK/exyonq.log" >&2
    exit 1
}

process_rss_kib() {
    awk '/^VmRSS:/ {print $2}' "/proc/$EXY_PID/status"
}

scrape() {
    local name=$1
    curl --max-time 10 -fsS "http://127.0.0.1:$PORT/metrics" >"$WORK/metrics-$name.txt"
    [[ -s "$WORK/metrics-$name.txt" ]] || {
        echo "empty /metrics response for $name" >&2
        exit 1
    }
}

assert_series() {
    local name="$1"
    local metrics="$WORK/metrics-$name.txt"
    local rss_before rss_after series count pool_value metric_value delta
    rss_before="$(process_rss_kib)"
    scrape "$name"
    rss_after="$(process_rss_kib)"

    series="$(grep -c '^exyonq_worker_resident_memory_bytes{' "$metrics" || true)"
    [[ "$series" -eq "$WORKERS" ]] || {
        echo "FAIL $name: worker series=$series expected=$WORKERS" >&2
        grep '^exyonq_worker_' "$metrics" >&2 || true
        exit 1
    }

    count="$(sed -n 's/^exyonq_worker_count{pool="accept"} //p' "$metrics")"
    [[ "$count" == "$WORKERS" ]] || {
        echo "FAIL $name: worker count=$count expected=$WORKERS" >&2
        exit 1
    }

    grep -q "^exyonq_worker_pool_resident_memory_bytes{pool=\"accept\",workers=\"$WORKERS\"} [0-9][0-9]*$" \
        "$metrics" || {
        echo "FAIL $name: missing bounded pool aggregate for workers=$WORKERS" >&2
        exit 1
    }
    pool_value="$(sed -n 's/^exyonq_worker_pool_resident_memory_bytes{[^}]*} //p' "$metrics")"
    metric_value="$(sed -n 's/^exyonq_worker_resident_memory_bytes{[^}]*} //p' "$metrics" | head -1)"

    [[ "$(grep '^exyonq_worker_resident_memory_bytes{' "$metrics" | awk '{print $NF}' | sort -u | wc -l)" -eq 1 ]] || {
        echo "FAIL $name: per-worker RSS values disagree despite one shared process" >&2
        exit 1
    }
    [[ "$(grep -c '^exyonq_worker_resident_memory_bytes{.*scope="shared"}' "$metrics")" -eq "$WORKERS" ]] || {
        echo "FAIL $name: worker series do not all declare scope=shared" >&2
        exit 1
    }
    [[ "$pool_value" == "$metric_value" ]] || {
        echo "FAIL $name: pool=$pool_value per-worker=$metric_value" >&2
        exit 1
    }

    delta=$((metric_value - rss_after * 1024))
    [[ "$delta" -ge 0 ]] || delta=$((-delta))
    # /metrics formatting allocates after the appender reads VmRSS. A small
    # bounded delta is expected; the value must remain the same process image,
    # not a fabricated per-thread number.
    [[ "$delta" -le $((8 * 1024 * 1024)) ]] || {
        echo "FAIL $name: metric=$metric_value os_rss_after=$((rss_after * 1024))" >&2
        exit 1
    }
    echo "OBSERVE $name workers=$WORKERS series=$series count=$count pool=$pool_value metric=$metric_value os_rss_kib_before=$rss_before os_rss_kib_after=$rss_after delta_bytes=$delta"
    grep -E '^exyonq_worker_(count|pool_resident_memory_bytes|resident_memory_bytes)' "$metrics" | head -"$((WORKERS + 2))"
}

# Real threads in the process; this is an additional observation, not the
# source of the metric's worker count.
THREADS="$(find "/proc/$EXY_PID/task" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')"
[[ "$THREADS" -ge "$WORKERS" ]] || {
    echo "FAIL process has $THREADS threads for $WORKERS workers" >&2
    exit 1
}
echo "OBSERVE process_pid=$EXY_PID task_threads=$THREADS"

assert_series idle

# Real concurrent requests; all must complete with the static body.
seq 1 512 | xargs -P 32 -n 1 \
    bash -c 'curl --max-time 10 -fsS -o /dev/null "$1"' _ \
    "http://127.0.0.1:$PORT/assets/small.txt"
assert_series under_load

grep -q "^exyonq_worker_count{pool=\"accept\"} $WORKERS$" "$WORK/metrics-under_load.txt"
if grep -q "$WORK" "$WORK/metrics-idle.txt" "$WORK/metrics-under_load.txt"; then
    echo "FAIL metrics leaked a local path" >&2
    exit 1
fi

BIN_SHA256="$(sha256sum "$EXYONQ_BIN" | awk '{print $1}')"
LOG_SHA256="$(sha256sum "$WORK/exyonq.log" | awk '{print $1}')"
echo "binary_sha256=$BIN_SHA256 server_log_sha256=$LOG_SHA256"
echo "RESULT=PASS WORKER_MEMORY workers=$WORKERS idle+concurrent_rss_observed"
