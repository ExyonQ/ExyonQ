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

- First tag/release on this clean repository will be **`v0.4.0`** (not yet created).
- Signing, container publication, and GitHub Release artifacts are deferred to **P14SIGN**.
- Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

---

## [0.4.0] — pending first clean-repository release

### Changed

- Clean repository restart: product workspace version set to `0.4.0`.
- Official addon `core_compat` window widened to `>=0.1.0, <0.5.0` so addons handshake with 0.4.x cores.
- Source publication excludes internal `docs/`, `benchmarks/`, and evidence packs.
- Default allocator remains system; optional jemalloc kept; mimalloc absent.
- Default HTTP/3 provider remains s2n; optional quiche; quinn-legacy rollback.

### Security

- Private keys, cosign private material, and registry credentials must never be committed.
- Artifact/container signing design tracked under P14SIGN (not opened in this packet).
