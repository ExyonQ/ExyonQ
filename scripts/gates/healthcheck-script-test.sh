#!/usr/bin/env bash
# Unit-level shell-logic test for packaging/scripts/exyonq-healthcheck.sh.
# systemctl and curl are test doubles: this proves the resolution and failure
# branches, not a real systemd/PHP-FPM/HTTP journey.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# Default to the source copy. HEALTHCHECK_SCRIPT lets the same gate run the
# script extracted from a built .deb/.rpm, which is the artifact that ships.
SCRIPT="${HEALTHCHECK_SCRIPT:-$ROOT/packaging/scripts/exyonq-healthcheck.sh}"
[[ -x "$SCRIPT" ]] || {
  echo "ERROR: healthcheck script is not executable: $SCRIPT" >&2
  exit 2
}
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
BIN="$TMP/bin"
STATUS="$TMP/status.txt"
mkdir -p "$BIN"

cat >"$BIN/systemctl" <<'EOF'
#!/bin/sh
case "${1:-}" in
  is-active)
    shift
    [ "${1:-}" = "--quiet" ] && shift
    unit="${1:-}"
    for candidate in ${FAKE_ACTIVE:-}; do
      [ "$candidate" = "$unit" ] && exit 0
    done
    exit 3
    ;;
  list-units)
    for unit in ${FAKE_ACTIVE:-}; do
      printf '%s loaded active running test\n' "$unit"
    done
    ;;
  list-unit-files)
    for unit in ${FAKE_KNOWN:-}; do
      printf '%s enabled\n' "$unit"
    done
    ;;
  *)
    exit 1
    ;;
esac
EOF

cat >"$BIN/curl" <<'EOF'
#!/bin/sh
case "${FAKE_CURL:-ok}" in
  fail) exit 22 ;;
  redirect) printf '301'; exit 0 ;;
  *) printf '200'; exit 0 ;;
esac
EOF
chmod +x "$BIN/systemctl" "$BIN/curl"

SHELL_BIN="$(command -v dash || command -v sh)"
CASE_OUTPUT=""
CASE_RC=0
PASS=0
FAIL=0

run_case() {
  rm -f "$STATUS"
  set +e
  CASE_OUTPUT="$(env \
    PATH="$BIN:$PATH" \
    TMPDIR="$TMP" \
    FAKE_ACTIVE="${FAKE_ACTIVE:-}" \
    FAKE_KNOWN="${FAKE_KNOWN:-}" \
    FAKE_CURL="${FAKE_CURL:-ok}" \
    EXYONQ_HEALTHCHECK_STATUS_FILE="$STATUS" \
    EXYONQ_HEALTHCHECK_URLS="${TEST_URLS:-http://127.0.0.1:8080/health}" \
    EXYONQ_HEALTHCHECK_PHP_FPM_UNIT="${TEST_PHP_UNIT:-}" \
    "$SHELL_BIN" "$SCRIPT" 2>&1)"
  CASE_RC=$?
  set -e
}

assert_rc() {
  local expected="$1" name="$2"
  if [[ "$CASE_RC" -eq "$expected" ]]; then
    echo "PASS $name (exit $CASE_RC)"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name (expected exit $expected, got $CASE_RC)"
    echo "$CASE_OUTPUT"
    FAIL=$((FAIL + 1))
  fi
}

assert_contains() {
  local needle="$1" name="$2"
  if grep -Fq -- "$needle" <<<"$CASE_OUTPUT"; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name (missing: $needle)"
    echo "$CASE_OUTPUT"
    FAIL=$((FAIL + 1))
  fi
}

assert_status_contains() {
  local needle="$1" name="$2"
  if [[ -f "$STATUS" ]] && grep -Fq -- "$needle" "$STATUS"; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name (missing from status: $needle)"
    [[ -f "$STATUS" ]] && cat "$STATUS"
    FAIL=$((FAIL + 1))
  fi
}

# Real-input cases: no systemctl/curl doubles. The Unix socket and the HTTP
# endpoints below are real; only systemd is bypassed with UNITS=none.
run_real_case() {
  rm -f "$STATUS"
  set +e
  CASE_OUTPUT="$(env \
    PATH="/usr/bin:/bin:/usr/sbin:/sbin" \
    TMPDIR="$TMP" \
    EXYONQ_HEALTHCHECK_UNITS="${TEST_UNITS:-none}" \
    EXYONQ_HEALTHCHECK_URLS="${TEST_URLS:-http://127.0.0.1:8080/health}" \
    EXYONQ_HEALTHCHECK_TIMEOUT="${TEST_TIMEOUT:-3}" \
    EXYONQ_HEALTHCHECK_PHP_FPM_SOCKET="${TEST_SOCKET:-}" \
    EXYONQ_HEALTHCHECK_PHP_FPM_UNIT="${TEST_PHP_UNIT:-}" \
    EXYONQ_HEALTHCHECK_STATUS_FILE="$STATUS" \
    "$SHELL_BIN" "$SCRIPT" 2>&1)"
  CASE_RC=$?
  set -e
}

echo "healthcheck-script-test: DEVELOPMENT_AID (stubbed systemctl/curl; not E2E)"

FAKE_ACTIVE="exyonq.service php-fpm.service"
FAKE_KNOWN="php-fpm.service"
TEST_PHP_UNIT=""
FAKE_CURL="ok"
run_case
assert_rc 0 "stable php-fpm unit is accepted"
assert_contains "OK unit php-fpm.service" "stable unit reported"
assert_status_contains "OK http http://127.0.0.1:8080/health status=200" "status.txt records HTTP probe"

FAKE_ACTIVE="exyonq.service php8.5.12-fpm.service"
FAKE_KNOWN="php8.5.12-fpm.service"
run_case
assert_rc 0 "versioned php-fpm unit is discovered"
assert_contains "OK unit php8.5.12-fpm.service" "versioned unit reported"

FAKE_ACTIVE="exyonq.service"
FAKE_KNOWN="php8.3-fpm.service php8.5.12-fpm.service"
TEST_PHP_UNIT="php8.5.12-fpm.service"
run_case
assert_rc 1 "explicit inactive unit fails"
assert_contains "candidates: php8.3-fpm.service php8.5.12-fpm.service" "failure lists candidates"

FAKE_ACTIVE="exyonq.service"
FAKE_KNOWN="php8.5.12-fpm.service"
TEST_PHP_UNIT=""
run_case
assert_rc 1 "inactive installed unit fails"
assert_contains "php-fpm no active unit" "no-active failure is explicit"
assert_contains "candidates: php8.5.12-fpm.service" "no-active failure lists candidates"

FAKE_ACTIVE="exyonq.service"
FAKE_KNOWN=""
run_case
assert_rc 0 "no installed php-fpm is skipped"
assert_contains "SKIP php-fpm no installed unit" "skip is recorded"

FAKE_ACTIVE="exyonq.service php-fpm.service"
FAKE_KNOWN="php-fpm.service"
FAKE_CURL="fail"
run_case
assert_rc 1 "HTTP failure fails the check"
assert_contains "FAIL http http://127.0.0.1:8080/health curl_exit=22" "HTTP failure is recorded"

FAKE_CURL="redirect"
run_case
assert_rc 1 "HTTP redirect is not accepted as healthy"
assert_contains "FAIL http http://127.0.0.1:8080/health status=301" "redirect status is recorded"

FAKE_ACTIVE="exyonq.service"
FAKE_KNOWN="php8.5.12-fpm.service"
TEST_PHP_UNIT="off"
FAKE_CURL="ok"
run_case
assert_rc 0 "explicit off skips php-fpm"
assert_contains "SKIP php-fpm disabled" "off is recorded"

# --- real socket and real HTTP cases (no doubles) -------------------------
HTTP_PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
python3 -m http.server "$HTTP_PORT" --bind 127.0.0.1 --directory "$TMP" >/dev/null 2>&1 &
HTTP_PID=$!
for _ in $(seq 1 40); do
  curl -fsS --max-time 1 "http://127.0.0.1:$HTTP_PORT/" >/dev/null 2>&1 && break
  sleep 0.1
done

SOCK="$TMP/php-fpm.sock"
python3 - "$SOCK" <<'PY_SOCK' &
import socket, sys, time
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.bind(sys.argv[1])
sock.listen(1)
time.sleep(30)
PY_SOCK
SOCK_PID=$!
for _ in $(seq 1 40); do
  [ -S "$SOCK" ] && break
  sleep 0.1
done

TEST_UNITS="none"
TEST_PHP_UNIT=""
TEST_TIMEOUT="3"
TEST_URLS="http://127.0.0.1:$HTTP_PORT/"
TEST_SOCKET="$SOCK"
run_real_case
assert_rc 0 "real php-fpm socket and real HTTP 200 pass (no doubles)"
assert_contains "OK php-fpm socket $SOCK" "real socket probe is recorded"
assert_contains "SKIP systemd unit checks disabled" "systemd-free mode is explicit"

TEST_SOCKET="$TMP/absent.sock"
run_real_case
assert_rc 1 "missing php-fpm socket fails"
assert_contains "FAIL php-fpm socket missing: $TMP/absent.sock" "missing socket is explicit"

TEST_SOCKET=""
run_real_case
assert_rc 0 "real HTTP 200 endpoint passes"
assert_contains "OK http http://127.0.0.1:$HTTP_PORT/ status=200" "real HTTP probe is recorded"

TEST_URLS="http://127.0.0.1:1/health"
run_real_case
assert_rc 1 "unreachable endpoint fails (php-fpm not answering)"
assert_contains "curl_exit=" "unreachable endpoint reports the curl exit code"

kill "$SOCK_PID" "$HTTP_PID" 2>/dev/null || true
wait "$SOCK_PID" "$HTTP_PID" 2>/dev/null || true

echo "RESULT pass=$PASS fail=$FAIL"
[[ "$FAIL" -eq 0 ]]
