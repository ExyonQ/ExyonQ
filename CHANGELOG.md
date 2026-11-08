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

## [0.9.2] — RELEASE CANDIDATE (not published)

```text
VERSION = 0.9.2
PUBLICATION_STATUS = NOT_PUBLISHED
TAG = LOCAL ONLY, OWNER-GATED
PREVIOUS_PUBLIC = 0.9.1
BASE_TAG = v0.9.1
RELEASE_NOTES = docs/release/v0.9.2.md
```

> **Candidate only.** No published `v0.9.2` tag, GitHub Release, GHCR image,
> or Docker Hub mirror exists. Do not present this entry as a shipped release.

This is the first 0.9.x changelog entry; 0.9.0 and 0.9.1 are documented in
`docs/release/v0.9.0.md` and `docs/release/v0.9.1.md`.

### Added

- **WordPress cache invalidation as a normal plugin.** The integration is
  loaded through the standard plugin lifecycle instead of as a must-use plugin.
  (`ab0e166`)
- **SSH v2 artifact signatures.** Release verification now supports the SSH
  signature format and the release signer trust policy. This is release
  tooling, not a change to the HTTP server. (`4956073`)
- **Artifact-bound OL-04/OL-03 E2E.** A dedicated workflow and negative control
  bind the tested binary hash to the evidence logs. (`731da56`, `524b0d3`)

### Fixed

- **OL-03 OpenLiteSpeed readiness.** The comparison waits for OLS to answer
  HTTP instead of treating an open TCP port as ready, and asserts only the
  documented `perClientConnLimit` contract. (`9550d7e`)

### Security

- **Security audit 2026-11-07 (H-01…H-10).** H-01: removed release notes that
  documented a `allow_sensitive` static option that does not exist. H-02: the
  H1/H2, bounded static-range and benchmark paths share a 64 MiB request-body
  budget and answer `413` before materializing more. H-03: `x-forwarded-host`,
  `x-forwarded-port`, `x-real-ip` and `forwarded` are no longer forwarded
  upstream, so a cached response cannot replay client-supplied forwarding
  metadata to the origin. H-04: the default WAF policy is `mode = "block"`,
  inspects up to 64 MiB and blocks when the inspection budget is exhausted
  (previously `monitor`/1 MiB/`log_and_allow`, which left a body tail
  uninspected). H-05: the quiche provider no longer enables HTTP/3 0-RTT.
  H-06: the security workflows pin `cargo-audit 0.22.2`, `cargo-deny 0.20.2`,
  `cargo-about 0.9.0` and `cargo-cyclonedx 0.5.9` and no longer mask failures
  with `|| true`. H-07: the wire parsers reject control characters and
  `raw_upstream` validates its target before writing. H-09: SNI wildcards match
  a single label and patterns are validated. H-10: static file cloning reports
  an error with `500` instead of panicking.
- H-06 tag/bundle signing is **not** implemented: it needs a release key and
  owner authorization.

### Changed

- **Per-route request-body limit.** `[[route]].max_request_body_bytes` answers
  `413` before WAF inspection and upstream dispatch, so it bounds both body
  materialization and WAF inspection CPU for that route; routes without it keep
  the shared 64 MiB proxy bound.
- WAF/OL-05 evidence records the final revalidation and reCAPTCHA v3 status;
  the positive v3 path still requires a real v3 key. (`38fdb74`)
-  Release evidence for the 0.9.1 image status and security audit was updated
  after the 0.9.1 release. (`4cdad85`)
- Workspace product version **0.9.1 → 0.9.2**; release-manifest and signing
  allowlists accept 0.9.2; packaging defaults use 0.9.2.
- The CI job invokes the OL-03 comparison against
  `litespeedtech/openlitespeed@sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504`;
  the launcher and the comparison reject mutable tags.

### Verification

- E2E workflow run `42331859740` completed successfully on `731da56`; artifact
  `ol04-ol03-artifact-evidence` (30,759 bytes, 54 files) reported
  `RESULT=PASS OL04 OL03 ARTIFACT_BOUND`.
- H-03/H-04 real E2E on 2026-11-07: `scripts/e2e/sec-h03-cache-forwarded-e2e.sh`
  PASS (two clean requests after the poisoned one do not hit the origin again;
  no `evil.example` in the forwarded request) and
  `scripts/e2e/sec-h04-waf-body-tail-e2e.sh` PASS against real CRS v4.25.0
  (`aabf675f`): benign bodies reach the origin and a 942100 payload at the tail
  of a 64 KiB and 2 MiB body (debug build) and of 2 MiB, 8 MiB and 64 MiB
  bodies (release build) is blocked with no `EXY-LIMIT-BODY-001`.
- Observed cost of full-body inspection on the development arm64 host, release
  build: 2 MiB ≈ 1.4 s, 8 MiB ≈ 5.4 s, 64 MiB ≈ 66 s per request. Size
  `max_inspection_body_bytes` to the largest body the protected routes accept.
- Route-body-limit E2E (`scripts/e2e/sec-h04-route-body-limit-e2e.sh`): a 64 MiB
  body on a 2 MiB route answers `413` in well under a second with zero origin
  hits, while the same body on the unset route is still inspected and accepted;
  the 2 MiB boundary stays `200` and a 942100 tail stays `403`.
- Full 0.9.2 release qualification is **NOT EXECUTED**: no tag candidate, full
  CI/security run, release build, image build, or signature verification has
  been run for this draft.

### Limitations (honest)

- The WordPress production-browser journey remains open.
- The positive reCAPTCHA v3 path remains **NOT EXECUTED** without a real v3 key.
- No Windows build is produced, by policy.
- NGINX/Apache/OpenLiteSpeed compatibility remains the documented scope; this
  draft does not claim total parity.

---

## [Unreleased]

### Notes

- Public **v0.4.11** published on exyonq-github (see below). Do not create `v0.3.x`, `v0.2.x`, or `v0.1.x` tags in this repository.

### Added

- **AP-05 route ACL:** `[[route]] access` supports `allow`/`deny` IP/CIDR entries, ordered first-match `rules`, an HTTP-method allowlist, and a UTC `schedule` (including overnight windows). For the list form, `deny` wins and a non-empty `allow` list is an allowlist. Denials return `403` before auth/content dispatch and are rechecked after internal rewrites. Common Apache `Require ip`/`method` and `Order`/`Allow`/`Deny` forms, nginx `allow`/`deny` with exact first-match order, and `.htaccess` `Require ip`/`method` now map to the same policy; unrepresentable forms fail closed.

### Fixed

- `cache_headers.by_mime` now accepts `max_age = 0` and emits `Cache-Control: no-cache`; `immutable = true` with zero is rejected.
- Routes with `rewrite` or `redirect` now use the compiled rewrite engine in the full dispatcher, including pattern captures; external redirects preserve the original query unless the replacement declares its own `?`.
- systemd `RuntimeDirectory` and the control socket use `%p`, so the single-instance unit keeps `/run/exyonq` and the two production units get separate `/run/exyonq-http` and `/run/exyonq-sites` paths.

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

- Still **REJECTED_UNSUPPORTED**: `Require env/group/expr`, `Satisfy any`, `ErrorDocument`, `ProxyPassReverse`, regex `RewriteRule`/`RewriteCond`, `Include` expansion. Basic `AuthType`/`Require valid-user` and common IP/method ACLs are migrated.
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
