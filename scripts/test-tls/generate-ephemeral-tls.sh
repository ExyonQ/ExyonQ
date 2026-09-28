#!/usr/bin/env bash
# Canonical ephemeral TLS material for tests / soak / bench / e2e.
# RULE: no static private keys in the repository or build contexts.
#
# Usage:
#   eval "$(bash scripts/test-tls/generate-ephemeral-tls.sh)"
#   # exports: EXYONQ_TLS_DIR EXYONQ_TLS_CERT EXYONQ_TLS_KEY
#   # optional: --print-paths  → lines: DIR= CERT= KEY=
#
# Cleanup:
#   bash scripts/test-tls/generate-ephemeral-tls.sh --cleanup "$EXYONQ_TLS_DIR"
set -euo pipefail
LC_ALL=C
export LC_ALL
set +x

PRINT_PATHS=0
CLEANUP_DIR=""
OUT_PARENT="${TMPDIR:-/tmp}"

usage() {
  cat <<'EOF'
Usage:
  generate-ephemeral-tls.sh [--print-paths] [--tmpdir DIR]
  generate-ephemeral-tls.sh --cleanup DIR
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --print-paths) PRINT_PATHS=1; shift ;;
    --tmpdir) OUT_PARENT="$2"; shift 2 ;;
    --cleanup) CLEANUP_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [[ -n "$CLEANUP_DIR" ]]; then
  case "$CLEANUP_DIR" in
    */exyonq-tls-ephemeral-*)
      rm -rf "$CLEANUP_DIR"
      exit 0
      ;;
    *)
      echo "ERROR: refusing to cleanup non-ephemeral path: $CLEANUP_DIR" >&2
      exit 2
      ;;
  esac
fi

OPENSSL_BIN="${EXYONQ_OPENSSL_BIN:-}"
if [[ -z "$OPENSSL_BIN" ]]; then
  for cand in /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
    if [[ -x "$cand" ]]; then
      OPENSSL_BIN="$cand"
      break
    fi
  done
fi
OPENSSL_BIN="${OPENSSL_BIN:-openssl}"
command -v "$OPENSSL_BIN" >/dev/null 2>&1 || { echo "ERROR: openssl required" >&2; exit 1; }

DIR="$(mktemp -d "${OUT_PARENT%/}/exyonq-tls-ephemeral-XXXXXX")"
chmod 700 "$DIR"
CERT="$DIR/cert.pem"
KEY="$DIR/key.pem"
CFG="$DIR/openssl.cnf"

cat >"$CFG" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = v3_req
prompt = no

[dn]
CN = localhost

[v3_req]
subjectAltName = @alt_names
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth

[alt_names]
DNS.1 = localhost
IP.1 = 127.0.0.1
IP.2 = ::1
EOF

# Unique per process; safe under parallel tests.
"$OPENSSL_BIN" req -x509 -newkey rsa:2048 -nodes \
  -keyout "$KEY" \
  -out "$CERT" \
  -days 1 \
  -config "$CFG" \
  -extensions v3_req \
  >/dev/null 2>&1

chmod 0600 "$KEY"
chmod 0644 "$CERT"
rm -f "$CFG"

if [[ "$PRINT_PATHS" == "1" ]]; then
  printf 'DIR=%s\nCERT=%s\nKEY=%s\n' "$DIR" "$CERT" "$KEY"
else
  # shell eval form
  printf 'export EXYONQ_TLS_DIR=%q\n' "$DIR"
  printf 'export EXYONQ_TLS_CERT=%q\n' "$CERT"
  printf 'export EXYONQ_TLS_KEY=%q\n' "$KEY"
fi
