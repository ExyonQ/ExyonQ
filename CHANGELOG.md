# Changelog

All notable changes to ExyonQ are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

```text
P14CLEAN_GITHUB_INITIAL_VERSION = v0.4.0
P14CLEAN_GITHUB_PREVIOUS_LEGACY_VERSION = v0.3.3
P14CLEAN_GITHUB_REUSE_LEGACY_TAGS = NO
P14CLEAN_GITHUB_IMMUTABLE_RELEASE_WARNING_ACK = YES
PUBLICATION_STATUS = FORBIDDEN
```

This repository is a **clean source tree**. Legacy tags from the previous GitHub repository
(`ExyonQ-Old`, last published lineage ending at `v0.3.3`) are **not** reused here because
GitHub immutable release history for the name `ExyonQ/ExyonQ` blocks recreating those tags.

---

## [Unreleased]

### Notes

- Next candidate after private maturation of **`0.4.1`** (this line). Publication/signing remain owner-gated.
- Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

---

## [0.4.1] — private maturation (not published)

```text
P14V041_RELEASE_EXECUTED = NO
PUBLICATION_STATUS = FORBIDDEN
BASE = d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910 (v0.4.0 freeze)
```

### Changed

- Dependency maturation under `P14V041` with mandatory completion (no convenience deferrals).
- `anyhow` 1.0.103 → 1.0.104.
- `serde` / `serde_derive` 1.0.228 → 1.0.229 (`serde_core` 1.0.229); `serde_json` left at 1.0.150 (no advisory/coupling).
- `bytes` 1.12.0 → 1.12.1 (hot path; dual-arch perf evidence required before release authorize).
- `notify` 7.0.0 → 8.2.0 (no `9.0.0-rc`); reload / htaccess watchers validated locally.
- `socket2` direct product usage unified on 0.6.x (`0.6.5`); `exyonq-platform-linux` migrated from 0.5.x (`set_nodelay` → `set_tcp_nodelay`). Transitive third-party `socket2` 0.5.10 may remain.
- GitHub Actions: `actions/cache` → v6.1.0 SHA-pinned; `actions/setup-go` → v7.0.0 SHA-pinned; remaining admitted active Actions SHA-pinned (including `dtolnay/rust-toolchain` master tip pin preserving toolchain inputs).

### Security

- `EXYONQ-SEC-PRIVATE-MATERIAL-ZERO` remains fail-closed; ephemeral TLS only.
- `cargo audit` / `cargo deny` advisories gates run per dependency commit; pre-existing `RUSTSEC-2025-0134` (rustls-pemfile unmaintained) unchanged.
- Notify 8 removes unmaintained `instant` from the lock graph.

### Limitations (honest)

- Dual-arch Linux (Netcup amd64 / Oracle arm64) functional + performance gates must pass before `P14V041_RELEASE_READY = YES`.
- No tag, GitHub Release, GHCR push, `latest` change, or signing in this phase.
- Docker Desktop / Mac results are `LOCAL_ITERATION_ONLY` / `NOT_LINUX_EVIDENCE`.

---

## [0.4.0] — privately closed freeze

Freeze commit: `d326b02b4ebc3b8dd7a8a3dbe7deeab5251d4910`.

### Changed

- Clean repository restart: product workspace version set to `0.4.0`.
- Official addon `core_compat` window widened to `>=0.1.0, <0.5.0` so addons handshake with 0.4.x cores.
- Source publication excludes internal `docs/`, `benchmarks/`, and evidence packs.
- Default allocator remains system; optional jemalloc kept; mimalloc absent.
- Default HTTP/3 provider remains s2n; optional quiche; quinn-legacy rollback.

### Security

- Private keys, cosign private material, and registry credentials must never be committed.
- Artifact/container signing remains owner-gated (not opened by P14V041).
