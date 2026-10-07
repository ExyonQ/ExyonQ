# Packaging (Fase 3)

Tier 1 release artifacts and Docker images for ExyonQ.

## Tier 1 targets

| OS | Arch | Artifact |
|----|------|----------|
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

## Optional L2 Redis coordination (WC7D)

Examples (Redis not bundled; private network only):

| Path | Purpose |
|------|---------|
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
|------|---------|
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
|------|----------|------|
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
  --build-arg EXYONQ_VERSION=0.4.5 \
  --build-arg "EXYONQ_GIT_REVISION=${REV}" \
  --build-arg EXYONQ_OFFICIAL_RELEASE=0 \
  -t exyonq/exyonq:local .

# Official release image — FAIL_CLOSED without a 40-hex revision; never rely on .git in context.
# Runtime stage is scratch. Debian is used only to copy CA certificates and is not shipped.
docker buildx build -f packaging/docker/Dockerfile \
  --platform linux/amd64,linux/arm64 \
  --provenance=mode=max \
  --sbom=true \
  --build-arg EXYONQ_VERSION=0.4.5 \
  --build-arg "EXYONQ_GIT_REVISION=${REV}" \
  --build-arg EXYONQ_OFFICIAL_RELEASE=1 \
  -t ghcr.io/exyonq/exyonq:0.4.5 .
```

## CI

GitHub Release workflow (`.github/workflows/release.yml`) builds Tier 1 matrix on tag `v*` and publishes SHA256 checksums. Linux packages job builds **both** amd64 and arm64 `.deb`/`.rpm`.

OCI publish (`docker` job) encodes the canonical contract explicitly:

- platforms: `linux/amd64,linux/arm64` (multiarch index)
- provenance: `mode=max`
- SBOM attestation: enabled
- tags: `ghcr.io/exyonq/exyonq:<version>` only (no `:latest`)
- revision/version/source via build-args + OCI labels


WordPress and SQLite, after the scratch image exists locally as `exyonq:0.4.5`:

```bash
docker build -f packaging/docker/wordpress/Dockerfile -t ghcr.io/exyonq/exyonq-wordpress:0.4.5 .
docker pull ghcr.io/exyonq/exyonq-wordpress:0.4.5
```

Signing (cosign/minisign) is optional follow-up when infra is ready.
