# ExyonQ security audit — v0.4.12

| Field | Value |
| --- | --- |
| **Version** | 0.4.12 |
| **Commit** | `2c9de255b1963878aa6085f310dc28a45010954d` |
| **Date** | 2026-10-12 |
| **Previous audit** | [`audit-v0.4.11.md`](audit-v0.4.11.md) |

## Summary

0.4.12 continues the public GitHub publish line from `main` @ `46935e068c6aeb91ed6add96f235a41ec6bf016a` (post Dependabot [#40](https://github.com/ExyonQ/ExyonQ/pull/40) `wat` 1.261.0 and [#43](https://github.com/ExyonQ/ExyonQ/pull/43) `crypto-common` 0.1.7) with product version **0.4.12**. No new product runtime surface. Workspace Wasmtime remains `=49.0.2`.

Lineage: based on public `main`, **not** lab `release/v0.4.4-integration` tip (unrelated histories; Netcup dataplane / Phase1 close not merged).

Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | not run on this commit | Tag workflow runs after publish |
| Security (`security.yml`) | host `cargo audit` | Required clean before tag |
| Nightly fuzz | skipped | Not run for this cut |
| WASM host tests | host required | `cargo test -p exyonq-wasm-host` |
| Container start-checks | required (rule 131) | `exyonq` GET `/` → 200; wordpress installer; amd64+arm64 |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None for a deps/version patch cut, contingent on: host audit clean, wasm-host tests PASS, dual-arch container start-checks PASS before tag/GHCR/release assets.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| Dependabot wat 1.260.0 → 1.261.0 | low | Merged [#40](https://github.com/ExyonQ/ExyonQ/pull/40) on main |
| Dependabot crypto-common 0.1.6 → 0.1.7 | low | Merged [#43](https://github.com/ExyonQ/ExyonQ/pull/43) on main |

## Waivers

None.

## Surfaces reviewed

- [x] WASM host dependency pins (`wat`, `crypto-common`)
- [x] Supply-chain lockfile after Dependabot merges
- [ ] HTTP/1.1 raw static loop (unchanged)
- [ ] Hyper proxy path (unchanged)
- [ ] TLS / HTTP/2 / HTTP/3 (unchanged)
- [ ] Modules (metrics, compression, ratelimit) (unchanged)
- [ ] `unsafe` sendfile / mmap (unchanged)

## Unsafe Changes Since Previous Release

None in this cut.

## Sign-off

- **Reviewer:** Antonio Cantallops Alba (maintainer publish cut)
- **Date:** 2026-10-12
- **Notes:** Phase1 remains NOT_CLOSED; TOTAL_COMPAT_PROMISE remains FORBIDDEN. Publication ships binaries + both GHCR images only after start-checks (rule 131).
