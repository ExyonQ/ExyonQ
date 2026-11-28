# Docker image builds, base images and the Docker Scout gate

Owner rule (2026-11-28): **no ExyonQ image is uploaded until Docker Scout
reports zero fixable vulnerabilities for it.** When a scan finds vulnerabilities
the pinned base image digests are refreshed, the images are rebuilt and the scan
is repeated until clean; only then are the tags published to GHCR and Docker
Hub. Base images are always Wolfi/Chainguard, never Debian/Ubuntu/Alpine.

## Pinned bases (refresh them, do not float)

`packaging/docker/Dockerfile` and `packaging/docker/wordpress/Dockerfile` pin
every base by multi-arch digest:

| Stage | Image |
| --- | --- |
| Server builder | `cgr.dev/chainguard/rust:latest-dev@sha256:f7e4d266…` |
| Server runtime | `cgr.dev/chainguard/glibc-dynamic:latest@sha256:db4aee41…` |
| WordPress fetcher/runtime | `cgr.dev/chainguard/wolfi-base:latest@sha256:5172a213…` |

Refresh procedure (run on a Linux host; do not compile on the macOS host):

```bash
for img in cgr.dev/chainguard/rust:latest-dev \
           cgr.dev/chainguard/glibc-dynamic:latest \
           cgr.dev/chainguard/wolfi-base:latest; do
  printf '%s ' "$img"
  docker buildx imagetools inspect --format '{{.Manifest.Digest}}' "$img"
done
# verify the candidate base is clean before rebuilding
docker scout cves --only-fixed local://cgr.dev/chainguard/glibc-dynamic@sha256:<new>
```

Update both Dockerfiles with the new digests, rebuild, and scan the result.

## Gate

```bash
bash scripts/release/scout-gate.sh --image ghcr.io/exyonq/exyonq:0.9.3
bash scripts/release/scout-gate.sh --image exyonq/exyonq-wordpress:0.9.3 \
  --platform linux/arm64
```

- Default policy `any`: any reported vulnerability (fixable or not) fails the
  gate. This is the publishing bar, matching the owner rule.
- Policy `fixable` (`--only-fixed`): CI uses it because a freshly published
  Wolfi base can still carry a CVE the distribution has not patched; a fixable
  finding means the build is stale and must be redone.
- A failing gate prints the fixable entries and the base-digest next step.

## The 0.9.3 image refresh (2026-11-28)

The 0.9.3 images were published with glibc `2.44-r13` from the previous pinned
bases. Docker Scout reported **27 fixable entries** on the amd64 server image
(`CVE-2026-84243` high, `CVE-2026-115819` medium, `CVE-2026-100290` low, across
`glibc-2.44`, `ld-linux-2.44` and `glibc-2.44-locale-posix`) and 36 on the
WordPress image, all fixed upstream in `glibc-2.44-r16`/`r20`. The bases were
refreshed to the digests above, the images were rebuilt natively on the two
Linux build hosts (netcup amd64, Oracle arm64), re-scanned to zero and the
`0.9.3` indexes were re-published on GHCR and Docker Hub with new digests.

## Publishing flow (no macOS builds)

1. Export the tagged tree, add `sbom.cdx.json` and `THIRD_PARTY_NOTICES.md`.
2. Build natively per architecture on the Linux hosts:
   `docker buildx build --platform linux/amd64|linux/arm64 --push -t <staging tag>`.
3. Pull the staging image and run `scripts/release/scout-gate.sh` on the
   architecture that was built; the image is not published until it passes.
4. Combine the two per-arch staging manifests into the final multi-arch index
   with `docker buildx imagetools create` and push the version tag to GHCR and
   Docker Hub (staging tags are deleted afterwards).
5. Verify the mirrored index digests match and run the images
   (`/health` for the server image; the WordPress image's entrypoint) before
   updating the release note.

## CI

`release.yml` scans the pushed GHCR image with Docker Scout (`scout-server`,
`scout-wordpress`) and the Docker Hub mirror plus the GitHub Release wait for
those jobs, so an image that fails the gate is never mirrored nor attached to a
release. GHCR is the staging registry; the public Docker Hub copy and the
GitHub Release only happen after the scan passes.

## Result of the 0.9.3 refresh

| Image | Index digest (GHCR = Docker Hub) | amd64 | arm64 |
| --- | --- | --- | --- |
| `exyonq` | `sha256:12f5f518592ab522f9719285247c90d364393256b2f933c7e38c6c570158489d` | `sha256:57c1d6c2…` | `sha256:e63af593…` |
| `exyonq-wordpress` | `sha256:5c79e3d9df291b9bd1ac54c059b4de7c3402beb8b6ebf5701d9b67410b167ac5` | `sha256:2cb28bd4…` | `sha256:c641e5b8…` |

Scout (`--policy any`) reports `0C 0H 0M 0L` for every platform of both images
on both registries, verified after the final tags were created. Functional
checks on the rebuilt images: server `/health` 200 + root page, WordPress
`/health` 200 + `/` 302 to the installer + PHP 8.5.13, and both binaries report
`source_tag=v0.9.3` / `source_revision=b0d25ed…`.

## Registry hygiene after a re-publish

Moving a tag leaves the previous index and its child manifests in the registry.
GHCR exposes them as untagged package versions and the API can delete them; do
not delete a version that a current tag still references (compute the digest
closure of every tagged index first, e.g. with `docker buildx imagetools
inspect --raw` recursively).

```bash
gh api --paginate /users/ExyonQ/packages/container/exyonq/versions \
  --jq '.[] | "\(.id) \(.name) tags=\(.metadata.container.tags|join(","))"'
gh api -X DELETE /users/ExyonQ/packages/container/exyonq/versions/<id>
```

Docker Hub's API only deletes tags; an untagged manifest has no public delete
endpoint, so old digests remain pullable by digest even though no tag reaches
them. Physical removal needs Docker Hub support (or deleting the repository,
which is not acceptable for a released version).

2026-11-28 purge: the four staging tags were deleted after publishing, and the
7 server + 5 WordPress GHCR versions of the vulnerable images were removed
(verified `not found` by digest; the `0.9.3` tags still resolve and scan clean).
On Docker Hub the old digests (`40873f66…`, `fe410132…`) still answer by digest
because there is no manifest-delete API; they are unreachable from any tag.
