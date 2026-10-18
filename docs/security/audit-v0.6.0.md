# ExyonQ security audit — v0.6.0

| Field | Value |
|-------|-------|
| **Version** | 0.6.0 |
| **Commit** | `_pending_publish_commit_` |
| **Date** | 2026-10-18 |
| **Previous audit** | [`audit-v0.5.0.md`](audit-v0.5.0.md) |

## Summary

0.6.0 publishes the F2 operational-confidence surface (Q-03…Q-08 and related ADRs 055–062) as a versioned public cut on top of the already-published 0.5.0 tree plus post-tag dependency pins on `main` (Dependabot cargo/GHA + coordinated OpenTelemetry 0.33). Container E2E gate 134 is mandatory before tag/GHCR. Wasmtime 49.0.2 advisory findings remain open from 0.5.0 unless cleared in this cut.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | pending | Runs after push/tag |
| Security (`security.yml`) | expect deny advisories fail | Wasmtime 49.0.2 (carry-forward) |
| Nightly fuzz | skipped | Not claimed for this cut |
| F2 lab E2E scripts | PASS in lab (2026-10-18) | `scripts/reality/q03`…`q08` + expires |
| Container E2E (134) | **required** | amd64+arm64; WP account+login |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None once container E2E 134 passes on the digests that will be pushed. Wasmtime advisories stay recorded, not waived.

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
|----|----------|------------|
| Addon core range for 0.6.x | high | `official_core_compat` → `>=0.1.0, <0.7.0` |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| | | | | |

## Surfaces reviewed

- [x] Config parse + reload + control socket (Q-06/Q-07)
- [x] Modules (metrics/compression/ratelimit handshake with 0.6 core)
- [x] Cache observability headers (Q-04)
- [x] FCGI pool diagnostics (Q-08)
- [x] htaccess fail-closed (Q-03)
- [ ] HTTP/1.1 raw static loop (unchanged vs 0.5.0)
- [ ] Hyper proxy path (unchanged vs 0.5.0)
- [ ] TLS / HTTP/2 / HTTP/3 (unchanged vs 0.5.0)

## Sign-off

- **Reviewer:** pending maintainer
- **Date:** 2026-10-18
- **Notes:** Prep audit; finalize commit SHA and E2E evidence paths before tag.
