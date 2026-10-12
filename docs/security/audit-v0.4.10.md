# ExyonQ security audit — v0.4.10

| Field | Value |
|-------|-------|
| **Version** | 0.4.10 |
| **Commit** | `822055caa362730ab0e4bc227ee72a0bb90fc1ef` |
| **Date** | 2026-10-11 |
| **Previous audit** | [`audit-v0.4.9.md`](audit-v0.4.9.md) |

## Summary

0.4.10 continues the `v0.4.9` publish line with a version bump and documentation that records the shipped mejoras (static-file deny → 403 on the Linux epoll path, WordPress `.htaccess` compile/watcher behavior, shared `EXYONQ_ARTIFACT_VERSION`). No new crate was added. Wasmtime stays pinned at 49.0.1. The three Wasmtime advisories that already failed `cargo audit` on the 0.4.8/0.4.9 tags are unchanged and are not introduced by this diff.

Lineage: based on `1973f35191b61ec77ce6721523e14867cefb7c6c` (`v0.4.9`). `release/v0.4.4-integration` tip `e7a649f` is not an ancestor (unrelated histories) and was not merged into this cut.

Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | not run on this commit | Tag workflow may run after publish |
| Security (`security.yml`) | inherited fail | `cargo audit` on v0.4.8/v0.4.9 reported RUSTSEC-2026-0325, RUSTSEC-2026-0326, and RUSTSEC-2026-0327 against wasmtime 49.0.1. The pin is unchanged |
| Nightly fuzz | skipped | Not run for this cut |
| Functional F1–F13 | not run | No product code change beyond version/docs identity |
| WASM host tests | N/A | WASM host was not modified |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None.

## Findings (open)

### 1. Wasmtime 49.0.1 advisories inherited from 0.4.8 / 0.4.9

- **Severidad:** alta
- **Archivo:** `Cargo.toml`
- **Función:** Wasmtime host
- **Condición:** The published pin is `=49.0.1`
- **Impacto:** RUSTSEC-2026-0325, RUSTSEC-2026-0326, and RUSTSEC-2026-0327. The same `cargo audit` failure stopped prior release workflow runs. This cut does not change the pin
- **Reproducción:** `cargo audit` on the 0.4.9 tag
- **Patch mínimo:** Bump Wasmtime to a release that includes the fixes, then rebuild both images
- **Test:** `cargo audit`
- **Estado:** open

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| (none new) | — | Static PHP/dotfile deny and 403 on epoll path were closed in 0.4.9 and remain in this cut |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| | | | | |

## Surfaces reviewed

- [x] Static path resolver (unchanged since 0.4.9 tip; deny + 403 retained)
- [x] Config parse (`allow_sensitive`, default deny — unchanged)
- [x] HTTP/1.1 raw static loop (epoll Forbidden → 403 — unchanged)
- [ ] Hyper proxy path
- [ ] Reverse proxy headers
- [ ] Config reload + control socket
- [ ] TLS / HTTP/2
- [ ] HTTP/3 QUIC
- [ ] Discovery env overlays
- [ ] Modules (metrics, compression, ratelimit)
- [ ] `unsafe` sendfile / mmap

## Unsafe Changes Since Previous Release

No `unsafe` block was added or edited in this version bump. `sendfile`, `openat2`, and `stat` stay in `exyonq-mod-static` as in 0.4.9.

## Sign-off

- **Reviewer:** release cut for 0.4.10 (version/docs + lineage documentation on the 0.4.9 publish line)
- **Date:** 2026-10-11
- **Notes:** Security compiler was not run as a separate Task in this lab cut. Wasmtime advisories stay open from 0.4.8/0.4.9. Product code surface matches `v0.4.9` tip plus identity/docs.
