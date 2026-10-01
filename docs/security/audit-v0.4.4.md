# ExyonQ security audit — v0.4.4

| Field | Value |
|-------|-------|
| **Version** | 0.4.4 |
| **Commit** | `1ce0a7007171d4af6d1f9f4510137298783575ee` |
| **Date** | 2026-10-01 |
| **Previous audit** | [`audit-v0.4.3.md`](audit-v0.4.3.md) |

## Summary

This audit covers the private v0.4.4 candidate. Tag `v0.4.4` already points at `1ce0a7007171d4af6d1f9f4510137298783575ee`. That commit does not contain this file. `release.yml` stopped before creating the GitHub Release or pushing `ghcr.io/exyonq/exyonq:0.4.4` because `Cargo.lock` was still `0.4.3` and still listed `exyonq-bench`, and because this audit and `docs/release/v0.4.4.md` were missing.

The correction lives on branch `release/v0.4.4-ci-repair` in the lab repository. It rewrites `Cargo.lock` to workspace `0.4.4`, drops `exyonq-bench`, and adds the two missing documents. It does not move the tag, create the GitHub Release, or push an image.

Wasmtime is pinned at `49.0.1`. The published patched range for RUSTSEC-2026-0222 includes `>=47.0.3`, so the v0.4.3 waiver (Wasmtime 45.0.2, review at v0.4.4) is not carried forward. `.cargo/audit.toml` still lists that advisory in `ignore`; `deny.toml` does not. No new waiver is opened here.

This document is not a security-compiler sign-off and does not authorize publication.

## Release gates

| Gate | Status | Notes |
|------|--------|-------|
| CI (`ci.yml`) | not executed | Not run for this repair |
| Security (`security.yml` local equiv.) | partial | `cargo audit` exit 0 on 2026-10-01 (advisory db fetched that day): no vulnerabilities; one allowed warning, yanked `yoke-derive` 0.8.3. `cargo deny check advisories bans sources`: advisories ok, bans ok, sources ok. Security nextest and the private-material scan were not run |
| Nightly fuzz | not executed | |
| Functional F1–F13 | not executed | |
| WASM host tests | not executed | |
| Benchmark Tier A | not executed | No official benchmark claim |

Lockfile check on the correction tree, toolchain 1.98.1: `cargo update --locked --offline --workspace` exits 0 after the rewrite. Before the rewrite the same command refused to update `Cargo.lock`.

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
|----|----------|------------|
| LOCK-044 | Release gate | Tagged `Cargo.lock` kept workspace packages at 0.4.3 and still contained `exyonq-bench`, which is not a workspace member. `cargo update --offline --workspace` on toolchain 1.98.1 rewrote only those package versions and removed `exyonq-bench`. No third-party version changed |
| RUSTSEC-2026-0222 | Low | v0.4.3 waiver was Wasmtime 45.0.2 and expired for review at v0.4.4. Pin is now 49.0.1, inside the published patched range `>=47.0.3`. Not re-waived. `cargo audit` did not report it |

`.cargo/audit.toml` still ignores `RUSTSEC-2026-0222`. That ignore is stale relative to the 49.0.1 pin. It is not an open product finding and it is not removed in this commit.

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
|----|----------|--------|------------|---------|
| | | | | |

No waiver is active for v0.4.4. The v0.4.3 RUSTSEC-2026-0222 waiver is historical and is not renewed.

## Surfaces reviewed

The only product source edit in the correction is deletion of unused `write_raw_proxy_response` in `crates/exyonq-mod-proxy/src/wire_conn.rs`. It had no callers. `release.yml` sets `RUSTFLAGS=-Dwarnings`, so that warning would fail the build after the lockfile fix. The boxes below stay unchecked: the HTTP surfaces were not re-reviewed for this repair.

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

## Unsafe changes since previous release

No `unsafe` block is added or edited. The correction deletes the unused private function `write_raw_proxy_response` and does not touch an `unsafe` block. A review of `unsafe` changes in the product delta between the v0.4.3 audit commit and `1ce0a70` was not done in this repair.

## Sign-off

- **Reviewer:** local candidate repair (lockfile, audit artifact, release notes)
- **Date:** 2026-10-01
- **Notes:** **Commit** names the tag that failed `release.yml`. The correction is the child commit on `release/v0.4.4-ci-repair` that introduces this file. Replacing `v0.4.4`, creating the GitHub Release, and pushing `ghcr.io/exyonq/exyonq:0.4.4` stay unauthorized.
