#!/usr/bin/env bash
# Real F-08 mTLS/CRL/min_version E2E over a real TLS listener.
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "RESULT=NOT_APPLICABLE LINUX_REQUIRED platform=$(uname -s)"
  exit 2
fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
[[ -x "$EXYONQ_BIN" ]] || { echo "RESULT=BLOCKED missing=$EXYONQ_BIN"; exit 3; }
command -v openssl >/dev/null || { echo "RESULT=BLOCKED missing=openssl"; exit 3; }

TMP="$(mktemp -d)"
PID=""
cleanup() {
  if [[ -n "$PID" ]] && kill -0 "$PID" 2>/dev/null; then
    kill "$PID" 2>/dev/null || true
    wait "$PID" 2>/dev/null || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/ca/newcerts" "$TMP/www"
: >"$TMP/ca/index.txt"
echo 1000 >"$TMP/ca/serial"
echo 1000 >"$TMP/ca/crlnumber"
printf 'ok\n' >"$TMP/www/index.html"
cat >"$TMP/ca/openssl.cnf" <<EOF
[ca]
default_ca = CA_default
[CA_default]
dir = $TMP/ca
database = \$dir/index.txt
new_certs_dir = \$dir/newcerts
certificate = $TMP/ca/ca.pem
private_key = $TMP/ca/ca.key
serial = \$dir/serial
crlnumber = \$dir/crlnumber
default_md = sha256
default_days = 2
default_crl_days = 2
policy = policy_any
unique_subject = no
x509_extensions = usr_cert
[policy_any]
commonName = supplied
[usr_cert]
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth, clientAuth
EOF

openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=F08 Test CA" \
  -keyout "$TMP/ca/ca.key" -out "$TMP/ca/ca.pem" >/dev/null 2>&1

issue() { # name cn san?
  local name="$1" cn="$2" san="${3:-}"
  openssl req -newkey rsa:2048 -nodes -subj "/CN=$cn" \
    -keyout "$TMP/ca/$name.key" -out "$TMP/ca/$name.csr" >/dev/null 2>&1
  local ext=()
  if [[ -n "$san" ]]; then
    printf 'subjectAltName=%s\n' "$san" >"$TMP/ca/$name.ext"
    ext=(-extfile "$TMP/ca/$name.ext")
  fi
  openssl ca -batch -config "$TMP/ca/openssl.cnf" -in "$TMP/ca/$name.csr" \
    -out "$TMP/ca/$name.pem" "${ext[@]}" >/dev/null 2>&1
  openssl x509 -in "$TMP/ca/$name.pem" -out "$TMP/ca/$name.pem" >/dev/null 2>&1
}

issue server localhost "DNS:localhost"
issue client client.test

openssl ca -config "$TMP/ca/openssl.cnf" -gencrl -out "$TMP/ca/crl.pem" >/dev/null 2>&1

PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')"
cat >"$TMP/exyonq.toml" <<EOF
config_version = 1
[[server]]
listen = "127.0.0.1:${PORT}"
routes = ["site"]
[server.tls]
cert = "$TMP/ca/server.pem"
key = "$TMP/ca/server.key"
client_auth = true
client_ca = "$TMP/ca/ca.pem"
crl = "$TMP/ca/crl.pem"
min_version = "TLSv1.3"
[[route]]
name = "site"
match = { path = "/" }
root = "$TMP/www"
EOF

start() {
  EXYONQ_CONTROL_SOCKET="$TMP/control.sock" RUST_LOG=info \
    "$EXYONQ_BIN" serve -c "$TMP/exyonq.toml" >"$TMP/ex.log" 2>&1 &
  PID=$!
  for _ in $(seq 1 100); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', $PORT)); s.close()" 2>/dev/null; then
      return 0
    fi
    kill -0 "$PID" 2>/dev/null || { echo "FAIL server exited"; cat "$TMP/ex.log"; return 1; }
    sleep 0.1
  done
  echo "FAIL server not ready"
  return 1
}
stop() {
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
  PID=""
}

code() { # extra curl args
  curl -sk -o "$TMP/out" -w '%{http_code}' --resolve "localhost:${PORT}:127.0.0.1" "$@" \
    "https://localhost:${PORT}/" 2>"$TMP/curl.err" || true
}

start || exit 1
NO_CERT="$(code)"
TLS12="$(code --tls-max 1.2 --cert "$TMP/ca/client.pem" --key "$TMP/ca/client.key" --cacert "$TMP/ca/ca.pem")"
VALID="$(code --cert "$TMP/ca/client.pem" --key "$TMP/ca/client.key" --cacert "$TMP/ca/ca.pem")"
S_CLIENT="$(openssl s_client -connect "127.0.0.1:${PORT}" -servername localhost \
  -cert "$TMP/ca/client.pem" -key "$TMP/ca/client.key" -CAfile "$TMP/ca/ca.pem" \
  -tls1_3 </dev/null 2>/dev/null | grep -c 'Verify return code: 0' || true)"
stop

openssl ca -batch -config "$TMP/ca/openssl.cnf" -revoke "$TMP/ca/client.pem" >/dev/null 2>&1
openssl ca -config "$TMP/ca/openssl.cnf" -gencrl -out "$TMP/ca/crl.pem" >/dev/null 2>&1
start || exit 1
REVOKED="$(code --cert "$TMP/ca/client.pem" --key "$TMP/ca/client.key" --cacert "$TMP/ca/ca.pem")"
stop

OK=1
[[ "$NO_CERT" == "000" ]] || { echo "FAIL no_cert=$NO_CERT"; OK=0; }
[[ "$TLS12" == "000" ]] || { echo "FAIL tls12=$TLS12"; OK=0; }
[[ "$VALID" == "200" ]] || { echo "FAIL valid=$VALID"; OK=0; }
[[ "$VALID" == "200" ]] || cat "$TMP/curl.err" >&2 || true
[[ "$VALID" == "200" ]] || cat "$TMP/ex.log" >&2 || true
[[ "$S_CLIENT" == "1" ]] || { echo "FAIL s_client_verify=$S_CLIENT"; OK=0; }
[[ "$REVOKED" == "000" ]] || { echo "FAIL revoked=$REVOKED"; OK=0; }
echo "F08_NO_CERT=$NO_CERT"
echo "F08_TLS12=$TLS12"
echo "F08_VALID=$VALID"
echo "F08_S_CLIENT_VERIFY=$S_CLIENT"
echo "F08_REVOKED=$REVOKED"
if [[ "$OK" == "1" ]]; then echo "F08=PASS"; exit 0; fi
echo "F08=FAIL"; exit 1
