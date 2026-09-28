#!/usr/bin/env bash
# CAPABILITY_010 oci-container dual-Linux REAL PRODUCTION E2E — Netcup amd64 ∥ Oracle arm64.
# Canonical matrix: FEATURE_ID=oci-container
# PRODUCT_CONTRACT=Multi-arch image runs config+www and serves HTTP
# Cap009 CLOSED. Cap011 MUST NOT start. GHCR_WRITE=NO. PUSH=NO.
# Builds CURRENT Cap010 HEAD into disposable local registry (not stale pre-Cap004 image).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_REPO="${NETCUP_OCI_REPO:-/root/exyonq-phase1-cap010-oci-src}"
REG_PORT="${REG_PORT:-5000}"
RUN_ID="${EXYONQ_CAP010_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
VERSION="0.4.4-cap010-${RUN_ID}"
HEAD="$(git rev-parse HEAD)"
HEAD="$(printf '%s' "$HEAD" | tr 'A-F' 'a-f')"
TREE="$(git rev-parse 'HEAD^{tree}')"
LOCK_SHA="$(shasum -a 256 Cargo.lock | awk '{print $1}')"
TOOLCHAIN="$(awk -F'"' '/^channel/ {print $2; exit}' rust-toolchain.toml)"
EVIDENCE="$ROOT/.exyonq-local/tmp/phase1-cap010-oci-$RUN_ID"
mkdir -p "$EVIDENCE" "$ROOT/.exyonq-local/logs" "$ROOT/.exyonq-local/status"

SEALED_LOCK="0d5c78445354fdd3319a3af2ed4b152456f33be0ec631fd99a030fc8c37c930e"
SEALED_TOOLCHAIN="1.98.1"
EXPECTED_HEAD_BEFORE="0dff9542d45b4275c74a0bdd3fce172da6c81d8e"

echo "RUN_ID=$RUN_ID"
echo "HEAD=$HEAD"
echo "TREE=$TREE"
echo "CARGO_LOCK_SHA256=$LOCK_SHA"
echo "RUST_TOOLCHAIN=$TOOLCHAIN"
echo "EVIDENCE=$EVIDENCE"
echo "CAPABILITY_010=oci-container"
echo "CAPABILITY_011_STARTED=NO"
echo "CAP010_HEAD_BEFORE=$EXPECTED_HEAD_BEFORE"
echo "CURRENT_HEAD=$HEAD"
echo "GHCR_WRITE=NO"
echo "OCI_VERSION=$VERSION"

if [[ "$LOCK_SHA" != "$SEALED_LOCK" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (Cargo.lock)"; exit 2
fi
if [[ "$TOOLCHAIN" != "$SEALED_TOOLCHAIN" ]]; then
  echo "DEPENDENCY_BASELINE_DRIFT=YES (rust-toolchain)"; exit 2
fi
if [[ "$HEAD" != "$EXPECTED_HEAD_BEFORE" ]]; then
  echo "NOTE: HEAD differs from CAP010_HEAD_BEFORE (harness commits may land after close)"
fi

cat >"$EVIDENCE/run_meta.txt" <<EOF
CAPABILITY_010=oci-container
CAPABILITY_NAME=oci-container
PRODUCT_CONTRACT=Multi-arch image runs config+www and serves HTTP
CAP010_BUILD_HEAD=$HEAD
OCI_SOURCE_REVISION=$HEAD
OCI_VERSION=$VERSION
GHCR_WRITE=NO
CAPABILITY_011_STARTED=NO
CAP010_CARGO_LOCK_SHA256=$LOCK_SHA
CAP010_RUST_TOOLCHAIN=$TOOLCHAIN
DEPENDENCY_BASELINE_DRIFT=NO
STARTED_UTC=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

ssh_n() { ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$NETCUP_HOST" "$@"; }
ssh_o() { ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$ORACLE_HOST" "$@"; }

echo "== ensure legal bundle =="
if [[ ! -s "$ROOT/THIRD_PARTY_NOTICES.md" || ! -s "$ROOT/sbom.cdx.json" ]]; then
  echo "FAIL: need THIRD_PARTY_NOTICES.md + sbom.cdx.json in build context" >&2
  exit 3
fi

echo "== sync Cap010 HEAD tree to Netcup =="
ssh_n "mkdir -p '$NETCUP_REPO'"
rsync -az --delete \
  --exclude target/ --exclude .exyonq-local/ --exclude benchmarks/ \
  --exclude .git/ --exclude study/ --exclude graphify-out/ \
  --exclude 'tools/' \
  -e "ssh -o BatchMode=yes -o ConnectTimeout=30" \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_REPO}/"
# Cap010 harness must be present on Netcup for local copy to Oracle via rsync of scripts only — also sync harness to both via separate path in e2e.
ssh_n "test -s '$NETCUP_REPO/THIRD_PARTY_NOTICES.md' && test -s '$NETCUP_REPO/sbom.cdx.json' && \
  test -f '$NETCUP_REPO/packaging/docker/Dockerfile' && echo REMOTE_CONTEXT_OK" | tee -a "$EVIDENCE/run_meta.txt"

# Push Cap010 e2e script explicitly (also in rsync)
scp -o BatchMode=yes \
  "$ROOT/scripts/reality/cap-010-oci-container-e2e.sh" \
  "${NETCUP_HOST}:/tmp/cap-010-oci-container-e2e.sh"
scp -o BatchMode=yes \
  "$ROOT/scripts/reality/cap-010-oci-container-e2e.sh" \
  "${ORACLE_HOST}:/tmp/cap-010-oci-container-e2e.sh"
ssh_n "chmod +x /tmp/cap-010-oci-container-e2e.sh"
ssh_o "chmod +x /tmp/cap-010-oci-container-e2e.sh"

echo "== disposable registry on Netcup :${REG_PORT} =="
ssh_n "docker rm -f exyonq-cap010-oci-reg 2>/dev/null || true
docker run -d --name exyonq-cap010-oci-reg --restart=no \
  -p ${REG_PORT}:5000 registry:2
mkdir -p /etc/docker
python3 - <<'PY'
import json, pathlib
p=pathlib.Path('/etc/docker/daemon.json')
cfg={}
if p.exists():
  try: cfg=json.loads(p.read_text() or '{}')
  except Exception: cfg={}
mirrors=cfg.get('insecure-registries') or []
host='127.0.0.1:${REG_PORT}'
if host not in mirrors:
  mirrors.append(host)
cfg['insecure-registries']=mirrors
p.write_text(json.dumps(cfg, indent=2)+'\n')
print('insecure-registries', mirrors)
PY
systemctl reload docker 2>/dev/null || true
sleep 2
curl -fsS http://127.0.0.1:${REG_PORT}/v2/ >/dev/null
echo REGISTRY_UP" | tee "$EVIDENCE/registry-up.log"

IMAGE="127.0.0.1:${REG_PORT}/exyonq/exyonq:${VERSION}"
echo "IMAGE=$IMAGE" | tee -a "$EVIDENCE/run_meta.txt"
echo "CAP010_BUILD_HEAD=$HEAD" | tee -a "$EVIDENCE/run_meta.txt"

echo "== multiarch build+push linux/amd64,linux/arm64 @ $HEAD =="
ssh_n "cd '$NETCUP_REPO' && \
  mkdir -p /etc/buildkit && cat >/etc/buildkit/buildkitd.toml <<'BK'
[registry.\"127.0.0.1:${REG_PORT}\"]
  http = true
  insecure = true
BK
  if ! docker buildx inspect cap010oci >/dev/null 2>&1; then
    docker buildx create --name cap010oci --driver docker-container \
      --driver-opt network=host \
      --buildkitd-flags '--allow-insecure-entitlement network.host' \
      --config /etc/buildkit/buildkitd.toml \
      --use
  else
    docker buildx use cap010oci
  fi
  docker buildx inspect --bootstrap >/dev/null
  docker buildx build -f packaging/docker/Dockerfile \
    --platform linux/amd64,linux/arm64 \
    --provenance=mode=max \
    --sbom=true \
    --push \
    --build-arg EXYONQ_VERSION=${VERSION} \
    --build-arg EXYONQ_GIT_REVISION=${HEAD} \
    --build-arg EXYONQ_OFFICIAL_RELEASE=1 \
    --label org.opencontainers.image.title=ExyonQ \
    --label org.opencontainers.image.version=${VERSION} \
    --label org.opencontainers.image.revision=${HEAD} \
    --label org.opencontainers.image.source=https://github.com/ExyonQ/ExyonQ \
    --label org.opencontainers.image.licenses=Apache-2.0 \
    -t '${IMAGE}' \
    ." 2>&1 | tee "$EVIDENCE/buildx-push.log"

echo "== inspect index =="
ssh_n "docker buildx imagetools inspect '${IMAGE}' --raw" >"$EVIDENCE/index.raw.json"
ssh_n "docker buildx imagetools inspect '${IMAGE}'" | tee "$EVIDENCE/imagetools.txt"
INDEX_DIGEST="$(python3 - <<'PY'
import json
from pathlib import Path
# digest of index is not always in raw; parse imagetools or compute from registry
print("see-imagetools")
PY
)"
# Prefer registry HEAD for index digest
OCI_INDEX_DIGEST="$(ssh_n "curl -sI -H 'Accept: application/vnd.oci.image.index.v1+json' http://127.0.0.1:${REG_PORT}/v2/exyonq/exyonq/manifests/${VERSION} | tr -d '\\r' | awk -F': ' 'tolower(\$1)==\"docker-content-digest\"{print \$2}'")"
echo "OCI_INDEX_DIGEST=$OCI_INDEX_DIGEST" | tee -a "$EVIDENCE/run_meta.txt"

python3 - "$EVIDENCE/index.raw.json" "$HEAD" <<'PY'
import json, sys
idx = json.loads(open(sys.argv[1]).read())
head = sys.argv[2]
descs = idx.get("manifests") or []
has_amd64 = has_arm64 = False
for d in descs:
    plat = d.get("platform") or {}
    if plat.get("os") == "linux" and plat.get("architecture") == "amd64":
        has_amd64 = True
    if plat.get("os") == "linux" and plat.get("architecture") == "arm64":
        has_arm64 = True
print(json.dumps({"HAS_LINUX_AMD64": has_amd64, "HAS_LINUX_ARM64": has_arm64, "DESCRIPTOR_COUNT": len(descs)}, indent=2))
if not (has_amd64 and has_arm64):
    raise SystemExit("FAIL: missing linux/amd64 or linux/arm64")
print("INDEX_STRUCTURE_OK")
PY

NETCUP_IP="$(ssh_n 'curl -4 -s ifconfig.me')"
ORACLE_IP="$(ssh_o 'curl -4 -s ifconfig.me')"
echo "NETCUP_IP=$NETCUP_IP" | tee -a "$EVIDENCE/run_meta.txt"
echo "ORACLE_IP=$ORACLE_IP" | tee -a "$EVIDENCE/run_meta.txt"
ssh_n "iptables -C INPUT -p tcp --dport ${REG_PORT} -s ${ORACLE_IP} -j ACCEPT 2>/dev/null || \
  iptables -I INPUT -p tcp --dport ${REG_PORT} -s ${ORACLE_IP} -j ACCEPT || true"

e2e_remote() {
  local host="$1" label="$2" expect_arch="$3" pull_image="$4" reg_host="$5"
  local remote_json="/tmp/cap010-${RUN_ID}-${label}.json"
  local remote_ev="/tmp/cap010-${RUN_ID}-${label}-ev"
  local log="$EVIDENCE/${label}.log"
  echo "[$(date -u +%H:%M:%S)] e2e $label on $host image=$pull_image"
  set +e
  ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$host" \
    env PULL_IMAGE="$pull_image" EXPECT_ARCH="$expect_arch" HEAD="$HEAD" \
        OUT_JSON="$remote_json" EV_DIR="$remote_ev" HOST_LABEL="$host" \
        REG_HOST="$reg_host" REG_PORT="$REG_PORT" \
    bash /tmp/cap-010-oci-container-e2e.sh >"$log" 2>&1
  local rc=$?
  set -e
  echo "[$(date -u +%H:%M:%S)] $label ssh exit=$rc"
  scp -o BatchMode=yes -q "${host}:${remote_json}" "$EVIDENCE/${label}.json" || true
  echo "$rc" >"$EVIDENCE/${label}.exit"
  return 0
}

e2e_remote "$NETCUP_HOST" "amd64" "amd64" "127.0.0.1:${REG_PORT}/exyonq/exyonq:${VERSION}" "127.0.0.1" &
pid_amd=$!
e2e_remote "$ORACLE_HOST" "arm64" "arm64" "${NETCUP_IP}:${REG_PORT}/exyonq/exyonq:${VERSION}" "$NETCUP_IP" &
pid_arm=$!

set +e
wait "$pid_amd"; rc_amd=$?
wait "$pid_arm"; rc_arm=$?
set -e
echo "wait_amd=$rc_amd wait_arm=$rc_arm"

python3 - "$EVIDENCE" "$RUN_ID" "$HEAD" "$TREE" "$LOCK_SHA" "$TOOLCHAIN" "$VERSION" "${OCI_INDEX_DIGEST:-}" <<'PY'
import json, sys
from pathlib import Path
ev = Path(sys.argv[1])
run_id, head, tree, lock_sha, toolchain, version, index_digest = sys.argv[2:9]

def load(label):
    p = ev / f"{label}.json"
    if not p.exists():
        return {"FINAL_RESULT": "MISSING_EVIDENCE", "label": label}
    d = json.loads(p.read_text()); d["label"] = label; return d

amd, arm = load("amd64"), load("arm64")
amd_pass = amd.get("FINAL_RESULT") == "PASS_REAL_E2E"
arm_pass = arm.get("FINAL_RESULT") == "PASS_REAL_E2E"
closed = amd_pass and arm_pass

def both(key):
    return {"amd64": amd.get(key), "arm64": arm.get(key)}

summary = {
    "CAPABILITY_ID": "010",
    "CAPABILITY_010": "oci-container",
    "CAPABILITY_NAME": "oci-container",
    "PRODUCT_CONTRACT": "Multi-arch image runs config+www and serves HTTP",
    "SUPPORTED_BEHAVIOR": "multiarch OCI @ Cap010 HEAD; pull+run; mounted config+www HTTP body; miss 404; invalid-config failure; stop lifecycle; privacy protector",
    "EXPLICIT_NON_SCOPE": [
        "Dependency-baseline OCI tooling alone as Cap010 product close",
        "GHCR write",
        "Cap011 reverse-proxy",
        "version-only / inspect-only proof",
        "stale pre-Cap004 OCI image",
    ],
    "CAPABILITY_011_STARTED": "NO",
    "GHCR_WRITE": "NO",
    "OPEN_DEFECT_CONTEXT": "RD-009",
    "RUN_ID": run_id,
    "HEAD": head,
    "TREE": tree,
    "CAP010_BUILD_HEAD": head,
    "OCI_SOURCE_REVISION": head,
    "OCI_VERSION": version,
    "OCI_INDEX_DIGEST": index_digest or None,
    "CAP010_CARGO_LOCK_SHA256": lock_sha,
    "CAP010_RUST_TOOLCHAIN": toolchain,
    "DEPENDENCY_BASELINE_DRIFT": "NO",
    "NETCUP_AMD64_BINARY_SHA256": amd.get("EXYONQ_BINARY_SHA256"),
    "ORACLE_ARM64_BINARY_SHA256": arm.get("EXYONQ_BINARY_SHA256"),
    "CAP010_NETCUP_AMD64": "PASS_REAL_PRODUCTION" if amd_pass else "FAIL",
    "CAP010_ORACLE_ARM64": "PASS_REAL_PRODUCTION" if arm_pass else "FAIL",
    "NETCUP_AMD64_PULL": "PASS" if amd_pass else "FAIL",
    "ORACLE_ARM64_PULL": "PASS" if arm_pass else "FAIL",
    "CAP010_POSITIVE_STATUS": both("CAP010_POSITIVE_STATUS"),
    "CAP010_NEGATIVE_STATUS": both("CAP010_NEGATIVE_STATUS"),
    "CAP010_FAILURE_STATUS": both("CAP010_FAILURE_STATUS"),
    "CAP010_BOUNDARY_STATUS": both("CAP010_BOUNDARY_STATUS"),
    "CAP010_CONCURRENCY_OR_LIFECYCLE_STATUS": both("CAP010_CONCURRENCY_OR_LIFECYCLE_STATUS"),
    "CAP010_CROSS_CAPABILITY_INVARIANTS": both("CAP010_CROSS_CAPABILITY_INVARIANTS"),
    "PRODUCT_DEFECT": "YES" if (amd.get("PRODUCT_DEFECT") == "YES" or arm.get("PRODUCT_DEFECT") == "YES") else "NO",
    "CAPABILITY_010_V044_FINAL_STATUS": "VERIFIED_REAL_PRODUCTION" if closed else "NOT_VERIFIED",
    "NEXT_CAPABILITY": "011",
    "amd64": amd,
    "arm64": arm,
}
(ev / "SUMMARY.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k: summary[k] for k in summary if k not in ("amd64", "arm64")}, indent=2))
sys.exit(0 if closed else 1)
PY
