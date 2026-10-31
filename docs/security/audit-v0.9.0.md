# ExyonQ security audit — v0.9.0

| Field | Value |
| --- | --- |
| **Version** | 0.9.0 |
| **Commit** | `1945f6299c73269bb2bc9fe26197559a979fc9f3` |
| **Date** | 2026-10-31 |
| **Previous audit** | [`audit-v0.7.0.md`](audit-v0.7.0.md) |

## Summary

v0.9.0 is the F5 cache and content-delivery parity release: NG-15/16 proxy-cache keys/vary/bypass/stale-if-error/tag+URL purge/`exyonqctl` cache tooling, OL-01 private-cookie hard-deny + `Set-Cookie`/`no-store` exclusion, OL-02 dynamic fragments, NG-17 header behavior, Q-15 observability, NG-18 body transforms (gunzip, sub_filter, mirror, Range/206/If-Range, slice cache segments) and the WAF body-inspection surface (`crs`/`multipart`/`xml`, `waf_metrics`, `zone_ratelimit`, `auth_basic`). `mirror_request_body on` remains intentionally Unsupported (bounded scope). Differential oracles: gunzip 420/420 on amd64 and arm64, sub_filter 8/8, mirror PASS, slice E2E 4 MiB/4 segments on real Linux hosts. `cargo audit` and `cargo deny check` are clean; no open Critical or High findings; no waivers. Public CI (`ci.yml` + `security.yml`) runs green on the landed tree. Independent audits (`contract-adversarial-auditor`, `exyonq-security-compiler`) and the official sequential benchmark are pending and reported as NOT RUN — not as pass.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| Public CI (`ci.yml`) | pass | run `41221976679` on the landed tree: fmt + clippy `-Dwarnings` + nextest **2111/2111** on Linux x86_64; `semver-checks`, `supply-chain`, `wasm-host-check` green |
| Security (`security.yml`) | pass | run `41221976663`: integrity-gates, private-material, functional-security-check, deps-audit, legal-compliance, security-tests all green (first public run of the synced workflow) |
| `cargo audit` | pass | 0 vulnerabilities, 526 crates scanned (1395 advisories db), 2026-10-31 |
| `cargo deny check` | pass | advisories/bans/licenses/sources ok; warnings only (unmatched `OpenSSL` allowance, base64 0.22/0.23 duplicate), 2026-10-31 |
| Security functional tests | pass | `xtask security pre-release`: `security.rs` 11/11, `wasm_host.rs` 3/3 (fuel/epoch/memory traps) |
| Release legal bundle | pass | `verify-release-legal-bundle.sh` + third-party notices generation OK |
| Nightly fuzz | skipped | not re-run for this release; NG-18 parsers (gzip stream, sub_filter, multipart) reuse existing fuzz surfaces; reported honestly as SKIPPED |
| Linux E2E — container server | pass | amd64 Netcup + arm64 Oracle: static, proxy, gunzip, passthrough, invalid-gzip abort |
| Linux E2E — WordPress | pass | amd64 Netcup + arm64 Mac: installer UI+assets, admin account created, login → `wordpress_logged_in` cookie → `/wp-admin/` 200, FPC miss→hit, nonce POST, logged-in bypass |
| NG-18 Phase A oracle | pass | NGINX real on Mac, Netcup amd64, Oracle arm64: 420 rows, 20 expected aborts (invalid gzip) |
| NG-18 Phase B gunzip differential | pass | 420/420 local (two consecutive runs), 420/420 Netcup amd64 + Oracle arm64 |
| NG-18 Phase C sub_filter differential | pass | 8/8 rows vs NGINX (HTML, text/plain, `once`, multi-rule, case, large/chunks, Last-Modified on/off) |
| NG-18 Phase D mirror differential | pass | identical main body; sink receives exactly one `GET /mirror` on both |
| NG-18 Phase E slice E2E | pass | 4 MiB → 4 cache segments, second request fully cached, 0 extra ranges — Netcup amd64 + Oracle arm64 with the real image |
| Docker Scout | pass | `exyonq:ng18-amd64` + `exyonq:ng18-arm64`: 0 Critical / 0 High / 0 Medium / 0 Low |
| Veyra Proof audit | skipped | not executed for 0.9.0 — reported SKIPPED, not claimed green |
| Official sequential benchmark | pending | `bench compare --all-servers` on Netcup amd64 + Oracle arm64 scheduled before tag |
| contract-adversarial-auditor | not run | independent contract audit pending before final verdict |
| exyonq-security-compiler verdict | not run | compiler verdict pending maintainer sign-off before tag |

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| ADDON-COMPAT-090 | Medium | `official_core_compat` bound `>=0.1.0, <0.9.0` rejected the 0.9.0 core at addon handshake — found by public CI (`metrics`/`ratelimit` integration tests), extended to `<0.10.0` and verified locally |
| NG-18 phases C/D/E | — | `sub_filter`, `mirror`, `Range`/`slice` landed after the draft audit: all differentials PASS; `mirror_request_body on` documented Unsupported |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Surfaces reviewed

- [x] HTTP/1.1 raw static loop (Range/206/If-Range on 200, slice segment assembly)
- [x] Hyper proxy path (cache-key equivalence NG-15, stale-if-error NG-16, safe client-header forwarding)
- [x] Static path resolver (unchanged semantics; NG-18 only on proxy/cache paths)
- [x] Reverse proxy headers (NG-17 behavior matrix)
- [x] Config parse + reload + control socket (`exyonqctl cache explain`/`purge`, purge site-ID binding)
- [x] TLS / HTTP/2 (no changes this cycle; wasmtime 49.0.4→49.0.5 RUSTSEC-2026-0422/0423/0424)
- [x] HTTP/3 QUIC (provider selection unchanged)
- [x] Cache privacy (OL-01: private/`Set-Cookie`/`no-store`/`no-cache` never cached; WordPress/session cookie hard-deny)
- [x] Dynamic fragments (OL-02: nonce-safe FPC — cached anonymous nonce accepted cross-client)
- [x] Modules (metrics Q-15 labels, compression, ratelimit; `zone_ratelimit`, `auth_basic` new surfaces)
- [x] WAF body inspection (`crs`, `multipart`, `xml`, `waf_body`, `waf_metrics`)
- [x] NG-18 transforms (`gunzip` streaming bounded decoder multi-member CRC/ISIZE; `sub_filter` streaming multi-rule; `mirror` anti-recursion guard empty body)
- [x] `unsafe` sendfile / mmap (no new unsafe this cycle beyond existing audited surfaces)

## Unsafe changes since previous release

No new `unsafe` blocks introduced by the v0.9.0 feature set (NG-15..18, OL-01/02, Q-15, WAF body surface). Validation: nextest 2111/2111 on public CI, real-Linux E2E on both architectures, differential oracles above. Miri/fuzz nightly not re-run — recorded as SKIPPED, not claimed green.

## Sign-off

- **Reviewer:** release pipeline (gates above, reproducible evidence in repo scripts + `.exyonq-local/`)
- **Date:** 2026-10-31
- **Notes:** Formal `exyonq-security-compiler` verdict, `contract-adversarial-auditor` pass and official sequential benchmark remain pending before tag — reported honestly as NOT RUN / pending, never as pass. `OPEN_PROVEN_PRODUCT_DEFECTS = 0` at this writing.
