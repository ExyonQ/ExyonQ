# Shared TLS e2e helpers (ephemeral material only).
# shellcheck shell=bash

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$ROOT"

EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}"
EXYONQCTL_BIN="${EXYONQCTL_BIN:-$ROOT/target/debug/exyonqctl}"
GEN_TLS="$ROOT/scripts/test-tls/generate-ephemeral-tls.sh"

# Populated by ensure_ephemeral_tls
EXYONQ_TLS_DIR="${EXYONQ_TLS_DIR:-}"
CERT_PEM="${CERT_PEM:-}"
KEY_PEM="${KEY_PEM:-}"

pick_port() {
  python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()'
}

ensure_ephemeral_tls() {
  [[ -x "$GEN_TLS" || -f "$GEN_TLS" ]] || { echo "FAIL: missing $GEN_TLS"; return 1; }
  if [[ -n "${EXYONQ_TLS_DIR:-}" && -f "${EXYONQ_TLS_KEY:-}" && -f "${EXYONQ_TLS_CERT:-}" ]]; then
    CERT_PEM="$EXYONQ_TLS_CERT"
    KEY_PEM="$EXYONQ_TLS_KEY"
    return 0
  fi
  # shellcheck disable=SC1090
  eval "$(bash "$GEN_TLS")"
  CERT_PEM="$EXYONQ_TLS_CERT"
  KEY_PEM="$EXYONQ_TLS_KEY"
  # Best-effort cleanup on shell exit
  # shellcheck disable=SC2064
  trap "bash '$GEN_TLS' --cleanup '$EXYONQ_TLS_DIR' >/dev/null 2>&1 || true" EXIT
}

ensure_bins() {
  if [[ ! -x "$EXYONQ_BIN" ]]; then
    cargo build -q -p exyonq --bin exyonq
  fi
  if [[ ! -x "$EXYONQCTL_BIN" ]]; then
    cargo build -q -p exyonqctl --bin exyonqctl 2>/dev/null \
      || cargo build -q -p exyonq --bin exyonqctl 2>/dev/null \
      || true
  fi
  [[ -x "$EXYONQ_BIN" ]] || { echo "FAIL: missing $EXYONQ_BIN"; return 1; }
}

wait_listen() {
  local port="$1"
  local i
  for i in $(seq 1 80); do
    if python3 -c "import socket; s=socket.socket(); s.settimeout(0.2); s.connect(('127.0.0.1', int('$port'))); s.close()" 2>/dev/null; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

write_tls_config() {
  local cfg="$1"
  local port="$2"
  ensure_ephemeral_tls
  local cert="${3:-$CERT_PEM}"
  local key="${4:-$KEY_PEM}"
  cat >"$cfg" <<EOF
config_version = 1

[[server]]
listen = "127.0.0.1:${port}"
routes = ["site"]
tls = { cert = "${cert}", key = "${key}" }

[[route]]
name = "site"
match = { path = "/site" }
root = "benchmarks/scenarios/payloads/www"
index = "index.html"
EOF
}

start_exyonq_tls() {
  local cfg="$1"
  local ctrl="$2"
  local log="$3"
  : >"$log"
  EXYONQ_CONFIG="$cfg" EXYONQ_CONTROL_SOCKET="$ctrl" RUST_LOG=error \
    "$EXYONQ_BIN" serve -c "$cfg" >"$log" 2>&1 &
  echo $!
}
