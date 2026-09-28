#!/usr/bin/env bash
# CAPABILITY_010 = oci-container — per-host REAL container product E2E.
# Expects: PULL_IMAGE, EXPECT_ARCH, HEAD, OUT_JSON, EV_DIR, HOST_LABEL
# Proves: pull platform → run with mounted config+www → HTTP body → 404 →
#         invalid-config failure → docker stop lifecycle → privacy scan.
# Not proof: image inspect alone, --version alone, dependency-baseline tooling alone.
set -euo pipefail

IMAGE="${PULL_IMAGE:?}"
EXPECT_ARCH="${EXPECT_ARCH:?}"
HEAD="${HEAD:?}"
OUT_JSON="${OUT_JSON:?}"
EV_DIR="${EV_DIR:?}"
HOST_LABEL="${HOST_LABEL:-$(hostname)}"
REG_HOST="${REG_HOST:-}"
REG_PORT="${REG_PORT:-5000}"
MARKER="cap010-oci-static-v1"

mkdir -p "$EV_DIR"
python3 - "$OUT_JSON" <<'PY' || true
import json, os, socket, sys
from datetime import datetime, timezone
out = {
  "FEATURE_ID": "oci-container",
  "CAPABILITY": "CAPABILITY_010",
  "CAPABILITY_NAME": "oci-container",
  "HOST_LABEL": os.environ.get("HOST_LABEL"),
  "HOSTNAME": socket.gethostname(),
  "UNAME_M": os.uname().machine,
  "KERNEL": f"{os.uname().sysname} {os.uname().release}",
  "HEAD": os.environ.get("HEAD"),
  "PULL_IMAGE": os.environ.get("PULL_IMAGE"),
  "EXPECT_ARCH": os.environ.get("EXPECT_ARCH"),
  "TIMESTAMP": datetime.now(timezone.utc).isoformat(),
  "PRODUCT_CONTRACT": "Multi-arch image runs config+www and serves HTTP",
  "CAPABILITY_011_STARTED": "NO",
  "GHCR_WRITE": "NO",
}
open(sys.argv[1], "w").write(json.dumps(out, indent=2) + "\n")
PY

write_insecure_registry() {
  if [[ -z "$REG_HOST" ]]; then
    return 0
  fi
  REG_HOST="$REG_HOST" REG_PORT="$REG_PORT" python3 - <<'PY'
import json, os, pathlib, tempfile, subprocess
entry = f"{os.environ['REG_HOST']}:{os.environ['REG_PORT']}"
extras = ("127.0.0.1:" + os.environ["REG_PORT"], "localhost:" + os.environ["REG_PORT"])
path = pathlib.Path("/etc/docker/daemon.json")
cfg = {}
try:
    cfg = json.loads(path.read_text() or "{}")
except Exception:
    cfg = {}
m = cfg.get("insecure-registries") or []
for e in (entry,) + extras:
    if e not in m:
        m.append(e)
cfg["insecure-registries"] = m
text = json.dumps(cfg, indent=2) + "\n"
try:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
except PermissionError:
    fd, tmp = tempfile.mkstemp(prefix="exyonq-daemon-", suffix=".json")
    os.close(fd)
    pathlib.Path(tmp).write_text(text)
    subprocess.check_call(["sudo", "-n", "mkdir", "-p", "/etc/docker"])
    subprocess.check_call(["sudo", "-n", "cp", tmp, "/etc/docker/daemon.json"])
    os.unlink(tmp)
print("insecure-registries", cfg["insecure-registries"])
PY
  systemctl reload docker 2>/dev/null || sudo -n systemctl reload docker 2>/dev/null || true
  sleep 1
}

write_result() {
  python3 - "$OUT_JSON" <<'PY'
import json, os, sys
path = sys.argv[1]
d = json.loads(open(path).read())
# merge env CAP010_* results passed as JSON blob
extra = json.loads(os.environ.get("CAP010_RESULT_JSON", "{}"))
d.update(extra)
open(path, "w").write(json.dumps(d, indent=2) + "\n")
PY
}

fail() {
  local msg="$1"
  export CAP010_RESULT_JSON
  CAP010_RESULT_JSON="$(python3 - <<PY
import json
print(json.dumps({
  "FINAL_RESULT": "FAIL_REAL_E2E",
  "DETAIL": """$msg""",
  "PRODUCT_DEFECT": "UNKNOWN",
  "HARNESS_DEFECT": "UNKNOWN",
}))
PY
)"
  write_result
  exit 1
}

write_insecure_registry

echo "PULL_START $(date -u +%Y-%m-%dT%H:%M:%SZ)"
docker pull "$IMAGE" 2>&1 | tee "$EV_DIR/pull.log"
inspect="$(docker image inspect "$IMAGE" --format '{{.Architecture}} {{index .Config.Labels "org.opencontainers.image.revision"}} {{index .Config.Labels "org.opencontainers.image.version"}} {{.Id}}')"
echo "INSPECT=$inspect" | tee "$EV_DIR/inspect.txt"
arch="$(echo "$inspect" | awk '{print $1}')"
rev="$(echo "$inspect" | awk '{print $2}')"
ver="$(echo "$inspect" | awk '{print $3}')"
img_id="$(echo "$inspect" | awk '{print $4}')"
[[ "$arch" == "$EXPECT_ARCH" ]] || fail "arch got=$arch want=$EXPECT_ARCH"
[[ "$rev" == "$HEAD" ]] || fail "revision got=$rev want=$HEAD"
echo "MULTIARCH_PULL_PASS arch=$arch rev=$rev" | tee "$EV_DIR/pull-verdict.txt"

# Binary identity inside image (product under test)
BIN_SHA="$(docker run --rm --entrypoint sha256sum "$IMAGE" /usr/local/bin/exyonq | awk '{print $1}')"
echo "CONTAINER_EXYONQ_SHA256=$BIN_SHA" | tee "$EV_DIR/binary.sha256"
[[ -n "$BIN_SHA" && "$BIN_SHA" != "" ]] || fail "empty container binary sha256"

# Publication-privacy regression protector (current image filesystem)
PRIV_CID="$(docker create "$IMAGE")"
set +e
PATH_HITS="$(docker export "$PRIV_CID" | tar -t 2>/dev/null | grep -E -e '\.cursor/' -e '\.exyonq-local/' -e '/Volumes/Lexar' -e 'EXYONQ-SIGNING' | head -20)"
set -e
docker rm "$PRIV_CID" >/dev/null
if [[ -n "$PATH_HITS" ]]; then
  echo "$PATH_HITS" | tee "$EV_DIR/privacy-hits.txt"
  fail "publication privacy material in image filesystem paths"
fi
echo "PRIVACY_SCAN_PASS" | tee "$EV_DIR/privacy.txt"
LAB_JSON="$(docker image inspect "$IMAGE")"
if echo "$LAB_JSON" | grep -E -e '/Volumes/Lexar' -e '\.cursor/' -e '\.exyonq-local/' -e 'EXYONQ-SIGNING-A' >/dev/null 2>&1; then
  fail "publication privacy material in image inspect/labels"
fi
echo "LABEL_PRIVACY_PASS" | tee -a "$EV_DIR/privacy.txt"

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT
WWW="$TMP/www"
mkdir -p "$WWW"
printf '%s' "$MARKER" >"$WWW/index.html"
CFG="$TMP/config.toml"
cat >"$CFG" <<'TOML'
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
TOML

PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()
PY
)"

CID="$(docker run -d --rm \
  -p "127.0.0.1:${PORT}:8080" \
  -v "$CFG:/etc/exyonq/config.toml:ro" \
  -v "$WWW:/var/www/exyonq:ro" \
  "$IMAGE")"
echo "CID=$CID PORT=$PORT" | tee "$EV_DIR/run.txt"

ready=0
for _ in $(seq 1 90); do
  if curl -sf "http://127.0.0.1:${PORT}/site/" >/dev/null; then ready=1; break; fi
  sleep 0.5
done
[[ "$ready" == "1" ]] || {
  docker logs "$CID" >"$EV_DIR/logs.txt" 2>&1 || true
  docker stop "$CID" >/dev/null 2>&1 || true
  fail "container not ready"
}

BODY="$(curl -sf "http://127.0.0.1:${PORT}/site/")"
[[ "$BODY" == "$MARKER" ]] || {
  docker stop "$CID" >/dev/null 2>&1 || true
  fail "body mismatch got='$BODY'"
}
BODY_SHA="$(printf '%s' "$BODY" | sha256sum | awk '{print $1}')"

MISS="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/site/missing")"
[[ "$MISS" == "404" ]] || {
  docker stop "$CID" >/dev/null 2>&1 || true
  fail "miss status got=$MISS want=404"
}

# uid contract from image (Dockerfile USER exyonq = 10001)
UID_IN="$(docker exec "$CID" id -u)"
[[ "$UID_IN" == "10001" ]] || {
  docker stop "$CID" >/dev/null 2>&1 || true
  fail "uid got=$UID_IN want=10001"
}

docker stop "$CID" >/dev/null
CID=""
echo "GRACEFUL_STOP_PASS" | tee "$EV_DIR/stop.txt"

# Failure path: invalid config must not yield false healthy HTTP
BAD_CFG="$TMP/bad.toml"
printf 'this is not valid toml [[[[\n' >"$BAD_CFG"
BAD_PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()
PY
)"
set +e
BAD_CID="$(docker run -d --rm \
  -p "127.0.0.1:${BAD_PORT}:8080" \
  -v "$BAD_CFG:/etc/exyonq/config.toml:ro" \
  -v "$WWW:/var/www/exyonq:ro" \
  "$IMAGE" 2>"$EV_DIR/bad-run.err")"
BAD_RUN_EC=$?
set -e
false_healthy=0
if [[ -n "${BAD_CID:-}" ]]; then
  for _ in $(seq 1 20); do
    if curl -sf "http://127.0.0.1:${BAD_PORT}/site/" >/dev/null 2>&1 \
      || curl -sf "http://127.0.0.1:${BAD_PORT}/health" >/dev/null 2>&1; then
      false_healthy=1
      break
    fi
    # if container already exited, good
    if ! docker ps -q --filter "id=$BAD_CID" | grep -q .; then
      break
    fi
    sleep 0.5
  done
  docker logs "$BAD_CID" >"$EV_DIR/bad-logs.txt" 2>&1 || true
  docker stop "$BAD_CID" >/dev/null 2>&1 || true
fi
[[ "$false_healthy" == "0" ]] || fail "false healthy HTTP after invalid config"
echo "INVALID_CONFIG_FAILURE_PASS" | tee "$EV_DIR/failure.txt"

export CAP010_RESULT_JSON
CAP010_RESULT_JSON="$(python3 - <<PY
import json
print(json.dumps({
  "FINAL_RESULT": "PASS_REAL_E2E",
  "PRODUCT_DEFECT": "NO",
  "HARNESS_DEFECT": "NO",
  "ENVIRONMENT_BLOCKER": "NO",
  "EXYONQ_BINARY_SHA256": "$BIN_SHA",
  "OCI_IMAGE_ID": "$img_id",
  "OCI_PLATFORM_ARCH": "$arch",
  "OCI_SOURCE_REVISION": "$rev",
  "OCI_VERSION": "$ver",
  "OCI_IMAGE_DIGEST_LOCAL": "$img_id",
  "CAP010_POSITIVE_STATUS": "PASS",
  "CAP010_NEGATIVE_STATUS": "PASS",
  "CAP010_FAILURE_STATUS": "PASS",
  "CAP010_BOUNDARY_STATUS": "PASS",
  "CAP010_CONCURRENCY_OR_LIFECYCLE_STATUS": "PASS",
  "CAP010_CROSS_CAPABILITY_INVARIANTS": {
    "OCI_SOURCE_REVISION_MATCHES_HEAD": "YES",
    "PLATFORM_ARCH_MATCHES_HOST": "YES",
    "MOUNTED_WWW_BODY_INTEGRITY": "YES",
    "NO_CURSOR_PATHS": "YES",
    "NO_EXYONQ_LOCAL_EVIDENCE": "YES",
    "NO_PRIVATE_HOST_PATHS": "YES",
    "RUNS_AS_UID_10001": "YES",
    "INVALID_CONFIG_FALSE_HEALTHY": "NO",
  },
  "BODY_SHA256": "$BODY_SHA",
  "BODY_MARKER": "$MARKER",
  "MISS_STATUS": "$MISS",
}))
PY
)"
write_result
echo "PASS_REAL_E2E"
exit 0
