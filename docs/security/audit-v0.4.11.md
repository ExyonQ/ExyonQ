# ExyonQ security audit — v0.4.11

| Field | Value |
| --- | --- |
| **Version** | 0.4.11 |
| **Commit** | `52dbf9c95a5c4d86bde0f535f6030b0abbc231c4` |
| **Date** | 2026-10-12 |
| **Previous audit** | [`audit-v0.4.10.md`](audit-v0.4.10.md) |

## Summary

0.4.11 continues the public GitHub publish line from `main` @ `94d9ad6` (docs already pointed at 0.4.10) with product version **0.4.11** and an offline **Apache → ExyonQ equivalent importer** (`exyonq-compat migrate --from apache`). No new third-party crate was introduced for the importer (uses existing `exyonq-compat-common` + IR constructs). Workspace Wasmtime remains `=49.0.2`. Host `cargo audit` on this cut reported **0 vulnerabilities**.

Lineage: based on `main` (`94d9ad61ee15bf004f446e2bd5f97a85cff50b32`), not lab `release/v0.4.4-integration` tip (unrelated histories; Netcup dataplane commits not merged).

Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | not run on this commit | Tag workflow runs after publish |
| Security (`security.yml`) | host `cargo audit` clean | 0 vulnerabilities on this lockfile (Wasmtime 49.0.2) |
| Nightly fuzz | skipped | Not run for this cut |
| Functional F1–F13 | not run | Host: `cargo test -p exyonq-compat-apache --lib` |
| WASM host tests | N/A | WASM host was not modified |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| Wasmtime 49.0.1 advisories from 0.4.8–0.4.10 | high | Pin is **49.0.2** on this line; host `cargo audit` clean |

## Waivers

None.

## Surfaces reviewed

- [x] Offline Apache importer (fail-closed reject paths; no runtime Apache parse)
- [x] Compat CLI `--from apache`
- [ ] HTTP/1.1 raw static loop (unchanged)
- [ ] Hyper proxy path (unchanged on this cut)
- [x] Config IR emit from importer (static/redirect/tls/upstream/fcgi)
- [ ] TLS / HTTP/2 / HTTP/3 (unchanged)
- [ ] Modules (metrics, compression, ratelimit) (unchanged)
- [ ] `unsafe` sendfile / mmap (unchanged)

## Unsafe Changes Since Previous Release

None in this cut.

## Sign-off

- **Reviewer:** Antonio Cantallops Alba (maintainer publish cut)
- **Date:** 2026-10-12
- **Notes:** TOTAL_COMPAT_PROMISE remains FORBIDDEN; Gate A Apache corpus not closed. Publication ships binaries + both GHCR images after start-checks (rule 131).
