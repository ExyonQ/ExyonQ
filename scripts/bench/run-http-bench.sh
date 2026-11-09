#!/usr/bin/env bash
# Q-12: reproducible HTTP/1.1 benchmark on one host.
#
# Runs the selected servers natively against the same payload directory, with
# equal keep-alive settings, interleaved repetitions per server/payload,
# `oha` JSON metrics and RSS sampled from each server's process tree.
#
# Every run is fail-closed: a non-200 response, a success rate below 1.0, a
# transport error or a missing repetition aborts the run instead of producing
# a partial report. Raw evidence, report.json and report.md stay in OUT.
#
# OpenLiteSpeed is opt-in: Q12_SERVERS=...,ols plus Q12_OLS_URL and
# Q12_OLS_PID for an already-running OLS process on this host. A container,
# VM or remote endpoint is not comparable and is not accepted as evidence.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPS="${Q12_REPETITIONS:-5}"
DURATION="${Q12_DURATION:-10s}"
CONNECTIONS="${Q12_CONNECTIONS:-16}"
SERVERS_CSV="${Q12_SERVERS:-exyonq,nginx,apache}"
PAYLOADS_CSV="${Q12_PAYLOADS:-1k.bin,64k.bin}"
OUT="${Q12_OUT:-$ROOT/target/q12-bench-$(date -u +%Y%m%dT%H%M%SZ)}"
EXYONQ_BIN="${Q12_EXYONQ_BIN:-$ROOT/target/release/exyonq}"
NGINX_BIN="${Q12_NGINX_BIN:-$(command -v nginx || true)}"
HTTPD_BIN="${Q12_HTTPD_BIN:-$(command -v httpd || command -v apache2 || true)}"
OHA_BIN="${Q12_OHA_BIN:-$(command -v oha || true)}"
OLS_URL="${Q12_OLS_URL:-}"
OLS_PID="${Q12_OLS_PID:-}"
OLS_VERSION="${Q12_OLS_VERSION:-external-native-ols}"
MEASUREMENT_CLASS="${Q12_MEASUREMENT_CLASS:-local_native_non_canonical}"
PAYLOAD_DIR="$ROOT/benchmarks/scenarios/payloads/www"
AGG="$ROOT/scripts/bench/aggregate.py"
SAMPLER="$ROOT/scripts/bench/rss_sampler.py"

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

hash_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print $1}'
    else
        fail "no sha256sum or shasum available"
    fi
}

[[ "$REPS" =~ ^[0-9]+$ ]] || fail "Q12_REPETITIONS must be an integer"
[[ "$REPS" -ge 2 ]] || fail "Q12_REPETITIONS must be >= 2 to compute an interval"
[[ "$DURATION" =~ ^[0-9]+s$ ]] || fail "Q12_DURATION must look like 10s"
DURATION_SECS="${DURATION%s}"
[[ "$CONNECTIONS" =~ ^[0-9]+$ ]] || fail "Q12_CONNECTIONS must be an integer"
[[ "$CONNECTIONS" -ge 1 ]] || fail "Q12_CONNECTIONS must be >= 1"
[[ -n "$OHA_BIN" && -x "$OHA_BIN" ]] || fail "oha not found (set Q12_OHA_BIN)"
[[ -f "$AGG" && -f "$SAMPLER" ]] || fail "missing aggregate/sampler helper"
[[ -d "$PAYLOAD_DIR" ]] || fail "missing payload directory: $PAYLOAD_DIR"

IFS=',' read -r -a RAW_SERVERS <<<"$SERVERS_CSV"
SERVERS=()
for server in "${RAW_SERVERS[@]}"; do
    server="${server//[[:space:]]/}"
    [[ -n "$server" ]] && SERVERS+=("$server")
done
[[ "${#SERVERS[@]}" -gt 0 ]] || fail "no servers selected"
for i in "${!SERVERS[@]}"; do
    case "${SERVERS[$i]}" in
        exyonq|nginx|apache|ols) ;;
        *) fail "unknown server: ${SERVERS[$i]} (expected exyonq, nginx, apache or ols)" ;;
    esac
    for j in "${!SERVERS[@]}"; do
        if [[ "$i" -lt "$j" && "${SERVERS[$i]}" == "${SERVERS[$j]}" ]]; then
            fail "duplicate server selected: ${SERVERS[$i]}"
        fi
    done
done
SERVERS_CSV="$(IFS=,; echo "${SERVERS[*]}")"

server_selected() {
    local wanted="$1" server
    for server in "${SERVERS[@]}"; do
        [[ "$server" == "$wanted" ]] && return 0
    done
    return 1
}

if server_selected exyonq; then
    [[ -x "$EXYONQ_BIN" ]] || fail "missing ExyonQ binary: $EXYONQ_BIN"
fi
if server_selected nginx; then
    [[ -n "$NGINX_BIN" && -x "$NGINX_BIN" ]] || fail "nginx not found (set Q12_NGINX_BIN)"
fi
if server_selected apache; then
    [[ -n "$HTTPD_BIN" && -x "$HTTPD_BIN" ]] || fail "Apache not found (set Q12_HTTPD_BIN)"
fi
if server_selected ols; then
    [[ -n "$OLS_URL" ]] || fail "OLS selected but Q12_OLS_URL is unset"
    [[ "$OLS_URL" =~ ^http://(127\.0\.0\.1|localhost|\[::1\]):[0-9]+/?$ ]] \
        || fail "Q12_OLS_URL must be a loopback URL such as http://127.0.0.1:8088"
    [[ "$OLS_PID" =~ ^[0-9]+$ ]] || fail "OLS selected but Q12_OLS_PID is not a PID"
    kill -0 "$OLS_PID" 2>/dev/null || fail "OLS process $OLS_PID is not running"
fi

IFS=',' read -r -a PAYLOADS <<<"$PAYLOADS_CSV"
[[ "${#PAYLOADS[@]}" -gt 0 ]] || fail "no payloads selected"
for payload in "${PAYLOADS[@]}"; do
    [[ -f "$PAYLOAD_DIR/$payload" ]] || fail "missing payload: $PAYLOAD_DIR/$payload"
done

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"
# Apache on macOS cannot read files under /Volumes (TCC/EPERM), so configs and
# payloads live in a temporary directory on every host. Results stay in OUT.
WORK="${Q12_WORK_DIR:-$(mktemp -d "${TMPDIR:-/tmp}/q12-bench.XXXXXX")}"
mkdir -p "$WORK"
WORK="$(cd "$WORK" && pwd)"
mkdir -p "$OUT/raw" "$WORK/www" "$WORK/logs"
for payload in "${PAYLOADS[@]}"; do
    cp "$PAYLOAD_DIR/$payload" "$WORK/www/"
done

RAWS=()
SERVER_PIDS=()

cleanup() {
    local pid
    for pid in "${SERVER_PIDS[@]:-}"; do
        [[ -n "$pid" ]] && kill "$pid" 2>/dev/null || true
    done
    for pid in "${SERVER_PIDS[@]:-}"; do
        [[ -n "$pid" ]] && wait "$pid" 2>/dev/null || true
    done
    if [[ "${Q12_KEEP_WORK:-0}" != "1" ]]; then
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT

pick_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

USED_PORTS=()
pick_unique_port() {
    local port used
    while :; do
        port="$(pick_port)"
        used=0
        for existing in "${USED_PORTS[@]:-}"; do
            [[ "$port" == "$existing" ]] && used=1 && break
        done
        if [[ "$used" -eq 0 ]]; then
            USED_PORTS+=("$port")
            printf '%s\n' "$port"
            return 0
        fi
    done
}

PORT_EXYONQ="$(pick_unique_port)"
PORT_NGINX="$(pick_unique_port)"
PORT_HTTPD="$(pick_unique_port)"

url_for() {
    case "$1" in
        exyonq) printf 'http://127.0.0.1:%s/%s' "$PORT_EXYONQ" "$2" ;;
        nginx) printf 'http://127.0.0.1:%s/%s' "$PORT_NGINX" "$2" ;;
        apache) printf 'http://127.0.0.1:%s/%s' "$PORT_HTTPD" "$2" ;;
        ols) printf '%s/%s' "${OLS_URL%/}" "$2" ;;
        *) fail "unknown server: $1" ;;
    esac
}

pid_for() {
    case "$1" in
        exyonq) printf '%s' "$PID_EXYONQ" ;;
        nginx) printf '%s' "$PID_NGINX" ;;
        apache) printf '%s' "$PID_HTTPD" ;;
        ols) printf '%s' "$OLS_PID" ;;
        *) fail "unknown server: $1" ;;
    esac
}

wait_ready() {
    local name="$1" url="$2" pid="$3" payload="${4:-${PAYLOADS[0]}}" code expected actual
    expected="$(hash_file "$WORK/www/$payload")"
    for _ in $(seq 1 200); do
        code="$(curl -sS --max-time 2 -o "$WORK/ready-$name" -w '%{http_code}' "$url" 2>/dev/null || true)"
        if [[ "$code" == "200" ]]; then
            actual="$(hash_file "$WORK/ready-$name")"
            [[ "$actual" == "$expected" ]] || fail "$name served a different body"
            return 0
        fi
        kill -0 "$pid" 2>/dev/null || {
            tail -n 20 "$WORK/logs/$name.log" >&2 || true
            fail "$name exited before readiness"
        }
        sleep 0.05
    done
    fail "$name did not become ready"
}

verify_body() {
    local name="$1" url="$2" payload="$3" expected actual
    expected="$(hash_file "$WORK/www/$payload")"
    curl -sS --max-time 5 -o "$WORK/body" "$url"
    actual="$(hash_file "$WORK/body")"
    [[ "$actual" == "$expected" ]] || fail "$name/$payload body oracle mismatch"
}

start_exyonq() {
    local cfg="$WORK/exyonq.toml"
    cat >"$cfg" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${PORT_EXYONQ}"
routes = ["static"]

[[route]]
name = "static"
match = { path = "/" }
root = "${WORK}/www"

[logging.access]
enabled = false

[logging.otel]
enabled = false

[waf]
enabled = false
mode = "disabled"
EOF
    "$EXYONQ_BIN" validate -c "$cfg" >"$WORK/logs/exyonq-validate.log" 2>&1 \
        || { cat "$WORK/logs/exyonq-validate.log" >&2; fail "exyonq validate rejected the benchmark config"; }
    "$EXYONQ_BIN" serve -c "$cfg" >"$WORK/logs/exyonq.log" 2>&1 &
    PID_EXYONQ=$!
    SERVER_PIDS+=("$PID_EXYONQ")
    wait_ready exyonq "$(url_for exyonq "${PAYLOADS[0]}")" "$PID_EXYONQ"
}

start_nginx() {
    local cfg="$WORK/nginx.conf"
    mkdir -p "$WORK/nginx"
    cat >"$cfg" <<EOF
worker_processes auto;
error_log ${WORK}/logs/nginx-error.log warn;
pid ${WORK}/nginx/nginx.pid;
events { worker_connections 4096; }
http {
    access_log off;
    default_type application/octet-stream;
    sendfile on;
    keepalive_timeout 65;
    server {
        listen 127.0.0.1:${PORT_NGINX};
        root ${WORK}/www;
        location / { try_files \$uri =404; }
    }
}
EOF
    "$NGINX_BIN" -t -c "$cfg" -p "$WORK/nginx" >"$WORK/logs/nginx-test.log" 2>&1 \
        || { cat "$WORK/logs/nginx-test.log" >&2; fail "nginx rejected the benchmark config"; }
    "$NGINX_BIN" -c "$cfg" -p "$WORK/nginx" -g 'daemon off;' >"$WORK/logs/nginx.log" 2>&1 &
    PID_NGINX=$!
    SERVER_PIDS+=("$PID_NGINX")
    wait_ready nginx "$(url_for nginx "${PAYLOADS[0]}")" "$PID_NGINX"
}

httpd_modules_dir() {
    local dir
    if [[ -n "${Q12_HTTPD_MODULES_DIR:-}" ]]; then
        printf '%s\n' "$Q12_HTTPD_MODULES_DIR"
        return 0
    fi
    for dir in /usr/libexec/apache2 /usr/lib/apache2/modules \
        /usr/lib64/httpd/modules /usr/lib/httpd/modules; do
        if [[ -d "$dir" ]]; then
            printf '%s\n' "$dir"
            return 0
        fi
    done
    return 1
}

load_apache_module() {
    local name="$1" path="$2" required="${3:-optional}"
    if [[ -f "$path" ]]; then
        printf 'LoadModule %s "%s"\n' "$name" "$path"
    elif [[ "$required" == "required" ]]; then
        fail "Apache module $name not found at $path (set Q12_HTTPD_MODULES_DIR)"
    fi
}

start_apache() {
    local cfg="$WORK/httpd.conf" mod_dir
    mod_dir="$(httpd_modules_dir || true)"
    [[ -n "$mod_dir" ]] || fail "cannot locate Apache modules (set Q12_HTTPD_MODULES_DIR)"
    mkdir -p "$WORK/httpd-root" "$WORK/httpd-run" "$WORK/httpd-lock"
    {
        printf 'ServerRoot "%s"\n' "$WORK/httpd-root"
        printf 'DefaultRuntimeDir "%s"\n' "$WORK/httpd-run"
        printf 'Mutex file:%s default\n' "$WORK/httpd-lock"
        printf 'PidFile "%s"\n' "$WORK/httpd.pid"
        printf 'ErrorLog "%s"\n' "$WORK/logs/httpd-error.log"
        printf 'LogLevel warn\n'
        load_apache_module mpm_event_module "$mod_dir/mod_mpm_event.so" required
        load_apache_module authz_core_module "$mod_dir/mod_authz_core.so" required
        load_apache_module unixd_module "$mod_dir/mod_unixd.so" optional
        printf 'ServerName localhost\n'
        printf 'Listen 127.0.0.1:%s\n' "$PORT_HTTPD"
        printf 'User "%s"\n' "$(id -un)"
        printf 'Group "%s"\n' "$(id -gn)"
        printf 'DocumentRoot "%s"\n' "$WORK/www"
        printf '<Directory "%s">\n' "$WORK/www"
        printf '    Require all granted\n'
        printf '</Directory>\n'
        printf 'EnableSendfile On\n'
        printf 'KeepAlive On\n'
        printf 'MaxKeepAliveRequests 0\n'
        printf 'KeepAliveTimeout 65\n'
    } >"$cfg"
    "$HTTPD_BIN" -t -f "$cfg" >"$WORK/logs/httpd-test.log" 2>&1 \
        || { cat "$WORK/logs/httpd-test.log" >&2; fail "httpd rejected the benchmark config"; }
    "$HTTPD_BIN" -f "$cfg" -DFOREGROUND >"$WORK/logs/httpd.log" 2>&1 &
    PID_HTTPD=$!
    SERVER_PIDS+=("$PID_HTTPD")
    wait_ready apache "$(url_for apache "${PAYLOADS[0]}")" "$PID_HTTPD"
}

start_ols() {
    # OLS is deliberately not launched by this harness: a native OLS install
    # owns its own config and lifecycle. The caller starts it on this host and
    # supplies its loopback URL and PID.
    kill -0 "$OLS_PID" 2>/dev/null || fail "OLS process $OLS_PID is not running"
}

start_server() {
    case "$1" in
        exyonq) start_exyonq ;;
        nginx) start_nginx ;;
        apache) start_apache ;;
        ols) start_ols ;;
        *) fail "unknown server: $1" ;;
    esac
}

run_cell() {
    local payload="$1" server="$2" rep="$3" base rss rc
    base="$OUT/raw/${payload}__${server}__r${rep}"
    python3 "$SAMPLER" --pid "$(pid_for "$server")" --duration "$((DURATION_SECS + 1))" \
        --out "${base}.rss.json" &
    rss=$!
    rc=0
    "$OHA_BIN" --no-tui --output-format json -c "$CONNECTIONS" -t 5s -w \
        -z "$DURATION" "$(url_for "$server" "$payload")" >"${base}.json" 2>"${base}.err" || rc=$?
    wait "$rss" || {
        echo "RSS sampler failed for ${payload}/${server}/r${rep}" >&2
        return 1
    }
    [[ "$rc" -eq 0 ]] || {
        cat "${base}.err" >&2 || true
        fail "oha failed for ${payload}/${server}/r${rep} (rc=$rc)"
    }
    python3 - "${base}.json" <<'PY' || fail "invalid oha evidence for ${payload}/${server}/r${rep}"
import json
import sys

data = json.load(open(sys.argv[1], encoding="utf-8"))
summary = data["summary"]
if summary["successRate"] != 1.0:
    raise SystemExit("successRate != 1.0")
if summary["requestsPerSec"] <= 0:
    raise SystemExit("requestsPerSec <= 0")
if not data.get("statusCodeDistribution") or any(k != "200" for k in data["statusCodeDistribution"]):
    raise SystemExit("non-200 response")
errors = data.get("errorDistribution") or {}
if any(int(v) != 0 for v in errors.values()):
    raise SystemExit("transport/application errors")
PY
    RAWS+=("$base")
}

echo "Q12_BENCH out=$OUT work=$WORK reps=$REPS duration=$DURATION connections=$CONNECTIONS servers=$SERVERS_CSV payloads=$PAYLOADS_CSV"
for server in "${SERVERS[@]}"; do
    start_server "$server"
    echo "OBSERVE server=$server pid=$(pid_for "$server")"
done

for payload in "${PAYLOADS[@]}"; do
    for server in "${SERVERS[@]}"; do
        verify_body "$server" "$(url_for "$server" "$payload")" "$payload"
        "$OHA_BIN" --no-tui --output-format json -c "$CONNECTIONS" -t 5s -w -z 2s \
            "$(url_for "$server" "$payload")" >"$WORK/warmup-${payload}-${server}.json" 2>"$WORK/warmup-${payload}-${server}.err" \
            || fail "warm-up failed for ${payload}/${server}"
    done
done
echo "OBSERVE warmup=OK body_oracle=OK"

for rep in $(seq 1 "$REPS"); do
    for payload in "${PAYLOADS[@]}"; do
        count="${#SERVERS[@]}"
        for k in $(seq 0 $((count - 1))); do
            idx=$(((rep + k) % count))
            run_cell "$payload" "${SERVERS[$idx]}" "$rep"
        done
    done
    echo "OBSERVE repetition=$rep/$REPS complete"
done

EXYONQ_VERSION=""
NGINX_VERSION=""
HTTPD_VERSION=""
if server_selected exyonq; then
    EXYONQ_VERSION="$("$EXYONQ_BIN" --version 2>&1 | head -n1)"
fi
if server_selected nginx; then
    NGINX_VERSION="$("$NGINX_BIN" -v 2>&1 | head -n1)"
fi
if server_selected apache; then
    HTTPD_VERSION="$("$HTTPD_BIN" -v 2>&1 | head -n1)"
fi
OHA_VERSION="$("$OHA_BIN" --version 2>&1 | head -n1)"
OLS_SOFTWARE=""
if server_selected ols; then
    OLS_SOFTWARE="$OLS_VERSION"
fi

Q12_REPETITIONS="$REPS" \
Q12_DURATION="$DURATION" \
Q12_CONNECTIONS="$CONNECTIONS" \
Q12_MEASUREMENT_CLASS="$MEASUREMENT_CLASS" \
Q12_SOFTWARE_EXYONQ="$EXYONQ_VERSION" \
Q12_SOFTWARE_NGINX="$NGINX_VERSION" \
Q12_SOFTWARE_HTTPD="$HTTPD_VERSION" \
Q12_SOFTWARE_OLS="$OLS_SOFTWARE" \
Q12_SOFTWARE_OHA="$OHA_VERSION" \
Q12_SERVERS="$SERVERS_CSV" \
Q12_PAYLOADS="$PAYLOADS_CSV" \
Q12_OLS_URL="$OLS_URL" \
python3 - "$ROOT" "$OUT/meta.json" "$EXYONQ_BIN" <<'PY'
import hashlib
import json
import os
import platform
import subprocess
import sys
from pathlib import Path

root = Path(sys.argv[1])
out = Path(sys.argv[2])
exyonq_path = Path(sys.argv[3])
servers = [item for item in os.environ.get("Q12_SERVERS", "").split(",") if item]


def run(*args):
    try:
        return subprocess.run(args, cwd=root, capture_output=True, text=True, check=False).stdout.strip()
    except OSError:
        return ""


def read_text(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except OSError:
        return ""


def sysctl(name):
    return run("sysctl", "-n", name)


def linux_cpu():
    for line in read_text("/proc/cpuinfo").splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return platform.processor() or "unknown"


def linux_memory():
    for line in read_text("/proc/meminfo").splitlines():
        if line.startswith("MemTotal:"):
            return int(line.split()[1]) * 1024
    return 0


def linux_os():
    for line in read_text("/etc/os-release").splitlines():
        if line.startswith("PRETTY_NAME="):
            return line.split("=", 1)[1].strip().strip('"')
    return f"Linux {platform.release()}"


def hardware():
    system = platform.system()
    if system == "Darwin":
        version = run("sw_vers", "-productVersion") or platform.mac_ver()[0] or "unknown"
        return {
            "cpu": sysctl("machdep.cpu.brand_string") or platform.processor() or "unknown",
            "logical_cpus": int(sysctl("hw.ncpu") or os.cpu_count() or 0),
            "memory_bytes": int(sysctl("hw.memsize") or 0),
            "os": f"macOS {version}",
            "arch": platform.machine(),
            "kernel": platform.release(),
        }
    if system == "Linux":
        return {
            "cpu": linux_cpu(),
            "logical_cpus": int(os.cpu_count() or 0),
            "memory_bytes": linux_memory(),
            "os": linux_os(),
            "arch": platform.machine(),
            "kernel": platform.release(),
        }
    return {
        "cpu": platform.processor() or "unknown",
        "logical_cpus": int(os.cpu_count() or 0),
        "memory_bytes": 0,
        "os": f"{system} {platform.release()}",
        "arch": platform.machine(),
        "kernel": platform.release(),
    }


software = {}
for key in ("exyonq", "nginx", "httpd", "ols", "oha"):
    value = os.environ.get(f"Q12_SOFTWARE_{key.upper()}", "").strip()
    if value:
        software[key] = value

meta = {
    "schema": 1,
    "started_at": run("date", "-u", "+%Y-%m-%dT%H:%M:%SZ"),
    "measurement_class": os.environ.get("Q12_MEASUREMENT_CLASS", "local_native_non_canonical"),
    "ci": os.environ.get("CI", "").lower() == "true",
    "repetitions": int(os.environ["Q12_REPETITIONS"]),
    "duration": os.environ["Q12_DURATION"],
    "connections": int(os.environ["Q12_CONNECTIONS"]),
    "payloads": [item for item in os.environ.get("Q12_PAYLOADS", "").split(",") if item],
    "servers": servers,
    "hardware": hardware(),
    "software": software,
    "exyonq_sha256": hashlib.sha256(exyonq_path.read_bytes()).hexdigest()
    if exyonq_path.is_file()
    else "",
    "git_head": run("git", "rev-parse", "HEAD"),
    "git_status_porcelain": run("git", "status", "--porcelain"),
}
if "ols" in servers:
    meta["ols_url"] = os.environ.get("Q12_OLS_URL", "")
out.write_text(json.dumps(meta, indent=2, sort_keys=True) + "\n", encoding="utf-8")
PY

mkdir -p "$OUT/logs"
cp -R "$WORK/logs/." "$OUT/logs/" 2>/dev/null || true
cp "$WORK"/*.toml "$WORK"/*.conf "$OUT/" 2>/dev/null || true

python3 "$AGG" --raw "$OUT/raw" --out "$OUT" --meta "$OUT/meta.json"

echo "OBSERVE report_json=$OUT/report.json report_md=$OUT/report.md"
cat "$OUT/report.md"
echo "RESULT=PASS Q12_HTTP_BENCH"
