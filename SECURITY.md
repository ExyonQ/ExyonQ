# Security Policy

## Supported versions

| Version   | Supported |
|-----------|-----------|
| 0.3.x     | Yes       |
| < 0.3     | No        |

## Current audit

Structured review for the latest release: [`docs/security/audit-v0.3.2.1.md`](docs/security/audit-v0.3.2.1.md).

## Reporting a vulnerability

Email security reports to the maintainers (see repository contacts). Do not open public issues for exploitable vulnerabilities.

Include:

- Affected version and commit
- Reproduction steps or proof-of-concept
- Impact assessment

We aim to acknowledge reports within 72 hours.

## Scope

In scope: ExyonQ core HTTP parser, static file resolver, reverse proxy, config reload, TLS termination, HTTP/3 QUIC, control socket (`exyonqctl`), official modules (metrics, compression, ratelimit).

Out of scope: third-party benchmark containers, rival server configurations, compatibility importers except when they emit unsafe native config.

### In-scope surfaces (0.1+)

| Surface | Location |
|---------|----------|
| HTTP/1 raw + hyper | `core/src/server/` |
| Static files | `core/src/static_files/` |
| Reverse proxy | `core/src/proxy/` |
| Hot reload | `core/src/reload/` |
| Control plane | `core/src/control/`, `cli/exyonqctl/` |
| TLS / HTTP/2 | `core/src/tls/` |
| HTTP/3 | `core/src/http3/` |
| Discovery | `core/src/discovery/` |
| Modules | `modules/*` |
| WASM plugin host | `wasm/exyonq-wasm-host/` |

## Release security gate

Each tag `vX.Y.Z` requires:

1. Green CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml))
2. Green security workflow ([`.github/workflows/security.yml`](.github/workflows/security.yml))
3. Audit artifact [`docs/security/audit-vX.Y.Z.md`](docs/security/audit-template.md) with no open **Blockers**
4. Nightly fuzz/Miri monitored ([`.github/workflows/security-nightly.yml`](.github/workflows/security-nightly.yml))

Local pre-tag check: `cargo xtask security pre-release --version X.Y.Z`

Checklist: [`docs/security/release-checklist.md`](docs/security/release-checklist.md)

## Blocking criteria

| Severity | Release policy |
|----------|----------------|
| Critical / High | **Block** — must fix before tag |
| Medium | Fix **or** documented waiver in version audit |
| Low | Backlog OK with linked issue |

## Hardening baseline

- Request header size capped (`MAX_HEADER` / `MAX_HEADER_CAP` = 8192 bytes)
- Header read timeout: `EXYONQ_READ_TIMEOUT_MS` (default 30000 ms) on static wire loop
- Path traversal blocked via canonical root checks
- Hop-by-hop headers stripped on proxy
- Duplicate / conflicting length headers rejected before proxy forward
- `.htaccess` never parsed at runtime (offline migration only)

## Trusted configuration

These environment variables are **trusted** (same privilege as the process). Compromise allows config or upstream redirection:

- `EXYONQ_CONFIG` — config file path for serve/reload
- `EXYONQ_CONTROL_SOCKET` — Unix socket for `exyonqctl`
- `EXYONQ_DISCOVERY_FILE`, `EXYONQ_DISCOVERY_K8S`, `EXYONQ_DISCOVERY_DOCKER` — discovery overlay paths

Run ExyonQ with minimal env exposure in production.

## Automated testing

- Integration security tests: `cargo nextest run -p exyonq-integration-tests --filter-expr 'test(security) or test(wasm_host)'`
- Fuzz: `scripts/security-fuzz.sh` (see [`fuzz/`](fuzz/))
- Dependency audit: `cargo audit`, `cargo deny check`
