# ExyonQ security audit — v0.4.3

| Field | Value |
|-------|-------|
| **Version** | 0.4.3 |
| **Commit** | `e8fa4dca9336bb8293b56ca8ad47e24bfd4fd792` |
| **Date** | 2026-08-04 |
| **Previous audit** | N/A (first release-track audit for v0.4.3; prior product-qualified revision `2f8e869…` is historical pre-R4 only) |

## Summary

Security audit for ExyonQ release track **v0.4.3**. Workspace Cargo / CLI product
version is **0.4.3**. Pre-bump freeze tip `e63325d53f1fb0848a37e55f8a52fc9f6409739e`
is historical qualification evidence only after the version bump (binary bytes change).
No Critical or High product security findings are open. One dependency advisory remains
**unfixed** and **waived for v0.4.3 only**: RUSTSEC-2026-0222 (Wasmtime 45.0.2).
Private-material, integrity, waiver-register, and pre-R5 GRC readiness gates must PASS
on the post-bump tip. This document is release evidence for R5 freeze qualification; it
does **not** authorize tag, production signing, push, GHCR, or public release.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | pass (local matrix) | Darwin fmt/check/clippy/tests PASS (`r5-darwin-20260803T162707Z`); Linux amd64+arm64 workspace/check/clippy/tests PASS (composites) |
| Security (`security.yml` local equiv.) | pass | `cargo audit` EC0 (advisory ignored under registered waiver); `cargo deny check` EC0; PMZ `--git-tree` PASS; waiver-expiry PASS |
| Nightly fuzz | skipped | Not re-executed in this freeze window; no parser surface change claimed beyond R4 endpoint-set/WRR already dual-arch tested |
| Functional F1–F13 | pass (via BPC/E2E) | `BASIC_PRODUCT_COMPLETENESS_GATE=PASS` reconfirmed Darwin; Linux real E2E static/proxy/tls/http2/fastcgi PASS dual-arch |
| WASM host tests | pass | Included in workspace test matrix Darwin + Linux |
| Benchmark Tier A | N/A | Official `bench compare` not required to close this security audit; R3 competitive PARITY retained as HISTORICAL for competitive claims |

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| P2B-OPEN-002 (eligible==1 Multi→Single) | Medium (correctness) | Closed in product `1d815fe`; Linux protectors tip-bound PASS |
| BENCH_API_CACHE_PATHS / integrity remediation lineage | Critical (integrity) | Removed earlier on product line; integrity gates PASS on tip |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| RUSTSEC-2026-0222 | Low | Temporary v0.4.3 waiver; Wasmtime remains 45.0.2; **VULNERABILITY_FIXED=NO** | `deny.toml` + `.cargo/audit.toml` ignore; reachability rationale in exception register; revisit ≥46.0.2 | v0.4.4 |

```text
RUSTSEC_2026_0222_EXCEPTION_STATUS = REGISTERED
VULNERABILITY_FIXED = NO
WAIVED_FOR_V043 = YES
Wasmtime = 45.0.2
EXPIRY_REVIEW = v0.4.4
CURRENT_RELEASE_WAIVERS_UNREGISTERED = 0
```

Do not describe RUSTSEC-2026-0222 as fixed, closed, or remediated while this waiver is active.
Exception SoT: [`docs/governance/exceptions/RUSTSEC-2026-0222.md`](../governance/exceptions/RUSTSEC-2026-0222.md).

## Surfaces reviewed

- [x] HTTP/1.1 raw static loop (Darwin + Linux E2E / BPC)
- [x] Hyper proxy path (Linux E2E + productive WRR protectors tip-bound)
- [x] Static path resolver (E2E static)
- [x] Reverse proxy headers (proxy E2E / module-api tests)
- [x] Config parse + reload + control socket (workspace tests; endpoint-set IR tests)
- [x] TLS / HTTP/2 (Linux E2E)
- [x] HTTP/3 QUIC (workspace/security suite lineage; POST-body fix retained)
- [x] Discovery env overlays (clippy fix surface `07273e7`; unit coverage)
- [x] Modules (metrics, compression, ratelimit) — workspace tests
- [x] `unsafe` sendfile / mmap — no new `unsafe` introduced on freeze tip vs R4 product commits; prior Phase 0 freeze constraints retained

## Evidence binding (local)

| Claim | Host / arch | Run / path |
|-------|-------------|------------|
| Darwin freeze gates | Darwin / local | `.exyonq-local/release/r5-freeze-6270968/darwin/r5-darwin-20260803T162707Z-*` |
| Linux amd64 qual | Netcup amd64 | `.exyonq-local/release/r5-freeze-6270968/evidence/R5_AMD64_COMPOSITE.txt` |
| Linux arm64 qual | Oracle arm64 | `.exyonq-local/release/r5-freeze-6270968/evidence/R5_ARM64_COMPOSITE.txt` |
| WRR protectors | Netcup+Oracle | `r5-wrr-20260803T162708Z` |
| Security bundle | Darwin | `.exyonq-local/release/r5-freeze-6270968/security/` |
| OCI E2E | local dual-arch | `oci/verdict-20260803T182753Z.txt` + proxy supplemental |

```text
CRITICAL_SECURITY_FINDINGS_OPEN = 0
HIGH_SECURITY_FINDINGS_OPEN = 0
R5_SECURITY_AUDIT_STATUS = PASS (document complete; verifier required)
PRODUCTION_SIGNING_EXECUTED = NO
PUBLICATION_STATUS = FORBIDDEN
```

## Sign-off

- **Reviewer:** R5 freeze qualification (security audit execution under owner authorization)
- **Date:** 2026-08-03
- **Notes:** Audit Commit binds to version-bump tip `e8fa4dca9336bb8293b56ca8ad47e24bfd4fd792`; pre-bump freeze `e63325d…` remains historical. Tag / push / production signing / GHCR / GitHub Release remain **FORBIDDEN** until separate owner ceremony.
