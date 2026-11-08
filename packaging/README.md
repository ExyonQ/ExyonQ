# Packaging (Fase 3)

Tier 1 release artifacts and Docker images for ExyonQ.

## Tier 1 targets

| OS | Arch | Artifact |
| --- | --- | --- |
| Linux | x86_64 | `exyonq-linux-amd64.tar.gz` (flat: binaries + legal + SBOM) |
| Linux | arm64 | `exyonq-linux-arm64.tar.gz` (flat: binaries + legal + SBOM) |
| Linux | x86_64 | `exyonq-${VERSION}-linux-amd64.tar.gz` (versioned FHS layout) |
| Linux | arm64 | `exyonq-${VERSION}-linux-arm64.tar.gz` (versioned FHS layout) |
| Linux | x86_64 | `exyonq_*_amd64.deb` / `exyonq-*.x86_64.rpm` |
| Linux | arm64 | `exyonq_*_arm64.deb` / `exyonq-*.aarch64.rpm` |
| macOS | arm64 | `exyonq-macos-arm64.tar.gz` |
| Windows | x86_64 | `exyonq-windows-amd64.zip` |

### Linux packaging contract (Cap062)

Required current product surfaces:

1. Flat Linux tarball (amd64 + arm64) — binaries + LICENSE/NOTICE/THIRD_PARTY_NOTICES/sbom
2. Versioned FHS Linux tarball (amd64 + arm64) — `/usr/bin`, `/etc/exyonq`, systemd unit, tmpfiles, logrotate, licenses, `build-manifest.json`
3. DEB (amd64 + arm64) via nfpm from the matching flat tarball payload
4. RPM (amd64 + arm64) via nfpm from the matching flat tarball payload

OCI multiarch images are Cap010 — not Cap062.

Architecture labels must agree across filename, package metadata, ELF payload, manifest, and SBOM binding.

Local helper (packages prebuilt binaries; does not execute them):

```bash
bash scripts/release/nfpm-package-linux.sh \
  --arch amd64|arm64 \
  --version "$(grep -E '^version' Cargo.toml | head -1 | cut -d'"' -f2)" \
  --bin-dir /path/to/binaries \
  --out-dir dist/packages
```

## Old split units and custom control sockets

The packaged unit uses `RuntimeDirectory=%p` and
`EXYONQ_CONTROL_SOCKET=/run/%p/control.sock`. Units copied from 0.9.0 or
older may still share `RuntimeDirectory=exyonq` and
`/run/exyonq/control.sock`; two live units cannot own the same socket.

`logrotate` now reads each live server's `EXYONQ_CONTROL_SOCKET` and
`EXYONQ_CONFIG` from `/proc/<pid>/environ` (or `-c/--config` from its command
line), so it reloads split units and custom socket paths with the matching
config and without an extra logrotate stanza. That does **not** repair a
socket bind collision. Migrate each old unit with a drop-in:

```ini
# /etc/systemd/system/exyonq-http.service.d/10-runtime-directory.conf
[Service]
RuntimeDirectory=
RuntimeDirectory=%p
Environment=EXYONQ_CONTROL_SOCKET=/run/%p/control.sock
ReadWritePaths=/run/%p
```

Then run `systemctl daemon-reload` and restart that unit. For a template unit,
use `RuntimeDirectory=exyonq-%i`,
`EXYONQ_CONTROL_SOCKET=/run/exyonq-%i/control.sock` and
`ReadWritePaths=/run/exyonq-%i` instead of `%p`.

## Optional L2 Redis coordination (WC7D)

Examples (Redis not bundled; private network only):

| Path | Purpose |
| --- | --- |
| `config/distributed-cache-redis.example.toml` | IR contract snippet (default off) |
| `config/distributed-cache-health.example.md` | Dataplane vs coordination health |
| `systemd/exyonq-l2-coord.env.example` | Env / secret-file mounts |
| `docker/distributed-cache-secrets.example.yml` | Compose secret mount sketch |
| `redis/exyonq-coord.acl` | Production ACL profile |

See `docs/operations/cache/DISTRIBUTED_INVALIDATION_REDIS_RUNBOOK.md`.

## Local release build check

```bash
cargo build --release -p exyonq
cargo build --release -p exyonqctl
cargo build --release -p exyonq-compat
```

## Legal bundle (release)

Public Tier 1 artifacts must ship:

| File | Purpose |
| --- | --- |
| `LICENSE` | ExyonQ Apache-2.0 license text |
| `NOTICE` | Project attribution |
| `THIRD_PARTY_NOTICES.md` | Human-readable third-party licenses (cargo-about) |
| `sbom.cdx.json` | CycloneDX SBOM for supply-chain audit (cargo-cyclonedx) |

```bash
cargo install cargo-about --locked --features cli
cargo install cargo-cyclonedx --locked
cargo install cargo-deny --locked

bash scripts/legal/generate-release-compliance-artifacts.sh
bash scripts/legal/verify-release-legal-bundle.sh
```

Before Docker build locally, generate the compliance artifacts first (Dockerfile copies them from context; it does not run `cargo-about`).

See [ADR-024](../docs/adr/024-license-compliance-third-party-notices.md).

## CI gates

| Gate | Workflow | When |
| --- | --- | --- |
| License compliance | `security.yml` → `legal-compliance` | Every PR and push to `main` |
| Advisories | `security.yml` → `deps-deny-advisories` | Push to `main` only (not PRs) |
| Release legal + SBOM | `release.yml` → `compliance-artifacts` | Tags `v*` |

## Docker (linux amd64 + arm64)

```bash
# Dev / local iteration — unknown revision allowed when EXYONQ_OFFICIAL_RELEASE=0.
bash scripts/legal/generate-release-compliance-artifacts.sh
REV="$(git rev-parse HEAD)"
docker buildx build -f packaging/docker/Dockerfile \
  --platform linux/amd64,linux/arm64 \
  --build-arg EXYONQ_VERSION=0.9.2 \
  --build-arg "EXYONQ_GIT_REVISION=${REV}" \
  --build-arg EXYONQ_OFFICIAL_RELEASE=0 \
  -t exyonq/exyonq:local .

# Official release image — FAIL_CLOSED without a 40-hex revision; never rely on .git in context.
docker buildx build -f packaging/docker/Dockerfile \
  --platform linux/amd64,linux/arm64 \
  --build-arg EXYONQ_VERSION=0.9.2 \
  --build-arg "EXYONQ_GIT_REVISION=${REV}" \
  --build-arg EXYONQ_OFFICIAL_RELEASE=1 \
  -t ghcr.io/exyonq/exyonq:0.9.2 .
```

## Docker image bases

Every image ExyonQ builds uses a Wolfi/Chainguard base. Oracle/reference images
pulled only by benchmarks (NGINX, ModSecurity/CRS, upstream PHP) are external
and are not rebuilt by ExyonQ.

| Image | Dockerfile | Base | Status (2026-11-06) |
| --- | --- | --- | --- |
| `exyonq` | `packaging/docker/Dockerfile` | `chainguard/rust:latest-dev` (build), `chainguard/glibc-dynamic` (runtime) | built and smoke-tested |
| `exyonq-wordpress` | `packaging/docker/wordpress/Dockerfile` | `chainguard/wolfi-base` + Wolfi PHP 8.5.12 | built; installer HTTP 200 on arm64 (0.9.1 local); 0.9.2 build/E2E pending |
| `exyonq-bench/server` | `benchmarks/docker/Dockerfile.exyonq` | `chainguard/rust` (build), `chainguard/wolfi-base` (runtime) | built; serves benchmark target |
| `exyonq-bench/tls` | `benchmarks/docker/Dockerfile.bench-tls` | `chainguard/wolfi-base` | built |
| `exyonq-bench/runner` | `benchmarks/docker/Dockerfile.bench-runner` | `chainguard/rust` (build), `chainguard/wolfi-base` (runtime) | built; `rewrk` real run PASS |
| `exyonq-bench/upstream` | `benchmarks/docker/Dockerfile.upstream` | `chainguard/rust` (build), `chainguard/wolfi-base` (runtime) | **open**: `tools/upstream/` absent from checkout and public history |

## CI

GitHub Release workflow (`.github/workflows/release.yml`) builds Tier 1 matrix on tag `v*` and publishes SHA256 checksums. Linux packages job builds **both** amd64 and arm64 `.deb`/`.rpm`.

OCI publish encodes the canonical contract explicitly for both images:

- `docker` job: `ghcr.io/exyonq/exyonq:<version>`
- `docker-wordpress` job: `ghcr.io/exyonq/exyonq-wordpress:<version>`,
  built from the just-published server image and the same version tag
- `dockerhub-mirror` job: digest copy of both published GHCR indexes to
  `docker.io/exyonq/exyonq:<version>` and
  `docker.io/exyonq/exyonq-wordpress:<version>`; the mirrored index digest must
  equal the GHCR one. Requires the `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN`
  repository secrets and fails closed without them.
- platforms: `linux/amd64,linux/arm64` (multiarch index)
- provenance: `mode=max`
- SBOM attestation: enabled
- version tag only (no `:latest`)
- revision/version/source via build-args + OCI labels

Signing (cosign/minisign) is optional follow-up when infra is ready.
