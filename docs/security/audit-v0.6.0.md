# ExyonQ security audit — v0.6.0

| Field | Value |
| --- | --- |
| **Version** | 0.6.0 |
| **Commit** | `39c85c1ec2a1754446f307df211a614adef0b247` |
| **Date** | 2026-10-18 |
| **Previous audit** | [`audit-v0.5.0.md`](audit-v0.5.0.md) |

## Summary

0.6.0 publishes operational surfaces (htaccess fail-closed diagnostics, cache observability, WebSocket reload/drain, config explain provenance, reload route diff, FastCGI pool diagnostics, and related ADRs 055–062) as a versioned public cut on top of the already-published 0.5.0 tree plus post-tag dependency pins on `main` (Dependabot cargo/GHA + coordinated OpenTelemetry 0.33). Container start and WordPress account/login checks on linux/amd64 and linux/arm64 are mandatory before tagging images. Wasmtime 49.0.2 advisory findings remain open from 0.5.0 unless cleared in this cut.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | pending | Runs after push/tag |
| Security (`security.yml`) | expect deny advisories fail | Wasmtime 49.0.2 (carry-forward) |
| Nightly fuzz | skipped | Not claimed for this cut |
| Container E2E | required before push | Server GET `/` → 200; WordPress account create and dashboard login; amd64 and arm64 |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None for publication of the images that pass the container E2E gate. Wasmtime advisories stay recorded, not waived.

## Findings (open)

### 1. Wasmtime 49.0.2 component-model advisories

- **Severidad:** alta
- **Archivo:** `Cargo.lock` (crate `wasmtime` 49.0.2)
- **Condición:** `cargo deny check advisories`
- **Impacto:** same set as audit-v0.5.0 (12 vulnerability errors)
- **Patch mínimo:** bump Wasmtime in development tree, then sync
- **Estado:** open (carry-forward)

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| Addon core range for 0.6.x | high | `official_core_compat` → `>=0.1.0, <0.7.0` |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| | | | | |

## Surfaces reviewed

- [x] Config parse + reload + control socket
- [x] Modules (metrics/compression/ratelimit handshake with 0.6 core)
- [x] Cache observability headers
- [x] FCGI pool diagnostics
- [x] htaccess fail-closed
- [ ] HTTP/1.1 raw static loop (unchanged vs 0.5.0)
- [ ] Hyper proxy path (unchanged vs 0.5.0)
- [ ] TLS / HTTP/2 / HTTP/3 (unchanged vs 0.5.0)

## Sign-off

- **Reviewer:** ExyonQ Maintainer
- **Date:** 2026-10-18
- **Notes:** Commit bound to the published v0.6.0 source revision. Wasmtime advisories remain open.
