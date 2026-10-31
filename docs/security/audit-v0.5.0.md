# ExyonQ security audit — v0.5.0

| Field | Value |
| --- | --- |
| **Version** | 0.5.0 |
| **Commit** | `e2f0a3827d71f8f6a92c20e4fcfff954432b10c7` |
| **Date** | 2026-10-18 |
| **Previous audit** | [`audit-v0.4.13.md`](audit-v0.4.13.md) |

## Summary

0.5.0 publishes the workspace that was already at product version 0.5.0, plus the container fixes required for the server image to answer `GET /` and for the WordPress image to complete account creation and login. The server image is Debian bookworm slim with glibc binaries, not the v0.4.13 scratch/musl image.

`cargo deny` reports 12 vulnerability errors against Wasmtime 49.0.2 (component-model panics and related host panics). `deny.toml` does not ignore them. The tag workflow's advisory job is expected to fail for that reason. This cut does not bump Wasmtime. Image publication is the manually built pair, checked on linux/amd64 and linux/arm64 before push.

Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| CI (`ci.yml`) | not run on this commit | Tag workflow runs after the tag is pushed |
| Security (`security.yml`) | `cargo deny` advisories fail | Wasmtime 49.0.2, 12 vulnerability errors |
| Nightly fuzz | skipped | Not run for this cut |
| WASM host tests | not re-run for this note | |
| Container E2E | required before push | Server GET `/` → 200; WordPress account create and dashboard login; amd64 and arm64 |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None for publication of the images that pass the container E2E gate. Wasmtime advisory errors stay recorded under findings and are not waived.

## Findings (open)

### 1. Wasmtime 49.0.2 component-model advisories

- **Severidad:** alta
- **Archivo:** `Cargo.lock` (crate `wasmtime` 49.0.2)
- **Función:** Wasmtime component-model and async host paths
- **Condición:** `cargo deny check advisories` against the locked tree
- **Impacto:** 12 vulnerability errors: panics and related host failures in component-model `call` / `call_async`, stream or future type confusion, guest realloc, zero-length stream writes, yielding in a start function, recursive type drop, fused string transcode, pending calls after a trap, async-entrypoint enforcement, per-instance handle tables, and exception-tag alignment
- **Reproducción:** `cargo deny check advisories`
- **Patch mínimo:** bump Wasmtime to a release that clears those advisories, in the development tree, then sync
- **Test:** `cargo deny check advisories` exits 0
- **Estado:** open

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| Server image config | high | Default config and document root so the container stays up and `GET /` is 200 |
| Addon core range | high | Official modules accept 0.5.x (`>=0.1.0, <0.6.0`) |
| WordPress `.htaccess` | high | Overlay no longer uses directives the 0.5 compiler rejects, so `/` reaches the installer |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| | | | | |

## Surfaces reviewed

- [x] Container start and `GET /` on the server image
- [x] WordPress installer account creation and dashboard login
- [ ] HTTP/1.1 raw static loop
- [ ] Hyper proxy path
- [ ] Static path resolver
- [ ] Reverse proxy headers
- [ ] Config parse + reload + control socket
- [ ] TLS / HTTP/2
- [ ] HTTP/3 QUIC
- [ ] Discovery env overlays
- [ ] Modules (metrics, compression, ratelimit)
- [ ] `unsafe` sendfile / mmap

## Sign-off

- **Reviewer:** release publication for v0.5.0
- **Date:** 2026-10-18
- **Notes:** Container E2E is the publish gate. Wasmtime findings are open, not waived.
