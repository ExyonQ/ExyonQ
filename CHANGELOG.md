# Changelog

All notable changes to ExyonQ are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

```text
CHANGELOG_STATUS = RC_READY
TARGET_RC_VERSION = 0.3.3-rc.1
RC_STATUS = NOT_DECLARED
PUBLICATION_STATUS = FORBIDDEN
```

Entries below group **demonstrated product capabilities** from P1.1–P1.6 closures.
They are **not** a commit-by-commit dump. This section prepares the informative packet for a future RC; it does **not** declare that the candidate exists.

---

## [0.3.3-rc.1] — DRAFT (not declared)

### Added

- Native configuration IR (TOML) with `config_version` v1/v2 loading and documented migration (`exyonq migrate`, schema lint/test/format/explain).
- Serverfile (`.exy`) compilation to IR with product profiles (static, proxy, php, wordpress).
- Control-plane operator surface via `exyonqctl` (reload check/diff, drain, shutdown, status, generation).
- FastCGI / PHP backend path with WordPress-oriented profile (cache off by default).
- HTTP/1, TLS termination, HTTP/2, and HTTP/3 (QUIC) data-plane capabilities with documented limits.
- Static file serving and reverse-proxy backends as first-class module owners.
- NGINX import path (`exyonqctl config migrate-nginx`) with fail-closed unknown-field policy — not total NGINX compatibility.
- Packaging contracts: Linux amd64/arm64 tarball naming, build manifest/SBOM/license packaging docs, nfpm for amd64 deb/rpm.
- Dependency containment live gate remediation to ERROR logical=0 / raw=0 (P1.6-WS2).
- Workspace flake containment for cache metrics, Redis Docker isolation, HMAC env tests, wasm epoch timing (P1.6-WS2C).

### Changed

- Operator config CLI canonical path is `exyonqctl config …`; legacy `exyonq` config subcommands remain available under deprecation policy (not removed in WS3).
- Readiness and liveness split: `/ready` vs `/live` (and related probe semantics) per P1.5 operations contract.
- Drain: after drain, new non-probe requests return **503**; process-level **SIGTERM** shutdown semantics documented.
- H3 reload limited to same-listener configuration changes; listener/protocol changes beyond limit require restart.
- Release honesty: signing designed but not provisioned; bit-for-bit reproducibility not claimed; arm64 nfpm deferred.

### Deprecated

- Legacy top-level `exyonq` config commands that duplicate `exyonqctl config …` (see deprecation policy). Migration path documented; removal not performed in P1.6-WS3.

### Removed

- Nothing operator-facing removed in the P1.6-WS3 informative packet. Prior kernel dispatch removals (e.g. HandlerTable / static_roots) remain historical architecture closure — not reopened here.

### Fixed

- NGINX import integration compile/runtime fixture gaps (KF-P16-001).
- Dependency containment admissions/baseline drift (KF-P16-002).
- Integration tests missing module bootstrap causing false 501 on Static (KF-P16-007 class).
- Parallel workspace races: cache size metrics, Redis container collision, HMAC wire/env, wasm epoch flake (KF-P16-008…011).

### Security

- Inherited P1.5 security soak / fuzz / threat-model closures with documented H3/H2/htaccess/NGINX limits.
- Public signing remains **DESIGNED_NOT_PROVISIONED** (KF-P16-003) — not a silent PASS.
- Fail-closed unknown config fields; importer expansion and total-compat promises forbidden.

### Performance

- No new competitive performance claim in P1.6. Official comparative publication remains a later block (P1.7 / separate admit). Historical v0.3.3 perf policy docs remain historical context only.

### Operations

- Upgrade/rollback operator guidance aligned with package layout, checksums, reload vs restart, `/ready`/`/live`, drain→503, SIGTERM.
- Dual-arch Linux evidence model: Netcup amd64 + Oracle arm64; macOS/Docker Desktop = local iteration only.

### Packaging

- Artifact naming contract (versioned `.tar.gz` primary; legacy unversioned aliases transitional).
- arm64 `.deb`/`.rpm` via nfpm = deferred with documented limit (KF-P16-004); tarball primary on arm64.

### Known limitations

- `FULL_HTACCESS_COMPATIBILITY = NO`
- `TOTAL_NGINX_COMPATIBILITY = NO` / `TOTAL_COMPAT_PROMISE = FORBIDDEN`
- `H3_RELOAD_SUPPORT = SAME_LISTENER_CONFIG_ONLY`
- `H3_SECURITY = WITH_DOCUMENTED_LIMITS`
- `WORDPRESS_CACHE = OFF`
- `LITESPEED_IMPORTER = DEFERRED`
- `CADDY_IMPORTER = DEFERRED`
- `BIT_FOR_BIT_CLAIM = NOT_CLAIMED`
- `SIGNING_STATUS = DESIGNED_NOT_PROVISIONED`
- See [p1.6-known-issues.md](docs/release/p1.6-known-issues.md) and [p1.6-rc-product-claims.md](docs/release/p1.6-rc-product-claims.md).

---

## [Unreleased]

Reserved for post-RC development. Do not treat workspace `0.3.3` commits after this draft as an undeclared RC.

---

## Historical notes

Prior narrative release notes (not Keep a Changelog format):

- [docs/release/v0.3.3.md](docs/release/v0.3.3.md)
- [docs/release/v0.3.2.1.md](docs/release/v0.3.2.1.md)
