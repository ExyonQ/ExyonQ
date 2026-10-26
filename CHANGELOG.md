# Changelog

All notable changes to ExyonQ are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

```text
P14CLEAN_GITHUB_INITIAL_VERSION = v0.4.0
P14CLEAN_GITHUB_PREVIOUS_LEGACY_VERSION = v0.3.3
P14CLEAN_GITHUB_REUSE_LEGACY_TAGS = NO
P14CLEAN_GITHUB_IMMUTABLE_RELEASE_WARNING_ACK = YES
PUBLICATION_STATUS = PRIVATE_ONLY
PUBLIC_OPENING = NOT_AUTHORIZED
LATEST_CHANGED = NO
```

This repository is a **clean source tree**. Legacy tags from the previous GitHub repository
(`ExyonQ-Old`, last published lineage ending at `v0.3.3`) are **not** reused here because
GitHub immutable release history for the name `ExyonQ/ExyonQ` blocks recreating those tags.

---

## [0.8.0] — published on exyonq-github

```text
RELEASE_AUTHORITY = exyonq-github
PUBLICATION_STATUS = PUBLISHED
PUBLIC_RELEASE = v0.8.0
RELEASE_COMMIT = f814bbf7e206581c1d8fea56e022ef9286d2cf26
GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.8.0
GHCR = ghcr.io/exyonq/exyonq:0.8.0 @ sha256:642e1e79a35dbd65d35402381b182275190e537f9df75a888fb54b536bde44e7
GHCR_WP = ghcr.io/exyonq/exyonq-wordpress:0.8.0 @ sha256:6fa222fca8cc8b9fb1f0059fdecda000a8e478de4ac65de7a5d02b0f2c00d623
DOCKERHUB = exyonq/exyonq:0.8.0 @ sha256:642e1e79a35dbd65d35402381b182275190e537f9df75a888fb54b536bde44e7
DOCKERHUB_WP = exyonq/exyonq-wordpress:0.8.0 @ sha256:6fa222fca8cc8b9fb1f0059fdecda000a8e478de4ac65de7a5d02b0f2c00d623
LINUX_AMD64 = exyonq-v0.8.0-linux-amd64 @ sha256:71f8ce01f46a2d81ed481da1c96ea60cf783de1a2dc801cabae575c808c9e205
LINUX_ARM64 = exyonq-v0.8.0-linux-arm64 @ sha256:08952427c5b28f84de7886fc81dcb24a737683aad2b83f701e8f873420529439
LATEST_CHANGED = NO
TOTAL_COMPAT_PROMISE = FORBIDDEN (Gate A not closed)
```

### Added

- ACME bootstrap from zero: a server configured with ACME starts with no
  certificate on disk; HTTP-01 is answered on the cleartext listener and
  the issued certificate is installed without a restart.
- Route response headers on the wire (`[[route.response_headers]]`, e.g. HSTS).
- Multi-listen: every `[[server]].listen` address is bound and served.
- Full-page cache: direct purge, store/lookup seams, coordinating purge.
- Gates: unsafe-allowlist, publication-privacy. TLS hardening guide.

### Changed

- TLS listeners fail closed while certificate material is pending; a reload
  removing `[server.tls]` intentionally reopens cleartext serving.

### Security

- zstd 0.14.3 / zstd-safe 8.2.0 (RUSTSEC-2026-0391..0406).
- Docker Scout on both images (amd64 + arm64): 0 critical, 0 high, 0 medium.

---

## [Unreleased]

### Notes

- Public **v0.4.11** published on exyonq-github (see below). Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

---

## [0.4.11] — published on exyonq-github

```text
RELEASE_AUTHORITY = exyonq-github
PUBLICATION_STATUS = PUBLISHED
PUBLIC_RELEASE = v0.4.11
RELEASE_COMMIT = 37364553677fd9283bebf9e36ea514e4e157ef9a
GITHUB_RELEASE = https://github.com/ExyonQ/ExyonQ/releases/tag/v0.4.11
GHCR = ghcr.io/exyonq/exyonq:0.4.11 @ sha256:d48d8af7f15b874ffaa14cee652e1e7da364b4d53dc234e45b42c9f9e043da4e
GHCR_WP = ghcr.io/exyonq/exyonq-wordpress:0.4.11 @ sha256:be578cb4e3eafd7032c904e0fa78195f8d469f23a3d2f4f0957d08cf011883e1
LATEST_CHANGED = NO
TOTAL_COMPAT_PROMISE = FORBIDDEN (Gate A not closed)
```

### Added

- **Apache → ExyonQ equivalent importer** (`exyonq-compat-apache` / `exyonq-compat migrate --from apache`): maps `VirtualHost` static sites plus `Alias`, `Redirect`, safe literal `RewriteRule … [R=…]`, `SSLEngine`+certs, authority-only `ProxyPass`, and PHP-FPM (`ProxyPassMatch` / `FilesMatch`+`SetHandler`) onto existing IR (`static` / `redirect` / `tls` / `upstream` / `fcgi_pool`).
- Phase 0 catalog + scope: `docs/compat/APACHE_DIRECTIVE_CATALOG_V1.md`, `docs/compat/compatibility-scope-v1.md`, `compat/apache/APACHE_STATIC_VHOST_MVP.md`.

### Changed

- Workspace product version **0.4.3 → 0.4.11** on this lab integration line (Cargo/CLI/`--version` identity for the 0.4.11 cut).
- Release tooling allowlists accept **0.4.11** (retaining 0.4.0–0.4.10).
- Proxy raw upstream: connect timeout / idle probe / checkout-within-budget (P4/P5 path; kit re-cell pending).
- HTTP/3 provider dependency: yanked `s2n-quic-h3` path replaced with `tachyon-quic` so the workspace resolves.

### Limitations (honest)

- Still **REJECTED_UNSUPPORTED**: `AuthType`/`Require`, `ErrorDocument`, `ProxyPassReverse`, regex `RewriteRule`/`RewriteCond`, `Include` expansion.
- No “full Apache substitution” claim until Gate A corpus on Netcup AMD64 + Oracle ARM64.
- Official AMD64 P4/P5 confirm cells after campaign binary rebuild remain **pending**.

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
