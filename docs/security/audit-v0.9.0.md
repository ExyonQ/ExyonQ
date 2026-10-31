# Security audit — v0.9.0 (NG-18 Phase B candidate)

**Status:** DRAFT — release blocked (independent gates not run)
**Date:** 2026-10-27
**Scope:** NG-18 Phase B (`gunzip`), Q-15 metrics labels, NG-17 cache headers, OL-02 nonce-safe FPC

## Automated gates

| Gate | Result |
| --- | --- |
| `cargo audit` | PASS (519 crates, no advisories) |
| `cargo deny check` | PASS (advisories/bans/licenses/sources) |
| `scan-private-material.sh --git-tree` | PASS (0 findings) |
| Docker Scout `exyonq:ng18-amd64` | 0 Critical / 0 High / 0 Medium / 0 Low |
| Docker Scout `exyonq:ng18-arm64` | 0 Critical / 0 High / 0 Medium / 0 Low |
| `cargo fmt --check` | PASS |
| Container E2E server amd64 (Netcup) | PASS (static, proxy, gunzip, passthrough, invalid-gzip abort) |
| Container E2E server arm64 (Oracle) | PASS (same matrix) |
| Container E2E WordPress amd64 (Netcup) | PASS (install, login, wp-admin 200, FPC hit, nonce POST, bypass) |
| Container E2E WordPress arm64 (Mac) | PASS (same matrix) |
| Gunzip differential (420 rows) | PASS local, Netcup amd64, Oracle arm64 |

## Surfaces reviewed

- `core/src/gunzip.rs`: streaming gzip decoder, bounded output, CRC/ISIZE, concatenated members, invalid-stream body error.
- `core/src/server/handler.rs`: final `BoxError` boundary, Hyper abort on invalid gzip, route/upstream labels, cache headers.
- `compat/nginx`: `gunzip`/`gunzip_buffers` http/server/location + inheritance; importer now `Supported`.
- `modules/metrics`: seconds histogram, route/upstream labels, 64-series cap.
- `config/ir`: `GunzipConfig`, route override, validation.

## Open blockers (release)

- `exyonq-security-compiler` verdict: **NOT RUN**.
- `contract-adversarial-auditor` independent pass: **NOT RUN**.
- NG-18 Phases C/D/E (`sub_filter`, `mirror`, `slice`): **OPEN**.
- Roadmap 0.5–0.8 `❌`/`⚠️` residuals: **OPEN**.
- Version bump / tag / release: **NOT DONE**.

## Verdict

**RELEASE BLOCKED** until the independent audits and the remaining NG-18 phases close.
