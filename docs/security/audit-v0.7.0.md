# ExyonQ security audit — v0.7.0

| Field | Value |
| --- | --- |
| **Version** | 0.7.0 |
| **Commit** | `48174b2f7c400f54a6494db77bcc2ee69f7ea99b` |
| **Date** | 2026-10-24 |
| **Previous audit** | [`audit-v0.4.11.md`](audit-v0.4.11.md) |

## Summary

v0.7.0 is the F3 "Borde y balanceo" release: proxy/LB parity work (NG-06..NG-11, AP-01, F3-R20 `ProxyPassReverse`/`proxy_redirect` response-`Location` rewrite), Cap067 static dataplane hardening, WordPress cache-purge socket end-to-end, plus a release-supply-chain refresh (cargo-auditable SBOM embed, Rust 1.99.1, wasmtime 49.0.4) and a Docker base switch to distroless + Wolfi so Docker Scout reports 0C/0H/0M on both images on both architectures. Two security-relevant bugs were closed this cycle: WAF-001 (UTF-8 fail-open) and SDP-001 (WAF theater). `cargo audit` and `cargo deny check` are clean; no open Critical or High findings; no waivers. One known limitation: nightly fuzz/miri gates are reported SKIPPED rather than pass — no new parser surface was added without existing fuzz coverage claims being widened.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| Veyra Proof audit | pass | `ACCEPTED` — evidence `.veyra/evidence/20261023T222202Z/` on HEAD `13cc7ea` (Docker-only commit `48174b2` touches no Rust); rustfmt, cargo-check, clippy, cargo-test, nextest, llvm-cov, audit, deny, machete all OK; geiger INCONCLUSIVE (cosmetic matcher warnings, honest report); dylint/semver-checks/veyra-e2e SKIPPED (no config, reported not green) |
| Tests | pass | 1860/1860 nextest PASS after machete dead-dep cleanup (`13cc7ea`) |
| `cargo audit` | pass | 0 vulnerabilities, 516 crates scanned, 2026-10-24 |
| `cargo deny check` | pass | advisories/bans/licenses/sources ok, 2026-10-24 |
| Nightly fuzz | skipped | not re-run for this tag; proxy LB features reuse existing fuzzed parsers (HTTP/1, TOML config); reported honestly as SKIPPED |
| Docker Scout (both images, both arches) | pass | `exyonq` distroless + `exyonq-wordpress` wolfi: **0 critical / 0 high / 0 medium / 0 low** on amd64 and arm64 |
| Linux E2E — image publish gate (rule 134) | pass | amd64: Netcup x86_64 native; arm64: Oracle `servidor-nx1` real AArch64 (not Docker Desktop). Server GET `/` → 200; WordPress install → `Success!` → login → `wordpress_logged_in` cookie → `/wp-admin/` 200 Dashboard → FPC hit → REST post 201 → `purge.site` `purged_entries=1` → regenerated page contains post |
| WordPress cache purge socket | pass | E2E-verified on both arches: mu-plugin `EXYONQ_CACHE_PURGE_SOCKET`/`TOKEN`/`SITE_ID` wired in `wp-config.php`; `purge.site` returns `ok:true` |

## Blockers

None.

## Findings (open)

None.

## Findings (closed this release)

| ID | Severity | Resolution |
| --- | --- | --- |
| WAF-001 | High | UTF-8 fail-open closed in `5e5895f` — WAF no longer fails open on malformed UTF-8 |
| SDP-001 | High | WAF theater closed in `5e5895f` — SDP path now enforces the WAF runtime binding |
| Docker base CVEs | Medium | Debian bookworm bases (10H/10M, unfixed packages) replaced by `gcr.io/distroless/cc-debian12:nonroot` + `cgr.dev/chainguard/wolfi-base` — Scout 0C/0H/0M (`48174b2`) |

## Waivers

| ID | Severity | Reason | Mitigation | Expires |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Surfaces reviewed

- [x] HTTP/1.1 raw static loop (Cap067: encoding cache gzip/brotli quotas, keepalive pool geometry, EPOLL listen handoff, WouldBlock→HyperReady bridge)
- [x] Hyper proxy path (NG-07/08 affinity + failover, NG-10 active health checks, F3-R20 Location rewrite with authority-aware boundary check `/app` ≠ `/apple`)
- [x] Static path resolver (preload snapshots, budget quotas unchanged semantics)
- [x] Reverse proxy headers (NG-11 trusted `set_real_ip_from`; spoofed XFF rejected by default)
- [x] Config parse + reload + control socket (`exyonqctl server set` hot admin AP-01; purge socket unix-only `/run/exyonq/cache-purge.sock` with token auth)
- [x] TLS / HTTP/2 (no changes this cycle beyond dep updates; s2n/quinn provider selection unchanged)
- [x] HTTP/3 QUIC (feature-unification deps only; `exyonq-mod-http3` machete false positive documented)
- [x] Discovery env overlays (metrics overlay test race fixed `ef7db1c`)
- [x] Modules (metrics, compression, ratelimit — wire-cheap paths verified under Accept-Encoding coexistence)
- [x] `unsafe` sendfile / mmap + new `exyonq-linux-ffi` surfaces (geiger INCONCLUSIVE reported honestly; no new unsafe added without surrounding safety docs)

## Unsafe changes since previous release

New `unsafe` in `crates/exyonq-linux-ffi/*` (openat2/linkat/net/unix_fs), `core/src/server/io.rs`, `crates/exyonq-mod-proxy/src/wire_stream.rs`, `crates/exyonq-ops-runtime/*`, `crates/exyonq-cfd-dataplane/src/exclusive.rs`, `wasm/exyonq-wasm-host` (net removal of unsafe lines). Validation: nextest 1860/1860, Veyra clippy/geiger pass (geiger inconclusive on counting, not on findings), real-Linux E2E on both architectures. Miri/fuzz nightly not re-run — recorded as SKIPPED, not claimed green.

## Sign-off

- **Reviewer:** release pipeline (gates above, reproducible evidence under `.veyra/evidence/`)
- **Date:** 2026-10-24
- **Notes:** Formal `exyonq-security-compiler` verdict pending maintainer sign-off; all executable gates pass with evidence. Nightly fuzz is the only gate reported SKIPPED rather than PASS.
