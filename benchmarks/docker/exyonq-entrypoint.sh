#!/bin/sh
set -eu
CONFIG="${EXYONQ_CONFIG:-/bench/bench.toml}"
# Ephemeral TLS for P11/P13 configs. Never COPY private keys into the image.
mkdir -p /bench/tls
if [ ! -f /bench/tls/key.pem ] || [ ! -f /bench/tls/cert.pem ]; then
  eval "$(generate-ephemeral-tls.sh --tmpdir /tmp)"
  cp "$EXYONQ_TLS_CERT" /bench/tls/cert.pem
  cp "$EXYONQ_TLS_KEY" /bench/tls/key.pem
  generate-ephemeral-tls.sh --cleanup "$EXYONQ_TLS_DIR"
fi
exec /bench/exyonq-env.sh exyonq serve -c "$CONFIG"
