#!/usr/bin/env bash
# V044_DEP_BASELINE_OCI_001 — real disposable-registry multiarch validation.
# GHCR_WRITE=NO. Mirrors release.yml docker contract without production mutation.
set -euo pipefail
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
# Ensure host tooling visible under nohup/non-interactive shells.
export PATH="${HOME}/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:${PATH:-}"

NETCUP_HOST="${NETCUP_HOST:-netcup-bench}"
ORACLE_HOST="${ORACLE_HOST:-oracle-quasar}"
NETCUP_REPO="${NETCUP_REPO:-/root/exyonq-dev-soak-src}"
REG_PORT="${REG_PORT:-5000}"
VERSION="${OCI_VALIDATE_VERSION:-0.4.4-oci-baseline}"
EV_ROOT="${EV_ROOT:-$ROOT/.exyonq-local/tmp/v044-oci-baseline-$(date -u +%Y%m%dT%H%M%SZ)}"
mkdir -p "$EV_ROOT"

HEAD="$(git -C "$ROOT" rev-parse HEAD)"
HEAD="$(printf '%s' "$HEAD" | tr 'A-F' 'a-f')"
echo "OCI_BUILD_HEAD=$HEAD" | tee "$EV_ROOT/meta.txt"
echo "VERSION=$VERSION" | tee -a "$EV_ROOT/meta.txt"
echo "GHCR_WRITE=NO" | tee -a "$EV_ROOT/meta.txt"
SKIP_BUILD="${OCI_SKIP_BUILD:-0}"

ssh_n() { ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$NETCUP_HOST" "$@"; }
ssh_o() { ssh -o BatchMode=yes -o ConnectTimeout=30 -o ServerAliveInterval=30 "$ORACLE_HOST" "$@"; }

if [[ "$SKIP_BUILD" != "1" ]]; then
echo "== ensure legal bundle in build context =="
# Netcup soak host may lack cargo-about/cargo-cyclonedx. Prefer existing host artifacts
# (Dockerfile only needs the files present). Regenerate only when forced.
if [[ -f "$ROOT/THIRD_PARTY_NOTICES.md" && -f "$ROOT/sbom.cdx.json" && "${OCI_FORCE_COMPLIANCE_REGEN:-0}" != "1" ]]; then
  echo "COMPLIANCE_REUSE_EXISTING_HOST_ARTIFACTS" | tee "$EV_ROOT/compliance.log"
elif command -v cargo-about >/dev/null && command -v cargo-cyclonedx >/dev/null; then
  bash "$ROOT/scripts/legal/generate-release-compliance-artifacts.sh" | tee "$EV_ROOT/compliance.log"
else
  echo "FAIL: need THIRD_PARTY_NOTICES.md + sbom.cdx.json (or cargo-about+cargo-cyclonedx)" >&2
  exit 1
fi

echo "== sync tree to Netcup @ $HEAD =="
ssh_n "mkdir -p '$NETCUP_REPO'"
# Sync only what the OCI Dockerfile COPY needs + packaging/scripts/legal (avoid vendored bench trees).
rsync -az --delete \
  --exclude target/ --exclude .exyonq-local/ --exclude benchmarks/ \
  --exclude .git/ --exclude study/ --exclude graphify-out/ \
  --exclude 'tools/' \
  -e "ssh -o BatchMode=yes -o ConnectTimeout=30" \
  "$ROOT/" "${NETCUP_HOST}:${NETCUP_REPO}/"
ssh_n "test -s '$NETCUP_REPO/THIRD_PARTY_NOTICES.md' && test -s '$NETCUP_REPO/sbom.cdx.json' && \
  test -f '$NETCUP_REPO/packaging/docker/Dockerfile' && \
  echo REMOTE_CONTEXT_OK" | tee -a "$EV_ROOT/meta.txt"

echo "== start disposable registry on Netcup :${REG_PORT} =="
ssh_n "docker rm -f exyonq-v044-oci-reg 2>/dev/null || true
docker run -d --name exyonq-v044-oci-reg --restart=no \
  -p ${REG_PORT}:5000 registry:2
# insecure local registry for disposable validation only
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
# reload docker config if possible (best-effort; may already allow localhost)
systemctl reload docker 2>/dev/null || true
sleep 2
curl -fsS http://127.0.0.1:${REG_PORT}/v2/ >/dev/null
echo REGISTRY_UP" | tee "$EV_ROOT/registry-up.log"

IMAGE="127.0.0.1:${REG_PORT}/exyonq/exyonq:${VERSION}"
echo "IMAGE=$IMAGE" | tee -a "$EV_ROOT/meta.txt"

echo "== multiarch build+push (mirrors release.yml docker contract) =="
# Long QEMU arm64 compile — do not swallow failures.
# Buildx container driver needs explicit HTTP/insecure for disposable localhost registry.
ssh_n "cd '$NETCUP_REPO' && \
  mkdir -p /etc/buildkit && cat >/etc/buildkit/buildkitd.toml <<'BK'
[registry.\"127.0.0.1:${REG_PORT}\"]
  http = true
  insecure = true
BK
  # Reuse builder when present to retain BuildKit cache across remediation rebuilds.
  if ! docker buildx inspect v044oci >/dev/null 2>&1; then
    docker buildx create --name v044oci --driver docker-container \
      --driver-opt network=host \
      --buildkitd-flags '--allow-insecure-entitlement network.host' \
      --config /etc/buildkit/buildkitd.toml \
      --use
  else
    docker buildx use v044oci
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
    ." 2>&1 | tee "$EV_ROOT/buildx-push.log"
else
  echo "OCI_SKIP_BUILD=1 — reusing registry image; HEAD must match image revision labels" | tee -a "$EV_ROOT/meta.txt"
  IMAGE="127.0.0.1:${REG_PORT}/exyonq/exyonq:${VERSION}"
  echo "IMAGE=$IMAGE" | tee -a "$EV_ROOT/meta.txt"
  ssh_n "curl -fsS http://127.0.0.1:${REG_PORT}/v2/ >/dev/null && echo REGISTRY_STILL_UP" | tee -a "$EV_ROOT/meta.txt"
fi

echo "== inspect index =="
ssh_n "docker buildx imagetools inspect '${IMAGE}' --raw" >"$EV_ROOT/index.raw.json"
ssh_n "docker buildx imagetools inspect '${IMAGE}'" | tee "$EV_ROOT/imagetools.txt"

python3 - "$EV_ROOT/index.raw.json" "$EV_ROOT/descriptor-table.json" "$HEAD" <<'PY'
import json, sys
raw_path, out_path, expect_rev = sys.argv[1:4]
idx = json.loads(open(raw_path).read())
descs = idx.get("manifests") or []
table = []
has_amd64 = has_arm64 = False
prov = sbom = 0
for d in descs:
    plat = d.get("platform") or {}
    os_ = plat.get("os")
    arch = plat.get("architecture")
    ann = d.get("annotations") or {}
    kind = ann.get("vnd.docker.reference.type") or ann.get("org.opencontainers.image.ref.name") or ""
    ref_type = ann.get("vnd.docker.reference.type", "")
    media = d.get("mediaType", "")
    row = {
        "DIGEST": d.get("digest"),
        "MEDIA_TYPE": media,
        "PLATFORM_OS": os_,
        "PLATFORM_ARCH": arch,
        "ANNOTATIONS": ann,
        "ATTESTATION_KIND": ref_type or ("attestation" if "attestation" in media or os_ == "unknown" else "image"),
    }
    table.append(row)
    if os_ == "linux" and arch == "amd64":
        has_amd64 = True
    if os_ == "linux" and arch == "arm64":
        has_arm64 = True
    # BuildKit attestation descriptors typically use platform unknown/unknown
    # and annotation vnd.docker.reference.type=attestation-manifest
    if ref_type == "attestation-manifest" or (os_ == "unknown" and arch == "unknown"):
        # classify later via subject inspect; count as attestation slot
        if "sbom" in json.dumps(ann).lower():
            sbom += 1
        else:
            prov += 1

report = {
    "OCI_INDEX_MEDIA_TYPE": idx.get("mediaType"),
    "OCI_INDEX_DESCRIPTOR_COUNT": len(descs),
    "HAS_LINUX_AMD64": has_amd64,
    "HAS_LINUX_ARM64": has_arm64,
    "OCI_INDEX_DESCRIPTOR_TABLE": table,
    "EXPECTED_REVISION": expect_rev,
}
open(out_path, "w").write(json.dumps(report, indent=2) + "\n")
print(json.dumps({k: report[k] for k in report if k != "OCI_INDEX_DESCRIPTOR_TABLE"}, indent=2))
if not (has_amd64 and has_arm64):
    raise SystemExit("FAIL: missing linux/amd64 or linux/arm64 image descriptor")
if len(descs) < 3:
    raise SystemExit("FAIL: expected image platforms + attestation descriptors")
print("INDEX_STRUCTURE_OK")
PY

NETCUP_IP="$(ssh_n 'curl -4 -s ifconfig.me')"
echo "NETCUP_IP=$NETCUP_IP" | tee -a "$EV_ROOT/meta.txt"

e2e_remote() {
  local host="$1" label="$2" expect_arch="$3" pull_image="$4"
  local remote_ev="/tmp/v044-oci-e2e-${label}"
  local registry_host="${pull_image%%:*}"
  echo "== $label multiarch pull + real E2E (expect $expect_arch) image=$pull_image =="
  # shellcheck disable=SC2087
  ssh -o BatchMode=yes -o ConnectTimeout=30 "$host" \
    env PULL_IMAGE="$pull_image" EXPECT_ARCH="$expect_arch" HEAD="$HEAD" EV="$remote_ev" REG_HOST="$registry_host" REG_PORT="$REG_PORT" \
    bash -s <<'EOF'
set -euo pipefail
IMAGE="$PULL_IMAGE"
rm -rf "$EV"
mkdir -p "$EV"
# Oracle is ubuntu+docker group (not root); Netcup is root. Prefer sudo when needed.
write_insecure_registry() {
python3 - <<'PY'
import json, os, pathlib, tempfile, subprocess
entry=f"{os.environ['REG_HOST']}:{os.environ['REG_PORT']}"
extras=("127.0.0.1:"+os.environ["REG_PORT"], "localhost:"+os.environ["REG_PORT"])
path=pathlib.Path("/etc/docker/daemon.json")
cfg={}
raw=""
try:
  raw=path.read_text()
  cfg=json.loads(raw or "{}")
except Exception:
  cfg={}
m=cfg.get("insecure-registries") or []
for e in (entry,)+extras:
  if e not in m:
    m.append(e)
cfg["insecure-registries"]=m
text=json.dumps(cfg, indent=2)+"\n"
try:
  path.parent.mkdir(parents=True, exist_ok=True)
  path.write_text(text)
  print("daemon.json written as", os.getuid(), cfg)
except PermissionError:
  fd, tmp=tempfile.mkstemp(prefix="exyonq-daemon-", suffix=".json")
  os.close(fd)
  pathlib.Path(tmp).write_text(text)
  subprocess.check_call(["sudo","-n","mkdir","-p","/etc/docker"])
  subprocess.check_call(["sudo","-n","cp",tmp,"/etc/docker/daemon.json"])
  os.unlink(tmp)
  print("daemon.json written via sudo", cfg)
PY
}
write_insecure_registry
if command -v systemctl >/dev/null; then
  systemctl reload docker 2>/dev/null || sudo -n systemctl reload docker 2>/dev/null || true
fi
sleep 1
docker pull "$IMAGE" 2>&1 | tee "$EV/pull.log"
inspect=$(docker image inspect "$IMAGE" --format '{{.Architecture}} {{index .Config.Labels "org.opencontainers.image.revision"}} {{index .Config.Labels "org.opencontainers.image.version"}}')
echo "INSPECT=$inspect" | tee "$EV/inspect.txt"
arch=$(echo "$inspect" | awk '{print $1}')
rev=$(echo "$inspect" | awk '{print $2}')
[[ "$arch" == "$EXPECT_ARCH" ]] || { echo "FAIL arch got=$arch want=$EXPECT_ARCH"; exit 2; }
[[ "$rev" == "$HEAD" ]] || { echo "FAIL revision got=$rev want=$HEAD"; exit 3; }
echo "MULTIARCH_PULL_PASS arch=$arch rev=$rev" | tee "$EV/pull-verdict.txt"
TMP=$(mktemp -d)
WWW="$TMP/www"; mkdir -p "$WWW"
printf 'oci-e2e-static' >"$WWW/index.html"
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
PORT=$((18080 + RANDOM % 1000))
CID=$(docker run -d --rm \
  -p "127.0.0.1:${PORT}:8080" \
  -v "$CFG:/etc/exyonq/config.toml:ro" \
  -v "$WWW:/var/www/exyonq:ro" \
  "$IMAGE")
echo "CID=$CID PORT=$PORT" | tee "$EV/run.txt"
ready=0
for _ in $(seq 1 90); do
  if curl -sf "http://127.0.0.1:${PORT}/site/" >/dev/null; then ready=1; break; fi
  sleep 0.5
done
[[ "$ready" == "1" ]] || { docker logs "$CID" >"$EV/logs.txt" 2>&1 || true; docker stop "$CID" || true; echo FAIL_NOT_READY; exit 4; }
BODY=$(curl -sf "http://127.0.0.1:${PORT}/site/")
[[ "$BODY" == "oci-e2e-static" ]] || { docker stop "$CID" || true; echo "FAIL body=$BODY"; exit 5; }
MISS=$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${PORT}/site/missing")
[[ "$MISS" == "404" ]] || { docker stop "$CID" || true; echo "FAIL miss=$MISS"; exit 6; }
docker stop "$CID" >/dev/null
rm -rf "$TMP"
echo "REAL_E2E_PASS" | tee "$EV/e2e-verdict.txt"
EOF
  mkdir -p "$EV_ROOT/$label"
  scp -o BatchMode=yes -q "${host}:${remote_ev}/*" "$EV_ROOT/$label/" 2>/dev/null || true
}

ORACLE_IP="$(ssh_o 'curl -4 -s ifconfig.me')"
echo "ORACLE_IP=$ORACLE_IP" | tee -a "$EV_ROOT/meta.txt"
ssh_n "iptables -C INPUT -p tcp --dport ${REG_PORT} -s ${ORACLE_IP} -j ACCEPT 2>/dev/null || \
  iptables -I INPUT -p tcp --dport ${REG_PORT} -s ${ORACLE_IP} -j ACCEPT || true"

# Same multiarch tag; Netcup uses loopback, Oracle uses Netcup public IP.
e2e_remote "$NETCUP_HOST" "netcup-amd64" "amd64" "127.0.0.1:${REG_PORT}/exyonq/exyonq:${VERSION}" &
PID_N=$!
e2e_remote "$ORACLE_HOST" "oracle-arm64" "arm64" "${NETCUP_IP}:${REG_PORT}/exyonq/exyonq:${VERSION}" &
PID_O=$!
wait "$PID_N"; EC_N=$?
wait "$PID_O"; EC_O=$?

echo "NETCUP_EC=$EC_N ORACLE_EC=$EC_O" | tee -a "$EV_ROOT/meta.txt"
[[ "$EC_N" -eq 0 ]] || { echo "FAIL netcup e2e"; exit 10; }
[[ "$EC_O" -eq 0 ]] || { echo "FAIL oracle e2e"; exit 11; }

# Attestation subject binding + distinct provenance/SBOM predicate proof (on Netcup registry).
ssh_n "docker buildx imagetools inspect '${IMAGE}' --format '{{json .}}'" >"$EV_ROOT/imagetools.json" || true
# Fetch attestation layer predicates from the disposable registry (fail-closed).
ssh_n "IMAGE='${IMAGE}' HEAD='${HEAD}' REG_PORT='${REG_PORT}' python3 -" >"$EV_ROOT/attestation-predicates.json" <<'REMOTE'
import json, os, urllib.request, gzip, sys
head = os.environ["HEAD"]
port = os.environ["REG_PORT"]
# IMAGE like 127.0.0.1:5000/exyonq/exyonq:tag
img = os.environ["IMAGE"]
host_port, rest = img.split("/", 1)
name, tag = rest.rsplit(":", 1)
reg = f"http://{host_port}"

def get_json(url, accept):
    req = urllib.request.Request(url, headers={"Accept": accept})
    return json.loads(urllib.request.urlopen(req).read())

def get_bytes(url):
    return urllib.request.urlopen(url).read()

idx = get_json(
    f"{reg}/v2/{name}/manifests/{tag}",
    "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json",
)
descs = idx.get("manifests") or []
att = [
    d for d in descs
    if (d.get("annotations") or {}).get("vnd.docker.reference.type") == "attestation-manifest"
    or (d.get("platform") or {}).get("os") == "unknown"
]
img_descs = [d for d in descs if (d.get("platform") or {}).get("os") == "linux"]
img_digests = {d.get("digest") for d in img_descs}
per_platform = {}
for a in att:
    ann = a.get("annotations") or {}
    subj = ann.get("vnd.docker.reference.digest")
    digest = a.get("digest")
    if subj not in img_digests:
        continue
    man = get_json(
        f"{reg}/v2/{name}/manifests/{digest}",
        "application/vnd.oci.image.manifest.v1+json",
    )
    preds = set()
    rev_in_provenance = False
    for layer in man.get("layers") or []:
        blob = get_bytes(f"{reg}/v2/{name}/blobs/{layer['digest']}")
        try:
            blob = gzip.decompress(blob)
        except Exception:
            pass
        try:
            j = json.loads(blob)
        except Exception:
            continue
        pt = j.get("predicateType")
        payload = j
        if "payload" in j and not pt:
            import base64
            raw = base64.b64decode(j["payload"] + "===")
            try:
                raw = gzip.decompress(raw)
            except Exception:
                pass
            try:
                payload = json.loads(raw)
                pt = payload.get("predicateType")
            except Exception:
                pass
        if pt:
            preds.add(pt)
        if pt and "provenance" in pt and head in json.dumps(payload):
            rev_in_provenance = True
    has_prov = any("slsa.dev/provenance" in p for p in preds)
    has_sbom = any("spdx.dev" in p or "cyclonedx" in p.lower() for p in preds)
    per_platform[subj] = {
        "attestation": digest,
        "predicates": sorted(preds),
        "HAS_PROVENANCE": has_prov,
        "HAS_SBOM": has_sbom,
        "SOURCE_REVISION_IN_PROVENANCE": rev_in_provenance,
    }

unbound = sorted(img_digests - set(per_platform))
all_bound = (not unbound) and len(img_digests) >= 2
provenance_ok = all_bound and all(v["HAS_PROVENANCE"] for v in per_platform.values())
sbom_ok = all_bound and all(v["HAS_SBOM"] for v in per_platform.values())
rev_ok = all_bound and all(v["SOURCE_REVISION_IN_PROVENANCE"] for v in per_platform.values())
report = {
    "ATTESTATION_DESCRIPTORS": len(att),
    "IMAGE_PLATFORM_DESCRIPTORS": len(img_descs),
    "PER_PLATFORM": per_platform,
    "UNBOUND_PLATFORM_DIGESTS": unbound,
    "ALL_PLATFORMS_BOUND": all_bound,
    "PROVENANCE_SUBJECT_MATCHES_PLATFORM_MANIFEST": provenance_ok,
    "SBOM_SUBJECT_MATCHES_PLATFORM_MANIFEST": sbom_ok,
    "SOURCE_REVISION_IN_PROVENANCE": rev_ok,
    "EXPECTED_REVISION": head,
}
print(json.dumps(report, indent=2))
if not all_bound:
    raise SystemExit(f"FAIL: unbound platforms {unbound}")
if not provenance_ok:
    raise SystemExit("FAIL: missing SLSA provenance predicate on one or more platforms")
if not sbom_ok:
    raise SystemExit("FAIL: missing SPDX/SBOM predicate on one or more platforms")
if not rev_ok:
    raise SystemExit("FAIL: SOURCE_REVISION missing from provenance blobs")
print("ATTESTATION_BINDING_OK", file=sys.stderr)
REMOTE
# Local copy for evidence package + fail-closed parse
cp "$EV_ROOT/attestation-predicates.json" "$EV_ROOT/attestation-binding.json"
python3 - "$EV_ROOT/attestation-predicates.json" <<'PY'
import json, sys
rep = json.loads(open(sys.argv[1]).read())
assert rep["PROVENANCE_SUBJECT_MATCHES_PLATFORM_MANIFEST"] is True
assert rep["SBOM_SUBJECT_MATCHES_PLATFORM_MANIFEST"] is True
assert rep["SOURCE_REVISION_IN_PROVENANCE"] is True
assert rep["ALL_PLATFORMS_BOUND"] is True
print("ATTESTATION_BINDING_OK")
PY

# Keep imagetools text for humans
if ! grep -qiE 'attestation|Provenance|SBOM' "$EV_ROOT/imagetools.txt"; then
  echo "WARN: imagetools text did not clearly name attestation; predicate binding OK"
fi

echo "V044_OCI_BASELINE_VALIDATE=PASS" | tee "$EV_ROOT/VERDICT.txt"
echo "EV_ROOT=$EV_ROOT"
