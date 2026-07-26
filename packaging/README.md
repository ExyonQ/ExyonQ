# Packaging (Fase 3)

Tier 1 release artifacts and Docker images for ExyonQ.

## Tier 1 targets

| OS | Arch | Artifact |
|----|------|----------|
| Linux | x86_64 | `exyonq-linux-amd64.tar.gz` |
| Linux | arm64 | `exyonq-linux-arm64.tar.gz` |
| macOS | arm64 | `exyonq-macos-arm64.tar.gz` |
| Windows | x86_64 | `exyonq-windows-amd64.zip` |

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

## Local release smoke

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
bash scripts/legal/generate-release-compliance-artifacts.sh
docker buildx build -f packaging/docker/Dockerfile \
  --platform linux/amd64,linux/arm64 \
  -t exyonq/exyonq:latest .
```

## CI

GitHub Release workflow (`.github/workflows/release.yml`) builds Tier 1 matrix on tag `v*` and publishes SHA256 checksums.

Signing (cosign/minisign) is optional follow-up when infra is ready.
