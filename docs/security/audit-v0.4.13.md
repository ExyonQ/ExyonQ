# ExyonQ security audit — v0.4.13

| Field | Value |
|-------|-------|
| **Version** | 0.4.13 |
| **Commit** | `e309867e6b057ab361600718fc3e918f81facfb6` (pre-tag cut; tag may share this SHA) |
| **Date** | 2026-10-14 |
| **Previous audit** | [`audit-v0.4.12.md`](audit-v0.4.12.md) |

## Summary

0.4.13 is a focused fix on the public GitHub publish line from `main` @ `5cfafce` (post-0.4.12) plus the WordPress FastCGI static-asset correction. Existing non-PHP files under a FastCGI + `.htaccess` overlay route are served statically; they are no longer sent to php-fpm (which returned `403` via `security.limit_extensions` and broke the WordPress installer UI).

Lineage: based on public `main`, **not** lab `release/v0.4.4-integration` tip (unrelated histories).

Nightly fuzz, Miri, and an official benchmark were not run for this cut.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | not run on this commit | Tag workflow runs after publish |
| Security (`security.yml`) | host `cargo audit` | Required clean before tag |
| Nightly fuzz | skipped | Not run for this cut |
| WASM host tests | host required | `cargo test -p exyonq-wasm-host` |
| Container E2E (rule 131 + 134) | required | `exyonq` GET `/` → 200; wordpress: CSS/JS 200, create admin, login → Dashboard; amd64+arm64 |
| Benchmark Tier A | not executed | No official benchmark claim |

## Blockers

None for this fix cut, contingent on: host audit clean, dual-arch container E2E PASS (including WordPress account create + login) before tag/GHCR/release assets.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| WP installer CSS/JS → php-fpm 403 | high (product usability) | Htaccess front-controller serves existing non-PHP as static; FastCGI dispatch skips non-PHP script names |

## Waivers

None.

## Surfaces reviewed

- [x] FastCGI + htaccess overlay (existing-file bypass / ServeStaticFile)
- [x] WordPress container install path (`/wp-admin` assets)
- [ ] HTTP/1.1 raw static loop (unchanged aside from FastCGI static fallback)
- [ ] Hyper proxy path (unchanged)
- [ ] TLS / HTTP/2 / HTTP/3 (unchanged)
- [ ] Modules (metrics, compression, ratelimit) (unchanged)
- [ ] `unsafe` sendfile / mmap (unchanged)

## Unsafe Changes Since Previous Release

None in this cut.

## Sign-off

- **Reviewer:** Antonio Cantallops Alba (maintainer publish cut)
- **Date:** 2026-10-14
- **Notes:** Phase1 remains NOT_CLOSED; TOTAL_COMPAT_PROMISE remains FORBIDDEN. Publication ships binaries + both GHCR images only after container E2E (rules 131 and 134).
