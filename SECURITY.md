# Security Policy

## Supported versions

| Version   | Supported |
| --- | --- |
| 0.7.x     | Yes (current release) |
| 0.6.x     | Security fixes only until superseded |
| < 0.6     | No |

## Current audit

Release tags require a tracked audit artifact:

[`docs/security/audit-vX.Y.Z.md`](docs/security/audit-template.md)

created from the template and verified with:

```bash
bash scripts/verify-release-audit.sh X.Y.Z
# or
cargo xtask security pre-release --version X.Y.Z
```

Until a release audit is owner-authorized and signed off for a given version, that version remains unpublished.

For **v0.7.0**:

```text
CURRENT_RELEASE_AUDIT = docs/security/audit-v0.7.0.md
PUBLIC_RELEASE = v0.7.0
RELEASE_AUTHORITY = exyonq-github
WAIVERS = NONE
CARGO_AUDIT = 0 vulnerabilities (516 crates scanned, 2026-10-24)
CARGO_DENY = advisories/bans/licenses/sources PASS
DOCKER_SCOUT = 0 critical / 0 high / 0 medium (exyonq + exyonq-wordpress, amd64 + arm64)
```

Canonical security SoT index: [`docs/security/README.md`](docs/security/README.md).

## Reporting a vulnerability

Email **security@exyonq.org**. Do not open public issues for exploitable vulnerabilities.

Include:

- Affected version and commit
- Reproduction steps or proof-of-concept
- Impact assessment

We aim to acknowledge reports within 72 hours.

## Scope

In scope: ExyonQ core HTTP parser, static file resolver, reverse proxy, config reload, TLS termination, HTTP/3 QUIC, control socket (`exyonqctl`), WAF, FastCGI, `.htaccess` overlay compiler, cache layer, official modules (metrics, compression, ratelimit, ACME), WASM host.

Out of scope: third-party benchmark containers, rival server configurations, compatibility importers except when they emit unsafe native config.

### In-scope surfaces

| Surface | Location |
| --- | --- |
| HTTP/1 raw + hyper | `core/src/server/` |
| Static files | `crates/exyonq-mod-static/` |
| Reverse proxy | `crates/exyonq-mod-proxy/` |
| Hot reload | `core/src/reload/`, `crates/exyonq-reload-runtime/` |
| Control plane | `crates/exyonq-ops-runtime/`, `crates/exyonq-control-api/`, `cli/exyonqctl/` |
| TLS | `crates/exyonq-mod-tls/` |
| HTTP/3 | `crates/exyonq-mod-http3/` |
| Discovery | `crates/exyonq-discovery-runtime/` |
| WAF | `crates/exyonq-waf/` |
| FastCGI | `crates/exyonq-mod-fastcgi/` |
| `.htaccess` overlay | `crates/exyonq-mod-htaccess/` |
| Cache | `crates/exyonq-cache/`, `crates/exyonq-cache-redis/` |
| Modules | `modules/*` |
| WASM plugin host | `wasm/exyonq-wasm-host/` |

## Release security gate

Each tag `vX.Y.Z` requires:

1. Audit artifact [`docs/security/audit-vX.Y.Z.md`](docs/security/audit-template.md) with no open **Blockers**
2. `cargo audit` and `cargo deny check` clean
3. Workspace test suite green
4. Private-material-zero PASS ([`docs/security/private-material-zero.md`](docs/security/private-material-zero.md))
5. Integrity triad PASS (project / benchmark / data-provenance gates)
6. Container images scanned clean (0 critical / high / medium) on both `linux/amd64` and `linux/arm64` before OCI publication
7. Image E2E on real Linux amd64 and arm64 hosts before registry publication

Local pre-tag check: `cargo xtask security pre-release --version X.Y.Z`

Checklist: [`docs/security/release-checklist.md`](docs/security/release-checklist.md)

## Blocking criteria

| Severity | Release policy |
| --- | --- |
| Critical / High | **Block** — must fix before tag |
| Medium | Fix **or** documented waiver in version audit + exception register |
| Low | Backlog OK with linked issue |

Waiver field requirements: [`docs/security/waiver-requirements.md`](docs/security/waiver-requirements.md).

## Hardening baseline

- Request header size capped (`MAX_HEADER` / `MAX_HEADER_CAP` = 8192 bytes)
- Header read timeout: `EXYONQ_READ_TIMEOUT_MS` (default 30000 ms) on static wire loop
- Path traversal blocked via canonical root checks
- Hop-by-hop headers stripped on proxy
- Duplicate / conflicting length headers rejected before proxy forward
- WAF fails closed on malformed input (no fail-open on undecodable bytes)
- `.htaccess` handled via offline compile + atomic overlay publish with bounded budgets — never ad-hoc per-request parsing
- Cache-purge socket is Unix-domain only with token authentication
- No committed TLS private keys (ephemeral generation only)

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
- Private material: `bash scripts/security/scan-private-material.sh --git-tree --repo .`
