# ExyonQ security audit — v0.4.9

| Field | Value |
|-------|-------|
| **Version** | 0.4.9 |
| **Commit** | `d18daa9f69fd07804d6d38ec1bd8c8754499af49` |
| **Date** | 2026-10-09 |
| **Previous audit** | [`audit-v0.4.4.1.md`](audit-v0.4.4.1.md) |

## Summary

0.4.9 is the 0.4.8 product line plus the static-file deny, the WordPress `.htaccess` compile fix, the cache purge change, and the shared artifact version. No new crate was added. Wasmtime stays pinned at 49.0.1. The three Wasmtime advisories that already failed `cargo audit` on the 0.4.8 tag are unchanged and are not introduced by this diff.

Host tests covered the static deny, the `.htaccess` compiler and watcher, the FastCGI `Authorization` parameter, and the WordPress purge plugin. Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | not run on this commit | Tag workflow runs after publish |
| Security (`security.yml`) | inherited fail | `cargo audit` on v0.4.8 reported RUSTSEC-2026-0325, RUSTSEC-2026-0326, and RUSTSEC-2026-0327 against wasmtime 49.0.1. The pin is unchanged |
| Nightly fuzz | skipped | Not run for this cut |
| Functional F1–F13 | not run | Host unit tests of the changed crates passed |
| WASM host tests | N/A | WASM host was not modified |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None.

## Findings (open)

### 1. Wasmtime 49.0.1 advisories inherited from 0.4.8

- **Severidad:** alta
- **Archivo:** `Cargo.toml`
- **Función:** Wasmtime host
- **Condición:** The published pin is `=49.0.1`
- **Impacto:** RUSTSEC-2026-0325, RUSTSEC-2026-0326, and RUSTSEC-2026-0327. The same `cargo audit` failure stopped the 0.4.8 release workflow. This cut does not change the pin
- **Reproducción:** `cargo audit` on the 0.4.8 tag
- **Patch mínimo:** Bump Wasmtime to a release that includes the fixes, then rebuild both images
- **Test:** `cargo audit`
- **Estado:** open

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| static PHP and dotfile download | alta | Static routes return 403 for `.php`, `.phtml`, `.phar`, and dotfile names. `.well-known` stays reachable. `allow_sensitive = true` is the opt-in |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| | | | | |

## Surfaces reviewed

- [x] Static path resolver
- [x] Config parse (new `allow_sensitive` field, default deny)
- [x] HTTP/1.1 raw static loop (epoll returns 403 for Forbidden; does not open the file)
- [ ] Hyper proxy path
- [ ] Reverse proxy headers
- [ ] Config reload + control socket
- [ ] TLS / HTTP/2
- [ ] HTTP/3 QUIC
- [ ] Discovery env overlays
- [ ] Modules (metrics, compression, ratelimit)
- [ ] `unsafe` sendfile / mmap

## Unsafe Changes Since Previous Release

No `unsafe` block was added or edited. `sendfile`, `openat2`, and `stat` stay in `exyonq-mod-static`.

## Sign-off

- **Reviewer:** release cut for 0.4.9, host tests of the changed crates
- **Date:** 2026-10-09
- **Notes:** Security compiler was not run. Wasmtime advisories stay open from 0.4.8.
