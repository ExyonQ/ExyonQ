#!/usr/bin/env bash
# OL-04 real E2E: native HMAC-PoW challenge, external CAPTCHA providers, grant,
# replay, tampering and explicit no-JavaScript fallback.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
PORT="${OL04_PORT:-19480}"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN="${OL04_RUN_DIR:-$ROOT/.exyonq-local/evidence/ol04/run-$STAMP}"
mkdir -p "$RUN/www"
printf 'challenge-ok' >"$RUN/www/index.html"

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

echo "== environment =="
echo "host=$(uname -s)/$(uname -m)"
echo "binary=$BIN"
echo "binary_sha256=$(hash_file "$BIN")"
"$BIN" --version
python3 -c 'open("'$RUN'/key.bin","wb").write(bytes(range(32)))'
EXY_PID=""
PASS=0
FAIL=0

# Isolate from a caller's WAF environment; the test sets only its own values.
unset EXYONQ_WAF_MODE EXYONQ_WAF_ENFORCE EXYONQ_WAF_ABUSE \
  EXYONQ_WAF_ABUSE_RPS EXYONQ_WAF_ABUSE_BURST
export OL04_RECAPTCHA_SECRET='6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe'
export OL04_HCAPTCHA_SECRET='0x0000000000000000000000000000000000000000'
export OL04_BAD_RECAPTCHA_SECRET='invalid-secret'

stop_server() {
  if [[ -n "$EXY_PID" ]]; then
    kill "$EXY_PID" 2>/dev/null
    wait "$EXY_PID" 2>/dev/null
    EXY_PID=""
  fi
}
trap stop_server EXIT

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

tcp_wait() {
  # TCP readiness only: an HTTP probe would consume the one-token burst.
  for _ in $(seq 1 120); do
    if python3 -c 'import socket,sys; s=socket.create_connection(("127.0.0.1", int(sys.argv[1])), 1); s.close()' "$PORT" 2>/dev/null; then
      return 0
    fi
    sleep 0.05
  done
  return 1
}

start_server() {
  local phase=$1 cfg=$2
  stop_server
  # macOS sockaddr_un is short; keep the control socket outside the long run path.
  local ctrl="/tmp/ol04-${phase}-$$.sock"
  rm -f "$ctrl"
  EXYONQ_CONTROL_SOCKET="$ctrl" \
  EXYONQ_WAF_CHALLENGE_KEY_FILE="$RUN/key.bin" \
    "$BIN" serve -c "$cfg" >"$RUN/exy-$phase.log" 2>&1 & EXY_PID=$!
  if ! tcp_wait; then
    echo "FAIL server $phase did not start"
    cat "$RUN/exy-$phase.log"
    exit 1
  fi
}

validate_cfg() {
  local name=$1 cfg=$2
  "$BIN" validate "$cfg" >"$RUN/validate-$name.log" 2>&1
  check "validate $name config" 0 "$?"
}

write_config() {
  local path=$1 provider=${2:-native} site_key=${3:-} secret_env=${4:-} waf_mode=${5:-block}
  cat >"$path" <<EOF
config_version = 1

[waf]
enabled = true
mode = "$waf_mode"

[waf.abuse]
mode = "challenge"
requests_per_second = 1
burst = 1
EOF
  if [[ "$provider" != "native" ]]; then
    cat >>"$path" <<EOF
provider = "$provider"
site_key = "$site_key"
secret_env = "$secret_env"
EOF
  fi
  cat >>"$path" <<EOF

[[server]]
listen = "127.0.0.1:$PORT"
routes = ["site"]

[[route]]
name = "site"
match = { path = "/" }
root = "$RUN/www"
EOF
}

cat >"$RUN/client.py" <<'PY'
import hashlib
import http.client
import json
import re
import sys
import urllib.parse

PORT = int(sys.argv[1])
MODE = sys.argv[2]
TOKEN = sys.argv[3]
EXPECT_SUCCESS = sys.argv[4] == '1'
HOST = '127.0.0.1'
PATH = '/index.html'
results = []

PROVIDERS = {
    'recaptcha': ('www.google.com/recaptcha/api.js', 'g-recaptcha'),
    'bad-recaptcha': ('www.google.com/recaptcha/api.js', 'g-recaptcha'),
    'hcaptcha': ('js.hcaptcha.com/1/api.js', 'h-captcha'),
}
SITE_KEY = '6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI' if 'recaptcha' in MODE else '10000000-ffff-ffff-ffff-000000000001'

def check(name, expected, actual):
    ok = expected == actual
    results.append((name, ok, expected, actual))
    print(('PASS ' if ok else 'FAIL ') + name)
    if not ok:
        print(f'  expected={expected!r} actual={actual!r}')

def request(method, path, body=None, headers=None):
    conn = http.client.HTTPConnection(HOST, PORT, timeout=10)
    h = {'Host': f'{HOST}:{PORT}'}
    if headers:
        h.update(headers)
    if body is not None and 'Content-Type' not in h:
        h['Content-Type'] = 'application/x-www-form-urlencoded'
    conn.request(method, path, body=body, headers=h)
    resp = conn.getresponse()
    data = resp.read()
    result = (resp.status, {k.lower(): v for k, v in resp.getheaders()}, data)
    conn.close()
    return result

def js_const(html, name):
    m = re.search(r'const ' + re.escape(name) + r' = ([^;]+);', html)
    if not m:
        raise SystemExit(f'missing {name} in challenge page')
    value = m.group(1).strip()
    return json.loads(value) if value.startswith('"') else int(value)

def leading_zero_bits(digest):
    bits = 0
    for byte in digest:
        if byte == 0:
            bits += 8
            continue
        bits += 8 - byte.bit_length()
        break
    return bits

check('first request reaches protected route', 200, request('GET', PATH)[0])
status, headers, body = request('GET', PATH)
check('second request is challenged', 403, status)
check('challenge is HTML', True, headers.get('content-type', '').startswith('text/html'))
html = body.decode('utf-8')
check('native page has noscript fallback', True, '<noscript>' in html)
check('noscript explains requirement', True, 'JavaScript is required' in html)
check('challenge page is no-store', True, headers.get('cache-control') == 'no-store')

if MODE in ('native', 'monitor'):
    check('native page has no external script', True, '<script src=' not in html)
elif MODE == 'recaptcha-v3':
    check('provider page loads expected v3 script', True, 'www.google.com/recaptcha/api.js?render=' in html)
    check('provider page carries configured site key', True, SITE_KEY in html)
    check('provider page executes expected action', True, 'action: "challenge"' in html)
    check('provider page has no v2 widget', True, 'g-recaptcha' not in html)
    check('provider page waits for provider token', True, 'await waitForProviderToken()' in html)
else:
    script, widget = PROVIDERS[MODE]
    check('provider page loads expected widget script', True, script in html)
    check('provider page renders expected widget', True, widget in html)
    check('provider page carries configured site key', True, f'data-sitekey="{SITE_KEY}"' in html)
    check('provider page waits for provider token', True, 'await waitForProviderToken()' in html)

try:
    challenge_id = js_const(html, 'challengeId')
    mac = js_const(html, 'mac')
    expires = js_const(html, 'expires')
    difficulty = js_const(html, 'difficulty')
except SystemExit as exc:
    print(f'FAIL {exc}')
    sys.exit(1)

nonce = 0
while True:
    digest = hashlib.sha256(f'{challenge_id}:{nonce}'.encode()).digest()
    if leading_zero_bits(digest) >= difficulty:
        break
    nonce += 1

form = {
    'challenge_id': challenge_id,
    'nonce': str(nonce),
    'mac': mac,
    'expires': str(expires),
    'difficulty': str(difficulty),
    'return_to': PATH,
}
if MODE != 'native':
    form['provider_token'] = TOKEN
body_form = urllib.parse.urlencode(form)

bad = dict(form)
bad['mac'] = '00' * 32
bad_status, _, _ = request('POST', '/.exyonq/waf-challenge', urllib.parse.urlencode(bad))
check('forged challenge MAC rejected', 403, bad_status)

ok_status, ok_headers, _ = request('POST', '/.exyonq/waf-challenge', body_form)
if EXPECT_SUCCESS:
    check('valid PoW/provider returns redirect', 302, ok_status)
    check('redirect targets return_to', PATH, ok_headers.get('location'))
    set_cookie = ok_headers.get('set-cookie', '')
    check('grant cookie is HttpOnly', True, 'HttpOnly' in set_cookie)
    check('grant cookie is SameSite=Lax', True, 'SameSite=Lax' in set_cookie)
    check('grant cookie has Max-Age=300', True, 'Max-Age=300' in set_cookie)
    m = re.search(r'exyonq_waf_grant=([^;]+)', set_cookie)
    if not m:
        print('FAIL grant cookie value missing')
        sys.exit(1)
    grant = m.group(1)
    check('grant cookie accepted', 200, request('GET', PATH, headers={'Cookie': f'exyonq_waf_grant={grant}'})[0])
    # The abuse quota is a one-token bucket. A provider round trip can take the
    # second that separates the grant check from this one, which would let the
    # request pass on quota alone and hide whether the forged grant was honoured.
    # Exhaust the bucket again so the outcome depends only on the grant.
    request('GET', PATH)
    request('GET', PATH)
    check('tampered grant rejected', 403, request('GET', PATH, headers={'Cookie': 'exyonq_waf_grant=v1.9999999999.16.0.deadbeef'})[0])
    replay_status, _, _ = request('POST', '/.exyonq/waf-challenge', body_form)
    check('replayed challenge rejected', 403, replay_status)
else:
    check('provider rejection is fail-closed', 403, ok_status)
    check('provider rejection sets no grant', None, ok_headers.get('set-cookie'))

failures = [r for r in results if not r[1]]
print(f'SUMMARY mode={MODE} checks={len(results)} failures={len(failures)}')
if failures:
    sys.exit(1)
PY

NATIVE_CFG="$RUN/native.toml"
MONITOR_CFG="$RUN/monitor.toml"
RECAPTCHA_CFG="$RUN/recaptcha.toml"
BAD_RECAPTCHA_CFG="$RUN/bad-recaptcha.toml"
HCAPTCHA_CFG="$RUN/hcaptcha.toml"
V3_RECAPTCHA_CFG="$RUN/recaptcha-v3.toml"
write_config "$NATIVE_CFG" native
write_config "$MONITOR_CFG" native '' '' monitor
write_config "$RECAPTCHA_CFG" recaptcha 6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI OL04_RECAPTCHA_SECRET
write_config "$BAD_RECAPTCHA_CFG" recaptcha 6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI OL04_BAD_RECAPTCHA_SECRET
write_config "$HCAPTCHA_CFG" hcaptcha 10000000-ffff-ffff-ffff-000000000001 OL04_HCAPTCHA_SECRET
write_config "$V3_RECAPTCHA_CFG" recaptcha_v3 6LeIxAcTAAAAAJcZVRqyHh71UMIEGNQ_MXjiZKhI OL04_RECAPTCHA_SECRET

for phase in native monitor recaptcha recaptcha-v3 bad-recaptcha hcaptcha; do
  validate_cfg "$phase" "$RUN/$phase.toml"
done

# Missing provider fields must fail config validation.
cat >"$RUN/bad-config.toml" <<EOF
config_version = 1
[waf]
enabled = true
mode = "block"
[waf.abuse]
mode = "challenge"
provider = "recaptcha"
[[server]]
listen = "127.0.0.1:$PORT"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/" }
root = "$RUN/www"
EOF
"$BIN" validate "$RUN/bad-config.toml" >"$RUN/validate-bad-config.log" 2>&1
check "invalid provider config rejected" 1 "$?"

run_client() {
  local phase=$1 mode=$2 token=$3 expect=$4
  python3 "$RUN/client.py" "$PORT" "$mode" "$token" "$expect" >"$RUN/client-$phase.log" 2>&1
  local rc=$?
  cat "$RUN/client-$phase.log"
  check "OL-04 $phase assertions" 0 "$rc"
}

for phase in native monitor recaptcha recaptcha-v3 hcaptcha; do
  start_server "$phase" "$RUN/$phase.toml"
  case "$phase" in
    native) run_client native native '' 1 ;;
    monitor) run_client monitor native '' 1 ;;
    recaptcha) run_client recaptcha recaptcha test-token 1 ;;
    # Google's official v2 test key accepts the token but omits v3 score/action.
    # This real provider call must fail closed; a positive v3 token needs a real
    # v3 key and is covered by the response-contract unit test, not invented here.
    recaptcha-v3) run_client recaptcha-v3 recaptcha-v3 test-token 0 ;;
    hcaptcha) run_client hcaptcha hcaptcha 10000000-aaaa-bbbb-cccc-000000000001 1 ;;
  esac
done

start_server bad-recaptcha "$BAD_RECAPTCHA_CFG"
run_client bad-recaptcha bad-recaptcha test-token 0

stop_server
echo
echo "SUMMARY pass=$PASS fail=$FAIL run=$RUN"
[[ $FAIL -eq 0 ]] || exit 1
echo "RESULT=PASS OL-04"
