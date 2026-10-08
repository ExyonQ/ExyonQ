# Changelog

All notable changes to ExyonQ are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

```text
P14CLEAN_GITHUB_INITIAL_VERSION = v0.4.0
P14CLEAN_GITHUB_PREVIOUS_LEGACY_VERSION = v0.3.3
P14CLEAN_GITHUB_REUSE_LEGACY_TAGS = NO
P14CLEAN_GITHUB_IMMUTABLE_RELEASE_WARNING_ACK = YES
PUBLICATION_STATUS = PUBLISHED
PUBLIC_RELEASE = v0.4.8
LATEST_CHANGED = NO
```

This repository is a **clean source tree**. Legacy tags from the previous GitHub repository
(`ExyonQ-Old`, last published lineage ending at `v0.3.3`) are **not** reused here because
GitHub immutable release history for the name `ExyonQ/ExyonQ` blocks recreating those tags.

---

## [Unreleased]

### Notes

- Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

---

## [0.4.8] — 2026-10-08

Product version 0.4.8. The GitHub release includes the Linux amd64 and arm64 binaries. Images: `ghcr.io/exyonq/exyonq:0.4.8` and `ghcr.io/exyonq/exyonq-wordpress:0.4.8`, both `linux/amd64` and `linux/arm64`. No `:latest` tag.

The official server image serves a default page on port 8080. The WordPress image starts with a configuration the server accepts, and `/` reaches the WordPress installer. `exyonq --version` reports Cargo, product, and artifact version `0.4.8`.

## [0.4.7] — 2026-10-07

Product version 0.4.7. The GitHub release includes the Linux amd64 and arm64 binaries. Images: `ghcr.io/exyonq/exyonq:0.4.7` and `ghcr.io/exyonq/exyonq-wordpress:0.4.7`, both `linux/amd64` and `linux/arm64`. No `:latest` tag.

One HTTPS listener can present a different certificate for each server name. The first certificate is used when the name is not listed. A redirect location may contain `{host}` and `{path}`, so an HTTP listener can send the client to HTTPS without dropping the request path. `exyonq --version` reports Cargo, product, and artifact version `0.4.7`.

## [0.4.6] — 2026-10-07

Product version 0.4.6. The GitHub release includes the Linux amd64 and arm64 binaries. Images: `ghcr.io/exyonq/exyonq:0.4.6` and `ghcr.io/exyonq/exyonq-wordpress:0.4.6`, both `linux/amd64` and `linux/arm64`. No `:latest` tag.

This cut carries the 0.4.5 WordPress page cache onto the current main line, including the dependency updates already merged there. `exyonq --version` reports Cargo, product, and artifact version `0.4.6`.

## [0.4.5] — 2026-10-07

Scratch image `ghcr.io/exyonq/exyonq:0.4.5` and WordPress image `ghcr.io/exyonq/exyonq-wordpress:0.4.5`, both `linux/amd64` and `linux/arm64`. No `:latest` tag.

The WordPress image turns the page cache on. Anonymous HTML is kept for 300 seconds. At startup ExyonQ creates `/run/exyonq/cache-purge.sock`, the token file, and the site index. Publishing a post sends `purge site` and that purge is accepted.

---

## [0.4.4.1] — 2026-10-03

Published release. Scratch runtime image for `linux/amd64` and `linux/arm64`, plus the matching static musl tarballs. Contact: contact@exyonq.org. Security: security@exyonq.org.

---

## [0.4.4] — published release

```text
PUBLICATION_STATUS = PUBLISHED
PUBLIC_RELEASE = v0.4.4
GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.4
GHCR = ghcr.io/exyonq/exyonq:0.4.4
LATEST_CHANGED = NO
TAGGED_COMMIT = 8f3336f783dc546bdaad4a60b5501568bf688b3a
```

### Changed

- Workspace product version **0.4.3 → 0.4.4**.
- Rust toolchain **1.98.1**.
- Production Linux binaries are musl and statically linked (`scripts/release/build-production.sh`).
- Dependency line moved to the current stables used by the product: Wasmtime 49.0.1, quiche 0.30.0, s2n-quic 1.89.0, tachyon-quic 0.3.0, rustls 0.23.45, hyper 1.11.1, OpenTelemetry 0.33.0, redis 1.7.1.
- `instant-acme` stays on the published 0.8.5 API. `third_party/instant-acme` is that release with `base64` 0.23, because 0.9.0 is not on crates.io.
- Dataplane fixes reproduced by independent Netcup oracles: static host routing, epoll inline access and request id, wire metrics, proxy upstream body completion, WAF challenge host, compression and rate-limit exemptions.

### Fixed

- `Cargo.lock` on the tagged commit still said `0.4.3` and still listed `exyonq-bench`, so `cargo build --locked` could not run. The lock now matches workspace `0.4.4` and no longer lists `exyonq-bench`.
- Release manifest allowlist now accepts `0.4.4`. `generate-release-manifest.sh` rejected it before.
- Added `docs/security/audit-v0.4.4.md` and `docs/release/v0.4.4.md`, which `release.yml` requires before it publishes. `.gitignore` now tracks `docs/release/` so that body file can be committed.
- Removed unused `write_raw_proxy_response`. `release.yml` builds with `-Dwarnings`, and that dead function was the warning.

### Not in this publication

- Benchmark suites, development HTML, and local evidence packs stay out of this tree.
- No GitHub Release, no `ghcr.io/exyonq/exyonq:0.4.4`, and no `:latest` tag.
- No claim that a public official benchmark was updated.

---


## [0.4.3] — private release (published on exyonq-github)

```text
RELEASE_AUTHORITY = exyonq-github (not this lab worktree)
PUBLICATION_STATUS = PRIVATE_V043_RELEASE_COMPLETE
PUBLIC_OPENING = NO
PUBLIC_RELEASE = v0.4.3
RELEASE_COMMIT = 3a8af75dd0939e5aae2f4d09b842573499e15a1c
EXYONQ_GITHUB_POST_RELEASE_HEAD = 22b20dfcce6718a0cad64938436af87e27ff383e
GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.3
GHCR = ghcr.io/exyonq/exyonq:0.4.3 @ sha256:ab02b5ffc3d54407a56b6edbb318d80a077da0f431970984408d82415217f2e5
LATEST_CHANGED = NO
PRODUCTION_SIGNING_EXECUTED = YES
SSH_TAG_SIGNATURE_STATUS = PASS
COSIGN_BLOB_SIGNATURE_STATUS = PASS
COSIGN_OCI_SIGNATURE_STATUS = PASS
```

### Changed

- Workspace product version **0.4.2 → 0.4.3** (Cargo/CLI/`--version`/packaging identity).
- Release tooling allowlists accept **0.4.3** while retaining supported 0.4.0–0.4.2 identities.
- Integrity corrections (Changeset A/B): remove implicit benchmark API response cache and retire `EXYONQ_BENCH_CACHE_HEADERS` response-header injection.
- HTTP/3 POST body collected before proxy dispatch (correctness).
- Productive endpoint-set IR + weighted endpoint selection (WRR); OPEN-002 eligible==1 Multi→Single collapse.
- Authoritative NO-SMOKE real E2E suite and Basic Product Completeness qualification path.
- R3 fixed-rate P4 dual-arch competitive result: **PARITY** (evidence-backed; not a superiority claim).
- Governance: security/governance SoT, integrity triad, PMZ scanner/hooks, release-audit infra, RUSTSEC-2026-0222 waiver register for Wasmtime **45.0.2** (`VULNERABILITY_FIXED=NO`, revisit v0.4.4).

### Security

- `docs/security/audit-v0.4.3.md` freeze-track audit executed (release evidence).
- RUSTSEC-2026-0222 remains **unfixed** (`VULNERABILITY_FIXED=NO`) and **waived for v0.4.3 only** (`WAIVED_FOR_V043=YES`; Wasmtime 45.0.2; expiry/review **v0.4.4**).

### Limitations (honest)

- No Kubernetes controller, Helm, or CRD support claimed.
- No full Apache htaccess compatibility claim.
- No open-loop benchmark superiority claim.
- `ghcr.io/exyonq/exyonq:latest` was **not** mutated by this release.

---

## [0.4.2] — private correctness and security maintenance (not published)

```text
P14V042_RELEASE_EXECUTED = NO
PUBLICATION_STATUS = FORBIDDEN
PUBLIC_OPENING = NO
LATEST_CHANGED = NO
BASE_HEAD = 25dcb4e9b323bbb2163c5f28f600175bfe725295
```

### Changed

- OCI/WS6 source revision injection + official fail-closed gate (`unknown` forbidden for official builds).
- Living docs honesty: `v0.4.1` privately published; `v0.4.2` this line.
- TLS PEM parsing via `rustls-pki-types`; remove direct `rustls-pemfile` / RUSTSEC-2025-0134 ignore.
- `serde_json` 1.0.150 → 1.0.151.
- `redis` 0.27.6 → 0.32.7; eliminate transitive `socket2` 0.5.x from the lock.

### Security

- `EXYONQ-SEC-PRIVATE-MATERIAL-ZERO` remains fail-closed.
- Official release builds require canonical 40-hex `EXYONQ_SOURCE_REVISION` (dev may still mark `unknown`).

### Limitations (honest)

- No tag, GitHub Release, GHCR push, `latest` change, or signing in this phase until separate owner close.
- Docker Desktop / Mac results are `LOCAL_ITERATION_ONLY` / `NOT_LINUX_EVIDENCE`.

## [0.4.1] — privately published

```text
P14V041_RELEASE_EXECUTED = YES
P14V041_PUBLICATION_STATUS = PRIVATE_V041_RELEASE_COMPLETE
PUBLIC_OPENING = NO
LATEST_CHANGED = NO
TAG = v0.4.1
FREEZE_HEAD = 43805eb04a79babfbe443e2db4ca0d5b6658c80c
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
- `cargo audit` / `cargo deny` advisories gates run per dependency commit; pre-existing `RUSTSEC-2025-0134` (rustls-pemfile unmaintained) unchanged at freeze.
- Notify 8 removes unmaintained `instant` from the lock graph.

### Limitations (honest)

- Private publication only — no public opening, no `latest` mutation.
- Known follow-up for `0.4.2`: OCI builds that omit an injected source revision embed `source_revision=unknown` (fixed under `P14V042`).
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
