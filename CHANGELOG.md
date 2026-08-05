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

## [Unreleased]

### Notes

- Follow-up after private **`0.4.3`** (this line). Publication/signing remain owner-gated.
- Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

---

## [0.4.3] — private release candidate (not published)

```text
PUBLICATION_STATUS = FORBIDDEN
PUBLIC_OPENING = NO
LATEST_CHANGED = NO
PRODUCTION_SIGNING_EXECUTED = NO
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

- `docs/security/audit-v0.4.3.md` freeze-track audit executed (release evidence; does not authorize tag/sign/publish).
- RUSTSEC-2026-0222 remains **unfixed** and **waived for v0.4.3 only** (Wasmtime 45.0.2).

### Limitations (honest)

- No Kubernetes controller, Helm, or CRD support claimed.
- No full Apache htaccess compatibility claim.
- No open-loop benchmark superiority claim.
- Tag, GitHub Release, GHCR push, `latest`, and production signing remain owner-gated.

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
