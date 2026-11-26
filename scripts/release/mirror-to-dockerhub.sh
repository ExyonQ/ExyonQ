#!/usr/bin/env bash
# Mirror the published GHCR multi-arch indexes to Docker Hub by digest (no rebuild).
#
# Same contract as the `dockerhub-mirror` job in .github/workflows/release.yml:
# copy the GHCR index digest instead of retagging a local image, so both
# registries resolve to the identical index, and verify per-platform digests
# (linux/amd64, linux/arm64) on the mirror.
set -euo pipefail
LC_ALL=C
export LC_ALL

SOURCE_NS="ghcr.io/exyonq"
TARGET_NS="docker.io/exyonq"
REPOS="exyonq exyonq-wordpress"
VERSION=""
DRY_RUN=0

usage() {
  cat <<'EOF'
Usage:
  mirror-to-dockerhub.sh --version VERSION [--source-ns NS] [--target-ns NS]
                         [--dry-run]

Mirrors ghcr.io/exyonq/<name>:VERSION to docker.io/exyonq/<name>:VERSION for
<name> in exyonq exyonq-wordpress, then prints the index digest and the
linux/amd64 and linux/arm64 manifest digests of each mirror.

Requires an authenticated Docker Hub account with push access to the target
namespace (`docker login`), and network access to both registries. No image is
built, pulled to disk or retagged: only manifest copies are pushed.
EOF
}

die() {
  echo "ERROR: $*" >&2
  exit 1
}

require_arg() {
  [[ -n "${2:-}" ]] || die "missing value for $1"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) require_arg --version "${2:-}"; VERSION="$2"; shift 2 ;;
    --source-ns) require_arg --source-ns "${2:-}"; SOURCE_NS="$2"; shift 2 ;;
    --target-ns) require_arg --target-ns "${2:-}"; TARGET_NS="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ -n "$VERSION" ]] || { usage >&2; die "--version is required"; }
case "$VERSION" in
  latest|*:latest|*/*) die "invalid version: $VERSION (no :latest, no path separators)" ;;
esac
[[ "$VERSION" =~ ^[0-9]+(\.[0-9]+)*(-[A-Za-z0-9.]+)?$ ]] || die "invalid version: $VERSION"

command -v docker >/dev/null 2>&1 || die "docker not found"
docker buildx version >/dev/null 2>&1 || die "docker buildx not available"

# Prints "<index-digest> <amd64-digest> <arm64-digest> <mediaType>" for a ref.
# Fails when the ref is not a multi-arch index carrying both platforms.
read_index() {
  docker buildx imagetools inspect "$1" --format '{{json .Manifest}}' | python3 -c '
import json, sys

m = json.load(sys.stdin)
index = m.get("digest", "")
media = m.get("mediaType", "")
if not index.startswith("sha256:"):
    sys.exit("ref did not resolve to a sha256 index digest")
platforms = {}
for entry in m.get("manifests") or []:
    p = entry.get("platform") or {}
    if p.get("os") in (None, "unknown"):
        continue
    platforms[p.get("os") + "/" + p.get("architecture")] = entry.get("digest", "")
missing = [k for k in ("linux/amd64", "linux/arm64") if not platforms.get(k)]
if missing:
    sys.exit("index lacks required platform(s): " + ", ".join(missing))
print(index, platforms["linux/amd64"], platforms["linux/arm64"], media)
'
}

printf 'SOURCE_NAMESPACE=%s\n' "$SOURCE_NS"
printf 'TARGET_NAMESPACE=%s\n' "$TARGET_NS"
printf 'VERSION=%s\n' "$VERSION"

for name in $REPOS; do
  src="${SOURCE_NS}/${name}:${VERSION}"
  dst="${TARGET_NS}/${name}:${VERSION}"

  src_meta="$(read_index "$src")" || die "source ${src} is not a pullable multi-arch index"
  read -r src_index src_amd64 src_arm64 src_media <<<"$src_meta"
  [[ "$src_media" == *"image.index"* || "$src_media" == *"manifest.list"* ]] \
    || die "source ${src} media type ${src_media} is not an index"

  echo "SOURCE=${name}:${VERSION}@${src_index} PLATFORMS=linux/amd64=${src_amd64},linux/arm64=${src_arm64}"

  if [[ "$DRY_RUN" == "1" ]]; then
    echo "PLAN=docker buildx imagetools create -t ${dst%:*}@${src_index} ${src%:*}@${src_index}"
    echo "MIRROR=${name}:${VERSION}:DRY_RUN"
    continue
  fi

  docker buildx imagetools create -t "$dst" "${SOURCE_NS}/${name}@${src_index}" \
    || die "push to ${dst} failed (check that docker login has push access to ${TARGET_NS})"

  dst_meta="$(read_index "$dst")" || die "mirror ${dst} is not a pullable multi-arch index after push"
  read -r dst_index dst_amd64 dst_arm64 _dst_media <<<"$dst_meta"
  [[ "$dst_index" == "$src_index" ]] \
    || die "mirror ${dst} index ${dst_index} differs from GHCR ${src_index}"
  [[ "$dst_amd64" == "$src_amd64" ]] \
    || die "mirror ${dst} linux/amd64 ${dst_amd64} differs from GHCR ${src_amd64}"
  [[ "$dst_arm64" == "$src_arm64" ]] \
    || die "mirror ${dst} linux/arm64 ${dst_arm64} differs from GHCR ${src_arm64}"

  if docker buildx imagetools inspect "${TARGET_NS}/${name}:latest" >/dev/null 2>&1; then
    die "${TARGET_NS}/${name}:latest exists; the contract forbids :latest"
  fi

  printf 'MIRROR=%s\n' "$dst"
  printf 'INDEX=%s\n' "$dst_index"
  printf 'AMD64=%s\n' "$dst_amd64"
  printf 'ARM64=%s\n' "$dst_arm64"
  printf 'LATEST=ABSENT\n'
done

echo "DOCKERHUB_MIRROR=VERIFIED version=${VERSION}"
