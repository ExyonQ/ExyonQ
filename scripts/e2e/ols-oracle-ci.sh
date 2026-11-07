#!/usr/bin/env bash
# Start the OL-03 OpenLiteSpeed oracle from an immutable digest and run the
# live comparison. The image reference is intentionally not a tag.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OLS_IMAGE_DIGEST="sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504"
OLS_IMAGE="${OLS_IMAGE:-litespeedtech/openlitespeed@$OLS_IMAGE_DIGEST}"
OLS_CONTAINER="${OLS_CONTAINER:-eq-ols-oracle-ci}"
OLS_PORT="${OLS_PORT:-19080}"
EXY_PORT="${EXY_PORT:-19390}"
OLS_RUN_DIR="${OL03_OLS_RUN_DIR:-${OLS_RUN_DIR:-$ROOT/.exyonq-local/evidence/ol03/ols-compare-ci-$(date -u +%Y%m%dT%H%M%SZ)}}"
FIXTURE_CONFIG="$ROOT/scripts/e2e/fixtures/ols-oracle/httpd_config.conf"

fail() {
  echo "FAIL OLS oracle: $*" >&2
  exit 1
}

case "$OLS_IMAGE" in
  *"@$OLS_IMAGE_DIGEST") ;;
  *) fail "image must be pinned to $OLS_IMAGE_DIGEST (got $OLS_IMAGE)" ;;
esac
[[ -f "$FIXTURE_CONFIG" ]] || fail "missing fixture config: $FIXTURE_CONFIG"

TMP="$(mktemp -d)"
cleanup() {
  docker rm -f "$OLS_CONTAINER" >/dev/null 2>&1 || true
  rm -rf "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/html"
python3 - "$TMP/html/big256.bin" <<'PY'
import pathlib
import sys
pathlib.Path(sys.argv[1]).write_bytes(b"y" * 262144)
PY

echo "ols_image=$OLS_IMAGE"
echo "ols_digest=$OLS_IMAGE_DIGEST"
echo "ols_container=$OLS_CONTAINER"
echo "ols_port=$OLS_PORT"
echo "ols_run_dir=$OLS_RUN_DIR"

docker pull "$OLS_IMAGE"
docker image inspect "$OLS_IMAGE" --format '{{range .RepoDigests}}{{println .}}{{end}}' |
  grep -Fq "@$OLS_IMAGE_DIGEST" ||
  fail "pulled image does not report digest $OLS_IMAGE_DIGEST"

docker rm -f "$OLS_CONTAINER" >/dev/null 2>&1 || true
docker run -d \
  --name "$OLS_CONTAINER" \
  --publish "127.0.0.1:${OLS_PORT}:8088" \
  --volume "$FIXTURE_CONFIG:/usr/local/lsws/conf/httpd_config.conf:ro" \
  --volume "$TMP/html:/usr/local/lsws/Example/html:ro" \
  "$OLS_IMAGE" >/dev/null

running="$(docker inspect --format '{{.State.Running}}' "$OLS_CONTAINER" 2>/dev/null || true)"
if [[ "$running" != "true" ]]; then
  docker logs "$OLS_CONTAINER" >&2 || true
  fail "container $OLS_CONTAINER did not stay running"
fi

configured="$(docker inspect --format '{{.Config.Image}}' "$OLS_CONTAINER")"
[[ "$configured" == *"@$OLS_IMAGE_DIGEST" ]] ||
  fail "container Config.Image is not digest-pinned: $configured"

OL03_OLS_RUN_DIR="$OLS_RUN_DIR" \
OLS_CONTAINER="$OLS_CONTAINER" \
OLS_PORT="$OLS_PORT" \
EXY_PORT="$EXY_PORT" \
EXYONQ_BIN="${EXYONQ_BIN:-$ROOT/target/debug/exyonq}" \
OLS_IMAGE="$OLS_IMAGE" \
OLS_IMAGE_DIGEST="$OLS_IMAGE_DIGEST" \
bash "$ROOT/scripts/e2e/ol03-ols-compare.sh"
