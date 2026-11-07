#!/usr/bin/env bash
# Reproduce the OL-03 comparison against the local OpenLiteSpeed oracle.
#
# Preconditions:
#   - Docker is available and the configured OLS container exists.
#   - The oracle image is litespeedtech/openlitespeed:latest and its server
#     listens on OLS_PORT (host mapping).
#   - The oracle has /usr/local/lsws/Example/html/big256.bin, 262144 bytes.
# The script restarts the oracle before every scenario because OLS keeps the
# soft-limit marker and bans in process state.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${OL03_OLS_RUN_DIR:-$ROOT/.exyonq-local/evidence/ol03/ols-compare-$STAMP}"
OLS_CONTAINER="${OLS_CONTAINER:-eq-ols-oracle}"
OLS_PORT="${OLS_PORT:-19080}"
EXY_PORT="${EXY_PORT:-19390}"
PROBE="$ROOT/scripts/e2e/ol03-ols-probe.py"
mkdir -p "$RUN/www"
python3 - "$RUN/www/big256.bin" <<'PY'
import pathlib
import sys
pathlib.Path(sys.argv[1]).write_bytes(b"y" * 262144)
PY

EXY_PID=""
cleanup() {
  if [[ -n "$EXY_PID" ]]; then
    kill "$EXY_PID" 2>/dev/null || true
    wait "$EXY_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

wait_tcp() {
  local port=$1
  for _ in $(seq 1 120); do
    if python3 - "$port" <<'PY' 2>/dev/null
import socket, sys
s = socket.create_connection(("127.0.0.1", int(sys.argv[1])), 1)
s.close()
PY
    then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

wait_http() {
  local port=$1
  for _ in $(seq 1 120); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

# Docker's published port accepts TCP before OpenLiteSpeed has finished its
# startup work (QUIC cloud list download). A TCP connect is therefore not a
# readiness signal; an HTTP response from the container is. The request is a
# small static path so it does not consume the bandwidth scenario's body.
wait_ols_http() {
  local port=$1
  for _ in $(seq 1 300); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:$port/" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

start_exy() {
  local scenario=$1
  EXYONQ_CONTROL_SOCKET="/tmp/eq-ol03-$$-$scenario.sock" \
    "$BIN" serve -c "$RUN/exyonq.toml" >"$RUN/exy-$scenario.log" 2>&1 &
  EXY_PID=$!
  wait_http "$EXY_PORT" || {
    echo "FAIL exyonq did not start for $scenario" >&2
    cat "$RUN/exy-$scenario.log" >&2
    exit 1
  }
}

stop_exy() {
  if [[ -n "$EXY_PID" ]]; then
    kill "$EXY_PID" 2>/dev/null || true
    wait "$EXY_PID" 2>/dev/null || true
    EXY_PID=""
  fi
}

start_ols() {
  docker restart "$OLS_CONTAINER" >/dev/null
  wait_ols_http "$OLS_PORT" || {
    echo "FAIL OLS $OLS_CONTAINER did not answer HTTP on $OLS_PORT" >&2
    docker logs "$OLS_CONTAINER" >&2 || true
    exit 1
  }
}

cat >"$RUN/exyonq.toml" <<EOF
config_version = 1
[modules.ratelimit.client]
enabled = true
soft_connections_per_ip = 1
max_connections_per_ip = 2
bandwidth_bytes_per_second = 32768
input_bandwidth_bytes_per_second = 32768
ban_seconds = 3
grace_seconds = 2

[[server]]
listen = "127.0.0.1:$EXY_PORT"
routes = ["static"]

[[route]]
name = "static"
match = { path = "/" }
root = "$RUN/www"
EOF

echo "== environment =="
echo "host=$(uname -s)/$(uname -m)"
echo "binary=$BIN"
echo "binary_sha256=$(hash_file "$BIN")"
"$BIN" --version
echo "ols_container=$OLS_CONTAINER"
echo "ols_port=$OLS_PORT"
echo "exy_port=$EXY_PORT"

"$BIN" validate "$RUN/exyonq.toml" >"$RUN/validate.log" 2>&1
echo "validate_status=0"

# Start once to verify the oracle configuration; each scenario restarts it.
docker start "$OLS_CONTAINER" >/dev/null
wait_ols_http "$OLS_PORT" || {
  echo "FAIL OLS $OLS_CONTAINER did not answer HTTP on $OLS_PORT" >&2
  docker logs "$OLS_CONTAINER" >&2 || true
  exit 1
}
test "$(wc -c <"$RUN/www/big256.bin")" -eq 262144
docker exec "$OLS_CONTAINER" sh -c '
  set -e
  test -f /usr/local/lsws/Example/html/big256.bin
  test "$(wc -c </usr/local/lsws/Example/html/big256.bin)" -eq 262144
  grep -Eq "softLimit[[:space:]]+1$" /usr/local/lsws/conf/httpd_config.conf
  grep -Eq "hardLimit[[:space:]]+2$" /usr/local/lsws/conf/httpd_config.conf
  grep -Eq "gracePeriod[[:space:]]+2$" /usr/local/lsws/conf/httpd_config.conf
  grep -Eq "banPeriod[[:space:]]+3$" /usr/local/lsws/conf/httpd_config.conf
  grep -Eq "outBandwidth[[:space:]]+32768$" /usr/local/lsws/conf/httpd_config.conf
  grep -Eq "inBandwidth[[:space:]]+32768$" /usr/local/lsws/conf/httpd_config.conf
' >"$RUN/ols-config-check.log" 2>&1 || {
  echo "FAIL OLS oracle config does not match soft=1 hard=2 grace=2 ban=3 bandwidth=32768" >&2
  cat "$RUN/ols-config-check.log" >&2
  exit 1
}
echo "ols_config=soft=1 hard=2 grace=2 ban=3 out=32768 in=32768"

for scenario in single ab abc soft; do
  stop_exy
  start_ols
  start_exy "$scenario"
  python3 "$PROBE" 127.0.0.1 "$OLS_PORT" "$scenario" >"$RUN/ols-$scenario.json" 2>"$RUN/ols-$scenario.err"
  python3 "$PROBE" 127.0.0.1 "$EXY_PORT" "$scenario" >"$RUN/exy-$scenario.json" 2>"$RUN/exy-$scenario.err"
  docker logs "$OLS_CONTAINER" >"$RUN/ols-$scenario.container.log" 2>&1 || true
  stop_exy
done

python3 - "$RUN" <<'PY'
import json
import pathlib
import sys

run = pathlib.Path(sys.argv[1])
required = {"single": 1, "ab": 2, "abc": 3, "soft": 2}
failures = []
summary = {}

def load(tag, scenario):
    return json.loads((run / f"{tag}-{scenario}.json").read_text())

def fail(message):
    failures.append(message)

for scenario, count in required.items():
    ols = load("ols", scenario)
    exy = load("exy", scenario)
    summary[scenario] = {}
    for tag, result in (("ols", ols), ("exy", exy)):
        records = result["records"]
        summary[scenario][tag] = {
            "elapsed": result["elapsed"],
            "records": [
                {
                    "label": r["label"],
                    "body_bytes": r.get("body_bytes"),
                    "first_data": r["first_data"][1] if r["first_data"] else None,
                    "closed": r.get("closed"),
                    "data_events": r["data_events"],
                }
                for r in records
            ],
        }
        if len(records) != count:
            fail(f"{tag} {scenario}: {len(records)} records, expected {count}")
    ols_records = {r["label"]: r for r in ols["records"]}
    exy_records = {r["label"]: r for r in exy["records"]}
    if scenario == "single":
        for tag, r in (("ols", ols_records["A"]), ("exy", exy_records["A"])):
            if r.get("body_bytes") != 262144:
                fail(f"{tag} single: body={r.get('body_bytes')} expected 262144")
            first = r["first_data"][1] if r["first_data"] else None
            if first is None or first > 0.5:
                fail(f"{tag} single: first data={first} expected <= 0.5s")
        if abs(ols["elapsed"] - exy["elapsed"]) > 0.8:
            fail(f"single elapsed differs: OLS={ols['elapsed']} Exy={exy['elapsed']}")
    elif scenario in ("ab", "soft"):
        for tag, records in (("ols", ols_records), ("exy", exy_records)):
            for label in ("A", "B"):
                body = records[label].get("body_bytes")
                if body != 262144:
                    fail(f"{tag} {scenario} {label}: body={body} expected 262144")
        # Concurrent A/B scheduling is a declared differential: OLS may
        # serialize A then B or interleave them. Total elapsed is therefore
        # not part of the perClientConnLimit contract; both streams must
        # still complete with their full body, which is asserted above.
    elif scenario == "abc":
        for tag, records in (("ols", ols_records), ("exy", exy_records)):
            for label in ("A", "B"):
                body = records[label].get("body_bytes", 0)
                if body >= 262144:
                    fail(f"{tag} abc {label}: body={body} was not truncated")
            c = records["C"]
            if c.get("body_bytes", 0) != 0 or c["data_events"] != 0:
                fail(f"{tag} abc C: expected no frame, got body={c.get('body_bytes')} events={c['data_events']}")
            if "closed" not in c:
                fail(f"{tag} abc C: connection did not close")
        if ols["elapsed"] > 5 or exy["elapsed"] > 5:
            fail(f"abc did not close promptly: OLS={ols['elapsed']} Exy={exy['elapsed']}")

(run / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
if failures:
    for message in failures:
        print("FAIL " + message)
    raise SystemExit(1)
for scenario in required:
    print(
        f"PASS {scenario}: OLS={summary[scenario]['ols']['elapsed']}s "
        f"Exy={summary[scenario]['exy']['elapsed']}s"
    )
print("RESULT=PASS OL-03 OLS COMPARISON")
PY

echo "run=$RUN"
