#!/usr/bin/env bash
# P1.1 correctness/stability soak runner. Run on a Linux benchmark host.
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  p1.1-correctness-stability-soak-remote.sh \
    --workspace PATH \
    --host-label amd64|arm64 \
    --expected-arch x86_64|aarch64 \
    --duration-sec SECONDS \
    --report-relpath docs/operations/p1.1-soak-*.md
EOF
}

WORKSPACE=""
HOST_LABEL=""
EXPECTED_ARCH=""
DURATION_SEC="${P1_1_SOAK_DURATION_SEC:-${P11_SOAK_DURATION_SEC:-900}}"
REPORT_RELPATH=""
RUN_ID="${P1_1_SOAK_RUN_ID:-${P11_SOAK_RUN_ID:-p1-1-soak-$(date -u +%Y%m%dT%H%M%SZ)}}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --host-label) HOST_LABEL="${2:-}"; shift 2 ;;
    --expected-arch) EXPECTED_ARCH="${2:-}"; shift 2 ;;
    --duration-sec) DURATION_SEC="${2:-}"; shift 2 ;;
    --report-relpath) REPORT_RELPATH="${2:-}"; shift 2 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ -n "$WORKSPACE" && -d "$WORKSPACE" && -n "$HOST_LABEL" && -n "$EXPECTED_ARCH" && -n "$REPORT_RELPATH" ]] || {
  usage >&2
  exit 2
}
[[ "$(uname -s)" == "Linux" ]] || { echo "ERROR: Linux host required" >&2; exit 2; }
ACTUAL_ARCH="$(uname -m)"
[[ "$ACTUAL_ARCH" == "$EXPECTED_ARCH" ]] || {
  echo "ERROR: arch mismatch expected=$EXPECTED_ARCH actual=$ACTUAL_ARCH" >&2
  exit 2
}

cd "$WORKSPACE"
export PATH="${HOME}/.cargo/bin:/usr/local/cargo/bin:${PATH}"

RESULT_DIR="$WORKSPACE/docs/operations/p1.1-soak-evidence/$RUN_ID/$HOST_LABEL"
REPORT_PATH="$WORKSPACE/$REPORT_RELPATH"
mkdir -p "$RESULT_DIR" "$(dirname "$REPORT_PATH")"
exec > >(tee "$RESULT_DIR/runner.log") 2>&1

echo "=== P1.1 correctness/stability soak ==="
echo "run_id=$RUN_ID host_label=$HOST_LABEL arch=$ACTUAL_ARCH duration_sec=$DURATION_SEC"
date -u

TMP="$(mktemp -d)"
DOCROOT="$TMP/public"
UPSTREAM_PID=""
EXYONQ_PID=""
SAMPLER_PID=""
LOADGEN_PID=""
LIFECYCLE_PID=""
PID_FILE="$TMP/exyonq.pid"
CTRL_SOCK="$TMP/control.sock"
CFG="$TMP/exyonq.toml"
SERVER_LOG="$RESULT_DIR/exyonq.log"
UPSTREAM_LOG="$RESULT_DIR/upstream.log"
RESOURCE_CSV="$RESULT_DIR/resources.csv"
LIFECYCLE_ENV="$RESULT_DIR/lifecycle.env"
LOADGEN_STATS="$RESULT_DIR/loadgen-stats.json"
STOP_FILE="$TMP/stop"

cleanup() {
  rm -f "$STOP_FILE"
  [[ -n "${LIFECYCLE_PID:-}" ]] && kill "$LIFECYCLE_PID" 2>/dev/null || true
  [[ -n "${LOADGEN_PID:-}" ]] && kill "$LOADGEN_PID" 2>/dev/null || true
  [[ -n "${SAMPLER_PID:-}" ]] && kill "$SAMPLER_PID" 2>/dev/null || true
  if [[ -n "${EXYONQ_PID:-}" ]] && kill -0 "$EXYONQ_PID" 2>/dev/null; then
    kill "$EXYONQ_PID" 2>/dev/null || true
    wait "$EXYONQ_PID" 2>/dev/null || true
  fi
  [[ -n "${UPSTREAM_PID:-}" ]] && kill "$UPSTREAM_PID" 2>/dev/null || true
  rm -rf "$TMP"
}
trap cleanup EXIT

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

write_fixture_tree() {
  mkdir -p "$DOCROOT/routes" "$DOCROOT/sub"
  python3 - "$DOCROOT" <<'PY'
import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
(root / "small.txt").write_bytes(b"x" * 1024)
(root / "large.bin").write_bytes((b"0123456789abcdef" * 65536))  # 1 MiB
(root / "sub" / "index.html").write_text("index-ok\n")
for i in range(1, 17):
    (root / "routes" / f"route{i:03d}.bin").write_bytes((f"route-{i:03d}\n").encode() * 128)
manifest = {}
for path in sorted(root.rglob("*")):
    if path.is_file():
        data = path.read_bytes()
        manifest[str(path.relative_to(root))] = {
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        }
(root / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY
}

write_config() {
  local listen_port="$1"
  local upstream_port="$2"
  cat >"$CFG" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${listen_port}"
routes = ["site", "api"]

[[route]]
name = "site"
match = { path = "/site" }
root = "${DOCROOT}"
index = "index.html"

[[route]]
name = "api"
match = { path = "/api" }
upstream = "backend"

[[upstream]]
name = "backend"
target = "http://127.0.0.1:${upstream_port}"
timeout_ms = 1000
EOF
}

start_upstream() {
  local port="$1"
  python3 - "$port" >"$UPSTREAM_LOG" 2>&1 <<'PY' &
import http.server
import json
import socketserver
import sys
import time

class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return
    def do_GET(self):
        body = json.dumps({"ok": True, "path": self.path, "ts": time.time()}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_HEAD(self):
        self.send_response(200)
        self.send_header("content-length", "0")
        self.end_headers()

with socketserver.ThreadingTCPServer(("127.0.0.1", int(sys.argv[1])), Handler) as srv:
    srv.daemon_threads = True
    srv.serve_forever()
PY
  UPSTREAM_PID=$!
}

wait_http_ready() {
  local port="$1"
  for _ in $(seq 1 100); do
    if curl -sf --max-time 2 "http://127.0.0.1:${port}/site/small.txt" >/dev/null 2>&1; then
      return 0
    fi
    if [[ -n "${EXYONQ_PID:-}" ]] && ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      echo "exyonq exited during startup" >&2
      cat "$SERVER_LOG" >&2 || true
      return 1
    fi
    sleep 0.1
  done
  echo "timeout waiting for exyonq listener" >&2
  cat "$SERVER_LOG" >&2 || true
  return 1
}

status_line() {
  "$EXYONQCTL_BIN" status --socket "$CTRL_SOCK" 2>&1 || true
}

start_exyonq() {
  rm -f "$CTRL_SOCK"
  EXYONQ_CONTROL_SOCKET="$CTRL_SOCK" RUST_LOG="${P1_1_SOAK_RUST_LOG:-${P11_SOAK_RUST_LOG:-error}}" \
    "$EXYONQ_BIN" serve --config "$CFG" >>"$SERVER_LOG" 2>&1 &
  EXYONQ_PID=$!
  echo "$EXYONQ_PID" >"$PID_FILE"
  wait_http_ready "$LISTEN_PORT"
}

wait_exyonq_exit() {
  local waited=0
  while [[ "$waited" -lt 20 ]]; do
    if [[ -z "${EXYONQ_PID:-}" ]] || ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      wait "$EXYONQ_PID" 2>/dev/null || true
      return 0
    fi
    sleep 1
    waited=$((waited + 1))
  done
  return 1
}

write_loadgen() {
  cat >"$TMP/loadgen.py" <<'PY'
import http.client
import json
import random
import socket
import sys
import threading
import time
from collections import defaultdict

host = sys.argv[1]
port = int(sys.argv[2])
duration = int(sys.argv[3])
out = sys.argv[4]
deadline = time.time() + duration
lock = threading.Lock()
stats = defaultdict(lambda: defaultdict(int))

def add(name, field, n=1):
    with lock:
        stats[name][field] += n

def request(name, path, min_bytes=0, timeout=2.0, keep=False):
    conn = http.client.HTTPConnection(host, port, timeout=timeout)
    try:
        conn.putrequest("GET", path)
        conn.putheader("Host", host)
        conn.putheader("Connection", "keep-alive" if keep else "close")
        conn.endheaders()
        resp = conn.getresponse()
        body = resp.read()
        add(name, "requests")
        if resp.status == 200 and len(body) >= min_bytes:
            add(name, "ok")
        else:
            add(name, "errors")
    except socket.timeout:
        add(name, "requests")
        add(name, "timeouts")
    except Exception:
        add(name, "requests")
        add(name, "errors")
    finally:
        conn.close()

def simple_worker(name, path, min_bytes, pause):
    while time.time() < deadline:
        request(name, path, min_bytes)
        time.sleep(pause)

def mixed_worker():
    paths = ["/site/small.txt", "/site/large.bin", "/api/health"]
    paths.extend(f"/site/routes/route{i:03d}.bin" for i in range(1, 17))
    while time.time() < deadline:
        path = random.choice(paths)
        request("mixed_routes", path, 1)
        time.sleep(0.01)

def keepalive_worker():
    while time.time() < deadline:
        conn = http.client.HTTPConnection(host, port, timeout=2.0)
        try:
            for _ in range(20):
                if time.time() >= deadline:
                    break
                conn.putrequest("GET", "/site/small.txt")
                conn.putheader("Host", host)
                conn.putheader("Connection", "keep-alive")
                conn.endheaders()
                resp = conn.getresponse()
                body = resp.read()
                add("http11_keepalive", "requests")
                if resp.status == 200 and len(body) == 1024:
                    add("http11_keepalive", "ok")
                else:
                    add("http11_keepalive", "errors")
                time.sleep(0.01)
        except socket.timeout:
            add("http11_keepalive", "requests")
            add("http11_keepalive", "timeouts")
        except Exception:
            add("http11_keepalive", "requests")
            add("http11_keepalive", "errors")
        finally:
            conn.close()
            time.sleep(0.05)

def invalid_worker():
    while time.time() < deadline:
        try:
            s = socket.create_connection((host, port), timeout=1.0)
            s.settimeout(1.0)
            s.sendall(b"BAD /not-http\r\nHost: invalid\r\n\r\n")
            try:
                s.recv(128)
            except Exception:
                pass
            add("invalid_requests", "sent")
            add("invalid_requests", "ok")
            s.close()
        except socket.timeout:
            add("invalid_requests", "sent")
            add("invalid_requests", "timeouts")
        except Exception:
            add("invalid_requests", "sent")
            add("invalid_requests", "errors")
        time.sleep(1.0)

def disconnect_worker():
    while time.time() < deadline:
        try:
            s = socket.create_connection((host, port), timeout=1.0)
            s.sendall(b"GET /site/large.bin HTTP/1.1\r\nHost: soak\r\n")
            s.close()
            add("client_disconnects", "sent")
            add("client_disconnects", "ok")
        except socket.timeout:
            add("client_disconnects", "sent")
            add("client_disconnects", "timeouts")
        except Exception:
            add("client_disconnects", "sent")
            add("client_disconnects", "errors")
        time.sleep(0.5)

threads = [
    threading.Thread(target=simple_worker, args=("static_small", "/site/small.txt", 1024, 0.005)),
    threading.Thread(target=simple_worker, args=("static_large", "/site/large.bin", 1024 * 1024, 0.05)),
    threading.Thread(target=mixed_worker),
    threading.Thread(target=keepalive_worker),
    threading.Thread(target=invalid_worker),
    threading.Thread(target=disconnect_worker),
]
for t in threads:
    t.daemon = True
    t.start()
for t in threads:
    t.join()

with open(out, "w") as f:
    json.dump({"duration_sec": duration, "stats": stats}, f, indent=2, sort_keys=True)
    f.write("\n")
PY
}

resource_sampler() {
  echo "ts_utc,pid,rss_kb,fd_count,cpu_pct,state" >"$RESOURCE_CSV"
  while [[ -e "$STOP_FILE" ]]; do
    local pid=""
    [[ -f "$PID_FILE" ]] && pid="$(cat "$PID_FILE" 2>/dev/null || true)"
    if [[ -n "$pid" && -d "/proc/$pid" ]]; then
      local rss fd cpu
      rss="$(awk '/VmRSS:/ {print $2; exit}' "/proc/$pid/status" 2>/dev/null || echo 0)"
      fd="$(ls "/proc/$pid/fd" 2>/dev/null | wc -l | tr -d ' ')"
      cpu="$(ps -p "$pid" -o %cpu= 2>/dev/null | tr -d ' ' || echo 0)"
      echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),$pid,${rss:-0},${fd:-0},${cpu:-0},running" >>"$RESOURCE_CSV"
    else
      echo "$(date -u +%Y-%m-%dT%H:%M:%SZ),${pid:-none},0,0,0,not_running" >>"$RESOURCE_CSV"
    fi
    sleep 2
  done
}

run_lifecycle_loop() {
  local end_epoch="$1"
  local reloads_ok=0 reloads_fail=0 drains_ok=0 drains_fail=0 shutdowns_ok=0 shutdowns_fail=0
  local restarts=0 unexpected_crashes=0 cycle_at=$((SECONDS + 120))
  while [[ "$(date +%s)" -lt "$end_epoch" ]]; do
    sleep 15
    if [[ -n "${EXYONQ_PID:-}" ]] && ! kill -0 "$EXYONQ_PID" 2>/dev/null; then
      unexpected_crashes=$((unexpected_crashes + 1))
      start_exyonq || true
      restarts=$((restarts + 1))
      cycle_at=$((SECONDS + 120))
      continue
    fi
    if "$EXYONQCTL_BIN" reload --config "$CFG" --socket "$CTRL_SOCK" >>"$RESULT_DIR/reload.log" 2>&1; then
      reloads_ok=$((reloads_ok + 1))
    else
      reloads_fail=$((reloads_fail + 1))
    fi
    status_line >>"$RESULT_DIR/status.log"
    if [[ "$SECONDS" -ge "$cycle_at" ]]; then
      if "$EXYONQCTL_BIN" drain --socket "$CTRL_SOCK" >>"$RESULT_DIR/drain.log" 2>&1; then
        drains_ok=$((drains_ok + 1))
      else
        drains_fail=$((drains_fail + 1))
      fi
      sleep 2
      if "$EXYONQCTL_BIN" shutdown --socket "$CTRL_SOCK" >>"$RESULT_DIR/shutdown.log" 2>&1; then
        shutdowns_ok=$((shutdowns_ok + 1))
      else
        shutdowns_fail=$((shutdowns_fail + 1))
      fi
      if ! wait_exyonq_exit; then
        echo "shutdown timeout, forcing process stop" >>"$RESULT_DIR/shutdown.log"
        kill "$EXYONQ_PID" 2>/dev/null || true
        wait "$EXYONQ_PID" 2>/dev/null || true
        shutdowns_fail=$((shutdowns_fail + 1))
      fi
      start_exyonq || true
      restarts=$((restarts + 1))
      cycle_at=$((SECONDS + 120))
    fi
    {
      echo "reloads_ok=$reloads_ok"
      echo "reloads_fail=$reloads_fail"
      echo "drains_ok=$drains_ok"
      echo "drains_fail=$drains_fail"
      echo "shutdowns_ok=$shutdowns_ok"
      echo "shutdowns_fail=$shutdowns_fail"
      echo "restarts=$restarts"
      echo "unexpected_crashes=$unexpected_crashes"
    } >"$LIFECYCLE_ENV"
  done
}

generate_report() {
  local verdict_file="$RESULT_DIR/verdict.txt"
  python3 - "$RESULT_DIR" "$REPORT_PATH" "$RUN_ID" "$HOST_LABEL" "$ACTUAL_ARCH" "$DURATION_SEC" "$WORKSPACE" >"$RESULT_DIR/report-generation.log" <<'PY'
import json
import os
import pathlib
import re
import subprocess
import sys

result_dir = pathlib.Path(sys.argv[1])
report_path = pathlib.Path(sys.argv[2])
run_id, host_label, arch, duration, workspace = sys.argv[3:8]
stats_path = result_dir / "loadgen-stats.json"
resource_path = result_dir / "resources.csv"
lifecycle_path = result_dir / "lifecycle.env"
server_log = result_dir / "exyonq.log"

stats = json.loads(stats_path.read_text()) if stats_path.exists() else {"stats": {}}
lifecycle = {}
if lifecycle_path.exists():
    for line in lifecycle_path.read_text().splitlines():
        if "=" in line:
            k, v = line.split("=", 1)
            lifecycle[k] = int(v or 0)

resource_rows = []
if resource_path.exists():
    for line in resource_path.read_text().splitlines()[1:]:
        parts = line.split(",")
        if len(parts) >= 6 and parts[2].isdigit() and parts[3].isdigit():
            resource_rows.append({
                "rss_kb": int(parts[2]),
                "fd_count": int(parts[3]),
                "cpu_pct": parts[4],
                "state": parts[5],
            })
max_rss_kb = max((r["rss_kb"] for r in resource_rows), default=0)
max_fd = max((r["fd_count"] for r in resource_rows), default=0)
not_running_samples = sum(1 for r in resource_rows if r["state"] == "not_running")

log_text = server_log.read_text(errors="replace") if server_log.exists() else ""
oom_count = len(re.findall(r"(?i)\b(oom|out of memory|killed process)\b", log_text))
panic_count = len(re.findall(r"(?i)\bpanic\b", log_text))

totals = {}
for name, values in stats.get("stats", {}).items():
    totals[name] = {k: int(v) for k, v in values.items()}
normal_requests = sum(v.get("requests", 0) for k, v in totals.items() if k not in {"invalid_requests", "client_disconnects"})
normal_ok = sum(v.get("ok", 0) for k, v in totals.items() if k not in {"invalid_requests", "client_disconnects"})
normal_timeouts = sum(v.get("timeouts", 0) for k, v in totals.items() if k not in {"invalid_requests", "client_disconnects"})
normal_errors = sum(v.get("errors", 0) for k, v in totals.items() if k not in {"invalid_requests", "client_disconnects"})
connections = normal_requests + sum(v.get("sent", 0) for v in totals.values())
timeout_limit = max(5, normal_requests // 50) if normal_requests else 5
blockers = []
if lifecycle.get("unexpected_crashes", 0):
    blockers.append(f"unexpected_crashes={lifecycle.get('unexpected_crashes')}")
if lifecycle.get("reloads_fail", 0):
    blockers.append(f"reloads_fail={lifecycle.get('reloads_fail')}")
if lifecycle.get("drains_fail", 0):
    blockers.append(f"drains_fail={lifecycle.get('drains_fail')}")
if lifecycle.get("shutdowns_fail", 0):
    blockers.append(f"shutdowns_fail={lifecycle.get('shutdowns_fail')}")
if oom_count:
    blockers.append(f"oom_count={oom_count}")
if panic_count:
    blockers.append(f"panic_count={panic_count}")
if normal_timeouts > timeout_limit:
    blockers.append(f"normal_timeouts={normal_timeouts} limit={timeout_limit}")
verdict = "PASS" if not blockers else "FAIL"

try:
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=workspace, text=True).strip()
except Exception:
    commit = os.environ.get("P1_1_SOAK_COMMIT") or os.environ.get("P11_SOAK_COMMIT", "unknown")
try:
    branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=workspace, text=True).strip()
except Exception:
    branch = os.environ.get("P1_1_SOAK_BRANCH") or os.environ.get("P11_SOAK_BRANCH", "unknown")

lines = [
    f"# P1.1 Soak Report ({host_label})",
    "",
    "## Verdict",
    "",
    f"**{verdict}**",
    "",
    "This is a bounded correctness/stability soak. It is not a competitive benchmark and makes no rival comparison or public performance claim.",
    "",
    "## Run Metadata",
    "",
    f"- Run ID: `{run_id}`",
    f"- Host label: `{host_label}`",
    f"- Architecture: `{arch}`",
    f"- Duration: `{duration}` seconds",
    f"- Commit: `{commit}`",
    f"- Branch: `{branch}`",
    f"- Evidence: `docs/operations/p1.1-soak-evidence/{run_id}/{host_label}/`",
    "",
    "## Coverage",
    "",
    "- Static small file: covered",
    "- Static large file: covered",
    "- HTTP/1.1 keep-alive: covered",
    "- Repeated reload: covered",
    "- Drain: covered",
    "- Shutdown/restart cycles: covered",
    "- Mixed static/proxy routes: covered",
    "- Client disconnects: covered",
    "- Invalid requests at bounded rate: covered",
    "",
    "## Counters",
    "",
    f"- Requests: {normal_requests}",
    f"- Connections/events: {connections}",
    f"- Successful normal responses: {normal_ok}",
    f"- Normal errors: {normal_errors}",
    f"- Normal timeouts: {normal_timeouts}",
    f"- Max RSS: {max_rss_kb} KiB",
    f"- Max FD count: {max_fd}",
    f"- Resource samples while process was intentionally down/restarting: {not_running_samples}",
    f"- Restarts: {lifecycle.get('restarts', 0)}",
    f"- Reloads OK/fail: {lifecycle.get('reloads_ok', 0)}/{lifecycle.get('reloads_fail', 0)}",
    f"- Drains OK/fail: {lifecycle.get('drains_ok', 0)}/{lifecycle.get('drains_fail', 0)}",
    f"- Shutdowns OK/fail: {lifecycle.get('shutdowns_ok', 0)}/{lifecycle.get('shutdowns_fail', 0)}",
    f"- Unexpected crashes: {lifecycle.get('unexpected_crashes', 0)}",
    f"- OOM markers: {oom_count}",
    f"- Panic markers: {panic_count}",
    "",
    "## Workload Breakdown",
    "",
]
for name in sorted(totals):
    values = totals[name]
    rendered = ", ".join(f"{k}={values[k]}" for k in sorted(values))
    lines.append(f"- `{name}`: {rendered}")
lines.extend([
    "",
    "## Blockers",
    "",
])
if blockers:
    lines.extend(f"- {b}" for b in blockers)
else:
    lines.append("- None.")
lines.extend([
    "",
    "## Notes",
    "",
    "- Errors during planned drain/shutdown/restart windows are counted, not hidden.",
    "- Timeout failure threshold for normal traffic is `max(5, requests / 50)` for this bounded gate.",
    "- Invalid requests and client disconnects are bounded fault inputs and are not counted as normal traffic failures.",
    "",
])
report_path.write_text("\n".join(lines) + "\n")
(result_dir / "verdict.txt").write_text(verdict + "\n")
print(verdict)
PY
}

BUILD_LOG="$RESULT_DIR/build.log"
echo "=== build release binaries ==="
cargo build --release -p exyonq -p exyonqctl >"$BUILD_LOG" 2>&1
EXYONQ_BIN="$WORKSPACE/target/release/exyonq"
EXYONQCTL_BIN="$WORKSPACE/target/release/exyonqctl"
[[ -x "$EXYONQ_BIN" && -x "$EXYONQCTL_BIN" ]] || {
  echo "ERROR: release binaries missing after build" >&2
  exit 1
}

LISTEN_PORT="${P1_1_SOAK_LISTEN_PORT:-${P11_SOAK_LISTEN_PORT:-$(pick_port)}}"
UPSTREAM_PORT="${P1_1_SOAK_UPSTREAM_PORT:-${P11_SOAK_UPSTREAM_PORT:-$(pick_port)}}"
write_fixture_tree
write_config "$LISTEN_PORT" "$UPSTREAM_PORT"
cp "$CFG" "$RESULT_DIR/exyonq.toml"
cp "$DOCROOT/manifest.json" "$RESULT_DIR/fixture-manifest.json"

start_upstream "$UPSTREAM_PORT"
start_exyonq
write_loadgen
touch "$STOP_FILE"
resource_sampler &
SAMPLER_PID=$!

END_EPOCH=$(($(date +%s) + DURATION_SEC))
python3 "$TMP/loadgen.py" 127.0.0.1 "$LISTEN_PORT" "$DURATION_SEC" "$LOADGEN_STATS" &
LOADGEN_PID=$!
run_lifecycle_loop "$END_EPOCH" &
LIFECYCLE_PID=$!

wait "$LOADGEN_PID"
LOADGEN_PID=""
kill "$LIFECYCLE_PID" 2>/dev/null || true
wait "$LIFECYCLE_PID" 2>/dev/null || true
LIFECYCLE_PID=""
rm -f "$STOP_FILE"
wait "$SAMPLER_PID" 2>/dev/null || true
SAMPLER_PID=""

if [[ -n "${EXYONQ_PID:-}" ]] && kill -0 "$EXYONQ_PID" 2>/dev/null; then
  "$EXYONQCTL_BIN" shutdown --socket "$CTRL_SOCK" >>"$RESULT_DIR/shutdown.log" 2>&1 || true
  wait_exyonq_exit || kill "$EXYONQ_PID" 2>/dev/null || true
fi

generate_report
VERDICT="$(cat "$RESULT_DIR/verdict.txt")"
echo "P1.1_SOAK_${HOST_LABEL}_VERDICT=$VERDICT report=$REPORT_PATH evidence=$RESULT_DIR"
[[ "$VERDICT" == "PASS" ]]
