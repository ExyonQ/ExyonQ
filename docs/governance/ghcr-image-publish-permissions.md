# GHCR image publication permissions

```text
SCOPE = release.yml docker / docker-wordpress jobs
SYMPTOM = denied: permission_denied: write_package
ROOT_CAUSE = package "Manage Actions access" granted the repository Role: Read
FIX = change that role to Write, per package
FIX_PLACE = GitHub package settings (no REST/GraphQL endpoint exists)
PROOF = successful push in a re-run of the tagged release run
```

## 1. Symptom

`release.yml` builds both platforms, then fails at the end of
`docker/build-push-action` while pushing:

```text
ERROR: failed to push ghcr.io/exyonq/exyonq:<version>: denied: permission_denied: write_package
```

Everything upstream is green (`security-release`, `compliance-artifacts`, the
three `build` targets, `package-linux`, `checksums`). The server image never
lands, so `docker-wordpress` and `github-release` are skipped and the GitHub
Release can exist with `container_images: []`.

Recorded failures, both with the same error and the same root cause:

| Release | Run | Failing moment |
| --- | --- | --- |
| v0.9.0 | `41361024701` | 2026-11-02T00:54:30Z, `docker` |
| v0.9.1 | `42247244600` | 2026-11-06T17:45:18Z, `docker` |

## 2. Root cause

`permissions: packages: write` on the job is necessary but **not sufficient**.
The package also has to grant the repository access through its Actions ACL.

Both packages are user-owned (`/users/<owner>/packages/...`), not org-owned, and
their *Manage Actions access* list contained exactly one entry:

```text
exyonq            -> ExyonQ/ExyonQ -> Role: Read
exyonq-wordpress  -> ExyonQ/ExyonQ -> Role: Read
```

`Read` permits pulling; publishing a version requires `Write`. A package created
by a manual `docker push` (PAT) rather than by a workflow does not get a write
grant automatically, which is why the first workflow-driven push for a new
version tag is denied while the package itself looks correctly linked through
`org.opencontainers.image.source`.

This ACL is not exposed by the REST or GraphQL API; it exists only in the
package settings UI. Any "fix" that cannot be read back through an API must be
proven by a real push (AGENTS.md §7.1).

## 3. Procedure (owner, UI)

Repeat for **each** package the release publishes (`exyonq`,
`exyonq-wordpress`):

1. Open `https://github.com/users/ExyonQ/packages/container/<package>/settings`
   (or: repo → Packages → `<package>` → *Package settings*).
2. In **Manage Actions access**, locate the `ExyonQ/ExyonQ` entry.
3. Set its role to **Write** (`Read` → `Write`).
4. Confirm the row now reads `Role: Write`.

Do not use **Admin**; the workflow only uploads versions.

## 4. Verification (real push, fail-closed)

`release.yml` has a dedicated `ghcr-preflight` job, run in parallel with
`security-release`, and `docker` depends on it. It checks both packages and
exchanges the workflow token for a GHCR token with
`repository:<repo>:pull,push` scope and `POST`s to `/v2/<repo>/blobs/uploads/`.
`202` means the package ACL grants Write. The step fails closed in seconds when
the exchange is refused (`403`, no package access) or when the upload session is
not created (`401`/`403`, pull-only grant). The probe opens an upload session
that is never completed, so it publishes no tag, manifest or blob.

The preflight proves push *permission* only. The end-to-end proof is still a
real push in a tagged run:

```bash
gh run rerun 42247244600 --repo ExyonQ/ExyonQ --failed
gh run view 42247244600 --repo ExyonQ/ExyonQ \
  --json jobs --jq '[.jobs[]|{name,conclusion}]'
```

Notes that matter for the evidence:

- A re-run reuses the original `GITHUB_SHA` and `GITHUB_REF` and the original
  actor's privileges, and it mints a fresh `GITHUB_TOKEN`; it is a real test of
  the ACL, not a rebuild of the tree.
- *Re-run failed jobs* also runs the jobs that depend on the failed one, so
  `docker-wordpress` and `github-release` execute once `docker` passes.
- Do not move or recreate the tag to force a run. Re-running the existing run
  keeps the tag target and the workflow revision immutable.

Then resolve the published artifacts by digest:

```bash
docker buildx imagetools inspect ghcr.io/exyonq/exyonq:0.9.1
docker buildx imagetools inspect ghcr.io/exyonq/exyonq-wordpress:0.9.1
```

The gate is: both resolve, both are multi-arch indexes (`linux/amd64`,
`linux/arm64`), no `:latest` is created, and the digest is what
`release-manifest.json` records.

## 5. Rejected alternative

A long-lived PAT (`secrets.GHCR_TOKEN`, `write:packages`) also works, but it
replaces a short-lived, repo-scoped `GITHUB_TOKEN` with a standing credential
that must be rotated. The ACL grant keeps the workflow on `GITHUB_TOKEN`; prefer
it unless a future registry requires a credential GitHub cannot mint.

## 6. Rollback

Set the role back to `Read`. Existing published versions remain pullable; only
future uploads are denied.

## 7. Docker Hub

`release.yml` mirrors both images to Docker Hub in the `dockerhub-mirror` job,
which runs after `docker` and `docker-wordpress` and is a dependency of
`github-release`. It copies the published GHCR index digest (no rebuild), so the
Hub and GHCR indexes are identical, and it **fails closed** when the
`DOCKERHUB_USERNAME` or `DOCKERHUB_TOKEN` repository secret is missing: a
GHCR-only publication is not a release (AGENTS.md §9). The mirror is not blocked
by the GHCR ACL and cannot run before the GHCR push succeeds.

Manual fallback for a release published before this job existed:

```bash
docker buildx imagetools create -t docker.io/exyonq/exyonq:<version> \
  ghcr.io/exyonq/exyonq@sha256:<ghcr-index-digest>
docker buildx imagetools create -t docker.io/exyonq/exyonq-wordpress:<version> \
  ghcr.io/exyonq/exyonq-wordpress@sha256:<ghcr-index-digest>
docker buildx imagetools inspect docker.io/exyonq/exyonq:<version>  # must match GHCR
```

## 8. Recorded verification (0.9.1)

```text
DATE = 2026-11-07
ACL_BEFORE = exyonq, exyonq-wordpress -> ExyonQ/ExyonQ -> Read
ACL_AFTER  = exyonq, exyonq-wordpress -> ExyonQ/ExyonQ -> Write (persisted after reload)
RUN        = 42247244600 attempt 2 -> success (docker, docker-wordpress, github-release)
SERVER     = ghcr.io/exyonq/exyonq:0.9.1@sha256:e147a31eea670f6f9f109d243e4f239704c448f1d5fc7a7202e12b65836eff06
WORDPRESS  = ghcr.io/exyonq/exyonq-wordpress:0.9.1@sha256:36d595d64cba38df5628b2ad0988ebcd522feadc99e6758f4f7075640e1d587a
LATEST     = 404 on both repositories
ARTIFACT   = arm64 image reports exyonq 0.9.1 / source_tag=v0.9.1 / source_revision=ed778ee…
PREFLIGHT  = 202/WRITABLE on both with a write token; 403 at exchange with a token lacking package access
PREFLIGHT_RUN = 42403720738 (workflow_dispatch) -> job success in 4 s
```

Both indexes are `application/vnd.oci.image.index.v1+json` with
`linux/amd64` and `linux/arm64` plus attestation manifests. Full commands and
results: `docs/releases/0.9.1-ghcr-publication.md`. The Docker Hub mirror and a
manifest that binds these digests remain open items.

### Executed in GitHub Actions

Run [`42403720738`](https://github.com/ExyonQ/ExyonQ/actions/runs/42403720738)
(`workflow_dispatch` on `ci/ghcr-push-preflight`): the `ghcr push permission`
job was **success** in 4 seconds (18:56:05Z to 18:56:09Z). Log:

```text
18:56:06.952Z GHCR_PUSH_PERMISSION=exyonq/exyonq:WRITABLE
18:56:07.333Z GHCR_PUSH_PERMISSION=exyonq/exyonq-wordpress:WRITABLE
```

The run as a whole is `failure` by design: `security-release` rejects any ref
that is not `refs/tags/v*`, so `Resolve version` exits 1 and `compliance`,
`docker`, `docker-wordpress` and `github-release` stay `skipped`. Nothing was
built, pushed or released. This is the mounted proof that the probe runs in
Actions and that a denied ACL stops the pipeline before the multi-arch build;
the `403` branch was exercised locally against the same script.
