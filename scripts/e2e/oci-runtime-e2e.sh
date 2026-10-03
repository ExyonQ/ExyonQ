#!/usr/bin/env bash
# AUTHORITATIVE OCI runtime E2E — not version-only.
# Real image → mounted config/www → HTTP GET body → stop signal.
set -euo pipefail
# shellcheck source=lib.sh
source "$(cd "$(dirname "$0")" && pwd)/lib.sh"
e2e_require_linux

if ! command -v docker >/dev/null 2>&1; then
  echo "FAIL: docker required for oci-runtime-e2e"
  exit 1
fi

ROOT="$E2E_ROOT"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) PLAT=linux/amd64; TAG=exyonq/exyonq:ns5-e2e-amd64 ;;
  aarch64|arm64) PLAT=linux/arm64; TAG=exyonq/exyonq:ns5-e2e-arm64 ;;
  *) echo "FAIL: unsupported arch $ARCH"; exit 1 ;;
esac

# Soak hosts may have incomplete .git (missing objects/); do not require git.
REV="${EXYONQ_GIT_REVISION:-$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)}"
[[ -n "$REV" ]] || REV="unknown"
EV="${E2E_EV:-$ROOT/.exyonq-local/tmp/e2e-oci/oci-${ARCH}}"
mkdir -p "$EV"
TMP="$(mktemp -d)"
WWW="$TMP/www"
mkdir -p "$WWW"
printf 'oci-e2e-static' >"$WWW/index.html"
CFG="$TMP/config.toml"
cat >"$CFG" <<EOF
config_version = 1
[[server]]
listen = "0.0.0.0:8080"
routes = ["site"]
[[route]]
name = "site"
match = { path = "/site" }
root = "/var/www/exyonq"
index = "index.html"
[modules.metrics]
enabled = true
health_path = "/health"
EOF

HOST_PORT="$(e2e_pick_port)"
CID=""
cleanup() {
  [[ -n "${CID:-}" ]] && docker stop "$CID" >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

echo "oci-runtime-e2e: building $TAG plat=$PLAT rev=$REV"
set +e
docker buildx build -f "$ROOT/packaging/docker/Dockerfile" \
  --platform "$PLAT" --load \
  --build-arg EXYONQ_VERSION=0.4.4.1 \
  --build-arg "EXYONQ_GIT_REVISION=$REV" \
  --build-arg EXYONQ_OFFICIAL_RELEASE=0 \
  -t "$TAG" "$ROOT" >"$EV/build.log" 2>&1
BUILD_EC=$?
set -e
[[ "$BUILD_EC" -eq 0 ]] || { echo "FAIL: docker build"; tail -40 "$EV/build.log"; exit 1; }
e2e_record_pass "image build"

CID="$(docker run -d --rm \
  -p "127.0.0.1:${HOST_PORT}:8080" \
  -v "$CFG:/etc/exyonq/config.toml:ro" \
  -v "$WWW:/var/www/exyonq:ro" \
  "$TAG")"
echo "CID=$CID" | tee "$EV/cid.txt"

ready=0
for _ in $(seq 1 60); do
  if curl -sf "http://127.0.0.1:${HOST_PORT}/health" >/dev/null 2>&1 \
    || curl -sf "http://127.0.0.1:${HOST_PORT}/site/" >/dev/null 2>&1; then
    ready=1
    break
  fi
  sleep 0.5
done
[[ "$ready" == "1" ]] || { docker logs "$CID" >"$EV/logs.txt" 2>&1 || true; e2e_record_fail "container not ready"; e2e_finish "oci-runtime-e2e"; exit 1; }
e2e_record_pass "container accepts HTTP"

BODY="$(curl -sf "http://127.0.0.1:${HOST_PORT}/site/")"
[[ "$BODY" == "oci-e2e-static" ]] && e2e_record_pass "static body via mounted www" || e2e_record_fail "body='$BODY'"

MISS="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${HOST_PORT}/site/missing")"
[[ "$MISS" == "404" ]] && e2e_record_pass "missing 404 in container" || e2e_record_fail "missing=$MISS"

VER="$(docker exec "$CID" exyonq --version 2>/dev/null || true)"
echo "VERSION=$VER" | tee "$EV/version.txt"
[[ -n "$VER" ]] && e2e_record_pass "exyonq --version in container" || e2e_record_fail "version empty"

USER="$(docker exec "$CID" id -u 2>/dev/null || true)"
[[ "$USER" == "10001" ]] && e2e_record_pass "runs as uid 10001" || e2e_record_fail "uid=$USER"

docker stop "$CID" >/dev/null
CID=""
e2e_record_pass "graceful docker stop"

e2e_finish "oci-runtime-e2e" | tee "$EV/summary.txt"
