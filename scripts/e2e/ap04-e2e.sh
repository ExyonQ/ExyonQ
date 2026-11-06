#!/usr/bin/env bash
# AP-04 real E2E: external auth subrequest against the built ExyonQ binary.
# Portable on macOS and Linux; no mocks or smoke shortcuts.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${AP04_RUN_DIR:-$ROOT/.exyonq-local/evidence/ap04/run-$STAMP}"
mkdir -p "$RUN"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

PORT_EXY=19080
PORT_AUTH=19081
PORT_ORIGIN=19082
EXY_PID=""
AUTH_PID=""
ORIGIN_PID=""
PASS=0
FAIL=0

cleanup() {
  [[ -n "$EXY_PID" ]] && kill "$EXY_PID" 2>/dev/null
  [[ -n "$AUTH_PID" ]] && kill "$AUTH_PID" 2>/dev/null
  [[ -n "$ORIGIN_PID" ]] && kill "$ORIGIN_PID" 2>/dev/null
  wait 2>/dev/null
}
trap cleanup EXIT

check() {
  local name=$1 expected=$2 actual=$3
  if [[ "$expected" == "$actual" ]]; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name expected=[$expected] actual=[$actual]"
    FAIL=$((FAIL + 1))
  fi
}

check_contains() {
  local name=$1 needle=$2 haystack=$3
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "PASS $name"
    PASS=$((PASS + 1))
  else
    echo "FAIL $name missing=[$needle] actual=[$haystack]"
    FAIL=$((FAIL + 1))
  fi
}

status() { curl -sS -o /dev/null -w '%{http_code}' --max-time 5 "$@"; }
auth_header() { printf 'Authorization: %s %s' 'Bearer' "$1"; }

wait_port() {
  local port=$1
  for _ in $(seq 1 100); do
    if curl -sS -o /dev/null --max-time 1 "http://127.0.0.1:$port/health" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

mkdir -p "$RUN/www"
printf 'static-origin' >"$RUN/www/index.html"

cat >"$RUN/auth.py" <<'PY'
import base64, hashlib, hmac, json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

JWT_SECRET = b'ap04-e2e-hs256-secret'

def b64url(raw):
    return base64.urlsafe_b64encode(raw).rstrip(b'=').decode('ascii')

def issue_jwt(sub='alice'):
    now = int(time.time())
    header = {'alg': 'HS256', 'typ': 'JWT'}
    claims = {
        'sub': sub,
        'iss': 'ap04-e2e',
        'aud': 'exyonq-ap04',
        'iat': now,
        'exp': now + 300,
    }
    signing_input = '.'.join((
        b64url(json.dumps(header, separators=(',', ':')).encode()),
        b64url(json.dumps(claims, separators=(',', ':')).encode()),
    )).encode('ascii')
    signature = hmac.new(JWT_SECRET, signing_input, hashlib.sha256).digest()
    return signing_input.decode('ascii') + '.' + b64url(signature)

def verify_jwt(token):
    try:
        header_b64, claims_b64, signature_b64 = token.split('.')
        signing_input = (header_b64 + '.' + claims_b64).encode('ascii')
        expected = hmac.new(JWT_SECRET, signing_input, hashlib.sha256).digest()
        actual = base64.urlsafe_b64decode(signature_b64 + '=' * (-len(signature_b64) % 4))
        if not hmac.compare_digest(expected, actual):
            return None
        header = json.loads(base64.urlsafe_b64decode(header_b64 + '=' * (-len(header_b64) % 4)))
        claims = json.loads(base64.urlsafe_b64decode(claims_b64 + '=' * (-len(claims_b64) % 4)))
        if header.get('alg') != 'HS256':
            return None
        if claims.get('iss') != 'ap04-e2e' or claims.get('aud') != 'exyonq-ap04':
            return None
        if int(claims.get('exp', 0)) < int(time.time()):
            return None
        sub = claims.get('sub')
        return sub if isinstance(sub, str) and sub else None
    except (ValueError, TypeError, KeyError, json.JSONDecodeError):
        return None

class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def reply(self, status, body=b'', headers=()):
        self.send_response(status)
        for name, value in headers:
            self.send_header(name, value)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        length = int(self.headers.get('Content-Length') or 0)
        body = self.rfile.read(length) if length else b''
        auth = self.headers.get('Authorization', '')
        if self.path == '/health':
            self.reply(200, b'ok'); return
        if self.path == '/issue':
            self.reply(200, issue_jwt().encode('ascii')); return
        if self.path == '/verify-jwt':
            token = auth[len('Bearer '):].strip() if auth.startswith('Bearer ') else ''
            sub = verify_jwt(token)
            if sub:
                self.reply(200, b'ok', [('X-Auth-Request-User', sub)]); return
            self.reply(401, b'bad jwt', [('WWW-Authenticate', 'Bearer realm="jwt"')]); return
        if self.path == '/slow':
            time.sleep(2.0)
            self.reply(200, b'slow', [('X-Auth-Request-User', 'slowpoke')]); return
        if self.path == '/inspect':
            problems = []
            expected_internal = {
                'X-ExyonQ-Original-URI': 1,
                'X-ExyonQ-Original-Method': 1,
                'X-ExyonQ-Original-Host': 1,
                'X-Auth-Request-User': 0,
                'X-ExyonQ-User': 0,
            }
            for name, expected in expected_internal.items():
                values = self.headers.get_all(name) or []
                if len(values) != expected:
                    problems.append('%s copies=%d expected=%d' % (name, len(values), expected))
            if self.command != 'GET': problems.append('method=' + self.command)
            if body: problems.append('body=%d' % len(body))
            if self.headers.get('X-ExyonQ-Original-URI') != '/inspect?probe=1':
                problems.append('uri=' + str(self.headers.get('X-ExyonQ-Original-URI')))
            if self.headers.get('X-ExyonQ-Original-Method') != 'POST':
                problems.append('orig_method=' + str(self.headers.get('X-ExyonQ-Original-Method')))
            if self.headers.get('X-ExyonQ-Original-Host') != '127.0.0.1:19080':
                problems.append('orig_host=' + str(self.headers.get('X-ExyonQ-Original-Host')))
            if self.headers.get('Authorization') != 'Bearer good':
                problems.append('authorization=' + str(self.headers.get('Authorization')))
            if self.headers.get('Cookie') != 'sid=abc':
                problems.append('cookie=' + str(self.headers.get('Cookie')))
            if self.headers.get('X-Custom') != 'yes':
                problems.append('x-custom=' + str(self.headers.get('X-Custom')))
            if self.headers.get('X-Hop') is not None:
                problems.append('x-hop leaked')
            if self.headers.get('Content-Length') is not None:
                problems.append('content-length leaked=' + str(self.headers.get('Content-Length')))
            if self.headers.get('Host') != '127.0.0.1:19081':
                problems.append('host=' + str(self.headers.get('Host')))
            if problems:
                sys.stderr.write('inspect rejected: ' + '; '.join(problems) + '\n')
                self.reply(403, ('; '.join(problems)).encode()); return
            self.reply(200, b'inspected', [('X-Auth-Request-User', 'inspector')]); return
        if self.path == '/verify':
            if auth == 'Bearer good':
                self.reply(200, b'ok', [('X-Auth-Request-User', 'alice')]); return
            if auth == 'Bearer bob':
                self.reply(200, b'ok', [('X-Auth-Request-User', 'bob')]); return
            if auth == 'Bearer nocontent':
                self.reply(204, b'ok', [('X-Auth-Request-User', 'carol')]); return
            if auth == 'Bearer forbidden':
                self.reply(403, b'forbidden'); return
            if auth == 'Bearer teapot':
                self.reply(418, b'teapot'); return
            if auth == 'Bearer redirect':
                self.reply(302, b'', [('Location', 'http://127.0.0.1:19081/elsewhere')]); return
            if auth == 'Bearer bare401':
                self.reply(401, b'no header'); return
            self.reply(401, b'no', [('WWW-Authenticate', 'Bearer realm="api"')]); return
        self.reply(404, b'not found')

    def log_message(self, *args):
        pass

ThreadingHTTPServer(('127.0.0.1', 19081), H).serve_forever()
PY

cat >"$RUN/origin.py" <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class H(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'
    def do_GET(self):
        body = b'static-origin' if self.path.startswith('/files/') else b'origin'
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_POST(self):
        length = int(self.headers.get('Content-Length') or 0)
        if length:
            self.rfile.read(length)
        body = b'origin'
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *args):
        pass

ThreadingHTTPServer(('127.0.0.1', 19082), H).serve_forever()
PY

cat >"$RUN/main.toml" <<EOF
config_version = 1

[modules.auth.routes.api]
auth_request = { endpoint = "http://127.0.0.1:19081/verify", timeout_ms = 500 }

[modules.auth.routes.files]
auth_request = { endpoint = "http://127.0.0.1:19081/verify", timeout_ms = 500 }

[modules.auth.routes.inspect]
auth_request = { endpoint = "http://127.0.0.1:19081/inspect", timeout_ms = 500 }

[modules.auth.routes.slowauth]
auth_request = { endpoint = "http://127.0.0.1:19081/slow", timeout_ms = 200 }

[modules.auth.routes.userzone]
auth_request = { endpoint = "http://127.0.0.1:19081/verify", timeout_ms = 500 }

[modules.auth.routes.jwt]
auth_request = { endpoint = "http://127.0.0.1:19081/verify-jwt", timeout_ms = 500 }

[modules.ratelimit]
enabled = false

[modules.ratelimit.zones.users]
key = "user"
requests_per_second = 1
burst = 1

[modules.ratelimit.routes]
userzone = "users"

[modules.ratelimit.route_options.userzone]
nodelay = true

[[server]]
listen = "127.0.0.1:19080"
routes = ["api", "files", "inspect", "slowauth", "userzone", "jwt"]

[[route]]
name = "api"
match = { path = "/api" }
upstream = "origin"

[[route]]
name = "files"
match = { path = "/files" }
root = "$RUN/www"
index = "index.html"

[[route]]
name = "inspect"
match = { path = "/inspect" }
upstream = "origin"

[[route]]
name = "slowauth"
match = { path = "/slowauth" }
upstream = "origin"

[[route]]
name = "userzone"
match = { path = "/user" }
upstream = "origin"

[[route]]
name = "jwt"
match = { path = "/jwt" }
upstream = "origin"

[[upstream]]
name = "origin"
target = "http://127.0.0.1:19082"
EOF

echo "== environment =="
echo "host=$(uname -s)/$(uname -m)"
echo "binary=$BIN"
echo "binary_sha256=$(hash_file "$BIN")"
"$BIN" --version
echo

echo "== config validation =="
"$BIN" validate "$RUN/main.toml" >"$RUN/validate-valid.log" 2>&1
check "validate accepted valid http endpoint" 0 "$?"
for case in https relative userinfo conflict; do
  case "$case" in
    https) endpoint="https://127.0.0.1:19081/verify"; extra="" ;;
    relative) endpoint="/verify"; extra="" ;;
    userinfo) endpoint="http://user:pass@127.0.0.1:19081/verify"; extra="" ;;
    conflict) endpoint="http://127.0.0.1:19081/verify"; extra='realm = "basic"
user_file = "/tmp/users"' ;;
  esac
  cat >"$RUN/invalid-$case.toml" <<EOF
config_version = 1
[modules.auth.routes.api]
auth_request = { endpoint = "$endpoint", timeout_ms = 500 }
$extra
[[server]]
listen = "127.0.0.1:19089"
routes = ["api"]
[[route]]
name = "api"
match = { path = "/" }
upstream = "origin"
[[upstream]]
name = "origin"
target = "http://127.0.0.1:19082"
EOF
  "$BIN" validate "$RUN/invalid-$case.toml" >"$RUN/validate-$case.log" 2>&1
  rc=$?
  if [[ $rc -ne 0 ]]; then
    echo "PASS validate rejected $case (exit=$rc)"
    PASS=$((PASS + 1))
  else
    echo "FAIL validate accepted $case"
    FAIL=$((FAIL + 1))
  fi
done
grep -q "absolute http:// URI" "$RUN/validate-https.log" && echo "PASS https rejection message" && PASS=$((PASS + 1)) || { echo "FAIL https rejection message"; FAIL=$((FAIL + 1)); }
grep -q "userinfo" "$RUN/validate-userinfo.log" && echo "PASS userinfo rejection message" && PASS=$((PASS + 1)) || { echo "FAIL userinfo rejection message"; FAIL=$((FAIL + 1)); }
grep -q "cannot be combined" "$RUN/validate-conflict.log" && echo "PASS conflict rejection message" && PASS=$((PASS + 1)) || { echo "FAIL conflict rejection message"; FAIL=$((FAIL + 1)); }
echo

echo "== live services =="
python3 "$RUN/auth.py" >"$RUN/auth.log" 2>&1 & AUTH_PID=$!
python3 "$RUN/origin.py" >"$RUN/origin.log" 2>&1 & ORIGIN_PID=$!
wait_port 19081 || { echo "FAIL auth server did not start"; exit 1; }
wait_port 19082 || { echo "FAIL origin server did not start"; exit 1; }
EXYONQ_CONTROL_SOCKET="$RUN/control.sock" "$BIN" serve -c "$RUN/main.toml" >"$RUN/exy.log" 2>&1 & EXY_PID=$!
for _ in $(seq 1 100); do
  code=$(status "http://127.0.0.1:19080/api")
  [[ "$code" == "401" ]] && break
  sleep 0.05
done
check "server ready with 401" 401 "$code"

base="http://127.0.0.1:19080"
check "no credentials -> 401" 401 "$(status "$base/api")"
check "valid bearer -> 200" 200 "$(status -H "$(auth_header good)" "$base/api")"
check "origin body after allow" origin "$(curl -sS -H "$(auth_header good)" "$base/api")"
check "valid 204 -> 200" 200 "$(status -H "$(auth_header nocontent)" "$base/api")"
check "forbidden -> 403" 403 "$(status -H "$(auth_header forbidden)" "$base/api")"
check "unexpected 418 -> 503" 503 "$(status -H "$(auth_header teapot)" "$base/api")"
check "redirect -> 503" 503 "$(status -H "$(auth_header redirect)" "$base/api")"
check "bare 401 stays 401" 401 "$(status -H "$(auth_header bare401)" "$base/api")"
headers=$(curl -sS -D - -o /dev/null "$base/api")
check_contains "401 forwards WWW-Authenticate" 'www-authenticate: bearer realm="api"' "$(printf '%s' "$headers" | tr '[:upper:]' '[:lower:]')"

slow=$(curl -sS -o /dev/null -w '%{http_code} %{time_total}' --max-time 5 -H "$(auth_header good)" "$base/slowauth")
check "auth timeout -> 503" 503 "${slow%% *}"
python3 - "$slow" <<'PY'
import sys
status, elapsed = sys.argv[1].split()
if float(elapsed) >= 1.5:
    raise SystemExit('timeout exceeded budget: ' + sys.argv[1])
PY
check "auth timeout under 1.5s" 0 "$?"

check "static route bypass blocked" 401 "$(status "$base/files/index.html")"
static=$(curl -sS -H "$(auth_header good)" "$base/files/index.html")
check "static route body after allow" static-origin "$static"

inspect=$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 \
  -X POST "$base/inspect?probe=1" \
  -H "$(auth_header good)" \
  -H 'Cookie: sid=abc' \
  -H 'X-Custom: yes' \
  -H 'X-ExyonQ-Original-URI: /spoofed' \
  -H 'X-ExyonQ-Original-Method: DELETE' \
  -H 'X-ExyonQ-Original-Host: evil.example' \
  -H 'X-Auth-Request-User: attacker' \
  -H 'X-ExyonQ-User: attacker' \
  -H 'Connection: X-Hop' \
  -H 'X-Hop: secret' \
  --data 'payload')
check "inspect subrequest headers/method/body" 200 "$inspect"

alice1=$(status -H "$(auth_header good)" "$base/user")
alice2=$(status -H "$(auth_header good)" "$base/user")
alice3=$(status -H "$(auth_header good)" "$base/user")
check "user zone alice first -> 200" 200 "$alice1"
check "user zone alice burst -> 200" 200 "$alice2"
check "user zone alice exhausted -> 429" 429 "$alice3"
check "user zone bob isolated -> 200" 200 "$(status -H "$(auth_header bob)" "$base/user")"

jwt=$(curl -sS --max-time 5 "http://127.0.0.1:19081/issue")
check "real HS256 JWT accepted at endpoint" 200 "$(status -H "$(auth_header "$jwt")" "$base/jwt")"
tampered=$(python3 - "$jwt" <<'PY'
import sys
parts = sys.argv[1].split('.')
signature = parts[2]
middle = len(signature) // 2
replacement = 'A' if signature[middle] != 'A' else 'B'
parts[2] = signature[:middle] + replacement + signature[middle + 1:]
print('.'.join(parts))
PY
)
check "tampered JWT rejected at endpoint" 401 "$(status -H "$(auth_header "$tampered")" "$base/jwt")"

kill "$AUTH_PID" 2>/dev/null; wait "$AUTH_PID" 2>/dev/null; AUTH_PID=""
sleep 0.2
check "auth server down -> 503" 503 "$(status -H "$(auth_header good)" "$base/api")"

echo
echo "== auth server diagnostics =="
cat "$RUN/auth.log"
echo
echo "SUMMARY pass=$PASS fail=$FAIL run=$RUN"
[[ $FAIL -eq 0 ]] || exit 1
echo "RESULT=PASS AP-04"
