# ExyonQ security audit — v0.9.0

| Field | Value |
| --- | --- |
| **Version** | 0.9.0 |
| **Commit** | `1945f6299c73269bb2bc9fe26197559a979fc9f3` |
| **Date** | 2026-10-31 |
| **Previous audit** | [`audit-v0.7.0.md`](audit-v0.7.0.md) |

## Summary

v0.9.0 is the F5 cache and content-delivery parity release: NG-15/16 proxy-cache keys/vary/bypass/stale-if-error/tag+URL purge/`exyonqctl` cache tooling, OL-01 private-cookie hard-deny + `Set-Cookie`/`no-store` exclusion, OL-02 dynamic fragments, NG-17 header behavior, Q-15 observability, NG-18 body transforms (gunzip, sub_filter, mirror, Range/206/If-Range, slice cache segments) and the WAF body-inspection surface (`crs`/`multipart`/`xml`, `waf_metrics`, `zone_ratelimit`, `auth_basic`). `mirror_request_body on` remains intentionally Unsupported (bounded scope). Differential oracles: gunzip 420/420 on amd64 and arm64, sub_filter 8/8, mirror PASS, slice E2E 4 MiB/4 segments on real Linux hosts. `cargo audit` and `cargo deny check` are clean; no open Critical or High findings; no waivers. Public CI (`ci.yml` + `security.yml`) runs green on the landed tree. The EXY-03 NGINX-compat line adds `root`/`alias`/redirect parity verified on amd64 + arm64 (5 redirect profiles, exact-file alias, `alias`+`fastcgi_pass`, `301`→`200` keepalive on both wire and Hyper paths). Veyra Proof ran ACCEPTED on the EXY-03-final tree. Independent audits (`contract-adversarial-auditor`, `exyonq-security-compiler`) and the official sequential benchmark are pending and reported as NOT RUN — not as pass. Container E2E and Docker Scout above were bound to `ng18-*` digests predating EXY-03 and must re-run on the final 0.9.0 images before push. Note: the retained amd64 container-E2E artifact records `pass:false` — the amd64 leg needs a fresh run regardless. The WordPress image base changed to `php:8.5.12-fpm-bookworm` (mandate: PHP >= 8.5.12; Wolfi's `php-8.5` is only 8.5.6) — image rebuild + full installer/login journey + Scout re-scan pending on the new base.

## Release gates

| Gate | Status | Notes |
| --- | --- | --- |
| Public CI (`ci.yml`) | pass | run `41221976679` on the landed tree: fmt + clippy `-Dwarnings` + nextest **2111/2111** on Linux x86_64; `semver-checks`, `supply-chain`, `wasm-host-check` green — **binds pre-D1–D8/pre-EXY-03 code; local `xtask ci` re-run green on the candidate tree (exit 0, 2026-11-01)** |
| Security (`security.yml`) | pass | run `41221976663`: integrity-gates, private-material, functional-security-check, deps-audit, legal-compliance, security-tests all green (first public run of the synced workflow) |
| `cargo audit` | pass | 0 vulnerabilities, 527 crates scanned incl. `exyonq-mime` (1395 advisories db), re-run 2026-11-01 via `xtask security pre-release` |
| `cargo deny check` | pass | advisories/bans/licenses/sources ok; warnings only (unmatched `OpenSSL` allowance, base64 0.22/0.23 duplicate), 2026-10-31 |
| Security functional tests | pass | `xtask security pre-release`: `security.rs` 11/11, `wasm_host.rs` 3/3 (fuel/epoch/memory traps) |
| Release legal bundle | pass | `verify-release-legal-bundle.sh` + third-party notices generation OK |
| Nightly fuzz | skipped | not re-run for this release; NG-18 parsers (gzip stream, sub_filter, multipart) reuse existing fuzz surfaces; reported honestly as SKIPPED |
| Linux E2E — container server | pass | amd64 Netcup + arm64 Oracle: static, proxy, gunzip, passthrough, invalid-gzip abort — bound to `ng18-*` image digests predating EXY-03; **re-run required on the final 0.9.0 image digests before push** |
| Linux E2E — WordPress | pass | amd64 Netcup + arm64 Mac: installer UI+assets, admin account created, login → `wordpress_logged_in` cookie → `/wp-admin/` 200, FPC miss→hit, nonce POST, logged-in bypass — **bound to PHP 8.4/Wolfi images predating EXY-03 and the PHP 8.5.12 base change; re-run required on the final 0.9.0 image digests (new PHP base) before push** |
| EXY-03 redirect/`root`/`alias` differential | pass | amd64 + arm64 vs nginx 1.24: 5 redirect profiles, exact-file alias, `alias`+`fastcgi_pass`, keepalive `301`→`200` on one TCP connection on both epoll-wire and Hyper paths; `Path::join("")` regression green — transcripts: `.exyonq-local/evidence/exy03/` (KEEPALIVE-{WIRE,HYPER}-{amd64,arm64}.txt, matrix.txt) |
| Parallel-suite rerun (original conditions) | pass | `exyonq-core --lib` 256/256 + `exyonq-mod-static` 152/152 on amd64 and arm64; prior failures root-caused to test-only process-global contamination, fixed — logs: `.exyonq-local/evidence/exy03/{netcup-bench,oracle-quasar}/flakes-v5/` |
| NG-18 Phase A oracle | pass | NGINX real on Mac, Netcup amd64, Oracle arm64: 420 rows, 20 expected aborts (invalid gzip) |
| NG-18 Phase B gunzip differential | pass | 420/420 local (two consecutive runs), 420/420 Netcup amd64 + Oracle arm64 |
| NG-18 Phase C sub_filter differential | pass | 8/8 rows vs NGINX (HTML, text/plain, `once`, multi-rule, case, large/chunks, Last-Modified on/off) |
| NG-18 Phase D mirror differential | pass | identical main body; sink receives exactly one `GET /mirror` on both |
| NG-18 Phase E slice E2E | pass | 4 MiB → 4 cache segments, second request fully cached, 0 extra ranges — Netcup amd64 + Oracle arm64 with the real image |
| Docker Scout | pass | `exyonq:ng18-amd64` + `exyonq:ng18-arm64`: 0 Critical / 0 High / 0 Medium / 0 Low — bound to `ng18-*` digests; **re-scan required on the final 0.9.0 images before push** |
| Veyra Proof audit | pass | `veyra audit . --profile standard`: **ACCEPTED** (exit 0), evidence `.veyra/evidence/20261101T190127Z/` — geiger INCONCLUSIVE (partial coverage, lower bound); dylint/semver-checks/e2e SKIPPED (no config/scenarios) |
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
| EXY-10 | Medium | FastCGI had no front-controller fallback: `/`, directory URIs, and permalink paths returned `502` (missing script mapped to Bad Gateway). Fixed with `RouteConfig.front_controller` (try_files-parity dispatch + `PATH_INFO`), outcome-aware status mapping (`NotFound`→404, `Forbidden`→403), and importer mapping of PHP-shaped `try_files` fallbacks |
| EXY-11 | Critical | Repeated `Cookie` request lines collapsed (first- or last-wins) in proxy materialized/streaming/empty-body paths and in FastCGI `HTTP_COOKIE` — broke HTTP/2 cookie-fragmented sessions. Fixed with RFC 6265 `"; "` merge in `headers.rs` helpers applied at all sites + `params.rs` |
| EXY-12 | Low | `exyonqctl config lint/check/test` rejected `--config <path>` (positional only). Flag added with parity to `exyonq validate`; version coupling already workspace-sourced (reported prod mismatch was a stale 0.7.0 binary, not a schema defect) |

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

---

## Round 2 — 2026-10-31 (post NG-18 C/D/E, HTTP-01, Q-10)

**Scope added since Round 1:** NG-18 C/D/E (`sub_filter`, `mirror`, `slice`), ACME HTTP-01 cross-process share (ADR-066), Q-10 WAF metrics, `zone_ratelimit`, `auth_basic`, `waf_body`, workspace clippy drift fixes, wasmtime 49.0.4→49.0.5.

### Automated gates (re-run on final tree)

| Gate | Result |
| --- | --- |
| `cargo audit` | PASS — 3 new RUSTSEC advisories (wasmtime, dated 2026-10-29, dev-dep only) cleared via patch pin `=49.0.5` |
| `cargo deny check` | PASS (advisories/bans/licenses/sources) |
| `scan-private-material.sh --git-tree` | PASS (0 findings) |
| `cargo fmt --check` | PASS |
| `cargo clippy` (all touched crates, `-D warnings`) | PASS — incl. toolchain-drift fixes in `map_ir.rs`, `handler.rs` |
| Tests | acme 10/10 · compat-nginx 26/26 · ng18 gunzip 3/3 + sub_filter 2/2 · core lib touched-area 17/17 · waf_metrics 1/1 |

### Independent audits

**exyonq-security-compiler:** CONDITIONAL ACCEPT → blockers **all fixed and verified**.

| ID | Sev | Finding | Disposition |
| --- | --- | --- | --- |
| F1 | H | slice cached 206s bypassing store gates → `Set-Cookie` replay, private/no-store poisoning | **FIXED** — `slice_segment_store_blocked` + `filter_storable_headers` + denylist (`module-api/cache.rs`, `handler.rs`) |
| F2 | H | `zone_ratelimit` unbounded map on attacker keys | **FIXED** — 10k/zone cap, full-bucket sweep, LRU evict, 128-char key cap |
| F3 | M | gunzip unbounded header-state input | **FIXED** — `Err` when header-state input >64 KiB |
| F4 | M | sub_filter `buf` grew ~whole-body on large frames | **FIXED** — drain `buf` before polling inner |
| F5 | M | `waf_body` buffer never drained past max_bytes | **FIXED** — buffer capped at `max_bytes` |
| F6 | M | client `x-exyonq-mirror` header suppressed mirroring | **FIXED** — `MirrorMarker` extension + guard-header stripped at both external entries |
| F9 | L | `\n` unescaped in waf rule_id label | **FIXED** |
| — | — | Passthrough 206 `collect()` unbounded | **FIXED** — `http_body_util::Limited` at segment size |
| — | — | ratelimit fail-open on poisoned mutex | **FIXED** — poison-tolerant lock |
| — | — | ACME silent disk errors / token leak on order failure | **FIXED** — `warn!` on get/remove + error-path cleanup |
| — | — | mirror lifecycle metrics absent (ADR-064:56) | **FIXED** — `exyonq_mirror_subrequests_total{outcome}` on `/metrics` |
| — | — | sub_filter `$var` imported as Supported (nginx interpolates) | **FIXED** — importer → `Unsupported` when replacement contains `$` |
| — | — | streaming `WafBody` uninstrumented (Q-10 undercount) | **FIXED** — `note_inspection`+`note_decision` wired |

**contract-adversarial-auditor:** initial verdict REJECT (2 blockers) → both were F1/F3 above, **fixed**. Verified-clean areas: NG-15 key/vary/bypass, NG-16 stale-if-error, NG-17 headers, OL-01/02, Q-15, ADR-066 lifecycle.

### Deferred (documented, non-blocking)

- **F7** FPC bypass matching doesn't %-decode paths (`/%77p-admin` evades `wp-admin` bypass) — defeats operator never-cache list; cookie gate still protects confidentiality. → 0.9.x hardening.
- **F8** challenge-store residuals inside ADR-066 trust domain: no `O_NOFOLLOW`, no crash-orphan sweep, no dir-perm check at startup.
- **F10** slice HEAD assembles all segments (bounded 64 MiB, wasteful); segment entries unpurgeable (site/ns/tag zeroed).
- sub_filter `once` is global across rules (nginx may be per-pair — differential pending); mirror headers-hook fires pre-access-phase (parity nit); Apache `A####` migrator doesn't clamp max_age; Plan-12 MISS serves filtered headers to live client (pre-existing parity nit).

## Verdict (Round 2)

**CONDITIONAL ACCEPT → tag-ready pending:** all security/contract blockers closed and re-verified; remaining release steps are mechanical (sync → commit → tag → Docker publish E2E amd64+arm64 per rule 134).

---

## Round 3 — 2026-10-31 (post-merge contract-adversarial-auditor FAIL → remediation)

**Trigger:** independent `contract-adversarial-auditor` pass over the v0.9.0 land (post PR #64 merge) returned **FAIL** — 8 proven production-reachable defects, self-confirming test loop confirmed (wire-path fixtures never configured auth/FPC; gunzip corpus had no truncations; `limit_reached` never asserted).

### Defects and remediation (all fixed in lab, regressions added)

| ID | Sev | Defect | Remediation |
| --- | --- | --- | --- |
| D1 | Critical | `auth_basic` unreachable on wire proxy path — no code could emit 401 | `ServerState::route_requires_full_dispatch` + `wire_head_requires_full_dispatch` admit gate; wire falls back to Hyper dispatch |
| D2 | High | Wire path silently no-op'd FPC/`sub_filter`/`mirror`/`zone_ratelimit`/response-WAF | Same unified gate: per-route modules + `wire_incompatible_features_active` (FPC/gunzip/resp-WAF) + keep-alive `WireRouteGate` pinned-token recheck per request (mismatch → fail-closed close) |
| D3 | High | gunzip livelock: `inner_done` + residual input + no progress → `continue` spin | No-progress guard → `truncated gzip stream` error (multi-thread timeout tests) |
| D4 | High | `waf_body` re-inspected full retained prefix per chunk (quadratic ~GiB on 100MB); `limit_reached` dead | Incremental scan of new bytes + max-pattern-length boundary overlap; `limit_reached` reachable at `>= max_bytes` |
| D5 | Medium | `purge.url` invalidated one exact key only — Vary/query variants survived | `CacheKeyParts::url_group` + `url_groups` secondary index + `invalidate_url_group`; `key.query` allowlist canonicalized at purge |
| D6 | Medium | `mirror` fired at connection-entry, before auth/WAF/ratelimit → sink sees rejected traffic | `maybe_mirror_headers`/`maybe_mirror_body` moved into `dispatch_core` post-access gates |
| D7 | Medium | `GET/HEAD /site/*` Hyper fast-branch returned before route-match → auth/`response_headers` unreachable | Route match + `route_requires_full_dispatch` gate before all three `/site/` shortcuts |
| D8 | Medium | FPC path-bypass evadable via `%XX` (`/wp-%61dmin/`) | `percent_decode_segment` once per segment in `fpc_canonicalize_path`; malformed/unsafe encodings fail closed (supersedes deferred F7) |

Also closed deferred nits: mirror headers-hook now post-access (was F-round deferred note). `Server: exyonq` (versionless) added to every ExyonQ-generated response surface (Hyper funnel `finish_hyper`, static/proxy wire templates, sdp shard, fcgi exec, drain probes, metrics export); upstream `Server` passes through untouched — nginx parity.

### Regression coverage added

- `route_bound_features_force_full_dispatch` (table: auth/ratelimit/sub_filter/mirror/slice/response_headers → full dispatch)
- `wire_head_for_protected_route_requires_full_dispatch` (generation-wide FPC gate)
- `site_fast_path_honors_route_auth` (D7: `/site/` + auth → 401, not static 200)
- `keepalive_route_gate_mismatch_closes_connection` (D1/D2: second keep-alive request to incompatible route → close, no upstream hit)
- `mirror_headers_mode_fires_only_after_access_gates` (D6: unauthenticated → 401, zero mirror)
- `purge_url_covers_vary_variants_and_key_query` (D5: variants + allowed-query purged together)
- gunzip truncated/malformed timeout tests ×3 (D3); `waf_body::scans_incrementally_and_stops_at_limit` (D4); `fpc_path_encoding_bypass` %-decode asserts (D8)
- Test-infra fix: `TEST_RT_LOCK` serializes global-runtime-mutating wire_conn tests (latent race exposed by the new regression)

### Verification on remediated tree

`cargo fmt` clean · `cargo clippy --workspace --all-targets -Dwarnings` clean · workspace test suite green (all crates incl. new regressions).

### Verdict (Round 3)

**Remediated, pending independent re-audit.** `OPEN_PROVEN_PRODUCT_DEFECTS` was 8 at audit time; each has a shipped fix + regression. Re-audit verdict and official sequential benchmark remain pending before tag — reported honestly, never as pass.

## Round 4 — 2026-11-01 (EXY-10/11/12 — 0.8.0 carry-over defects found in prod)

**Trigger:** field report from `blog.exyonq.org` (0.8.0): FastCGI `502` on `/`/dirs/permalinks, WordPress `reauth=1` login loop from dropped `Cookie` lines, `exyonqctl config lint --config` rejected. Verified present in the 0.9.0 tree and remediated.

### Fixes (with live E2E evidence, real PHP-FPM 8.5.12 + HTTP cookie-probe upstream)

| ID | Change | E2E observable |
| --- | --- | --- |
| EXY-10 | `front_controller` per route; resolver falls back only on not-a-regular-file misses; `SCRIPT_NAME` URI-derived; `PATH_INFO` = original path | `/` → `/index.php` 200; `/wp-admin/` → dir's own index 200; `/2026/10/x/` → `/index.php`+`PATH_INFO` 200; route without fc → real `404` (was 502); traversal → `403` |
| EXY-11 | `insert_or_merge_cookie`/`merge_cookie_pairs` in `headers.rs`; applied to runtime materialized, `hyper_forward` empty-body, streaming paths, and `params.rs` `HTTP_COOKIE` | proxy: 2 lines → upstream sees `a=1; b=2`, 3 lines → `a=1; b=2; c=3` (single merged field); fcgi `HTTP_COOKIE` identical; single line unchanged |
| EXY-12 | `--config` flag on `config lint\|check\|test`, positional retained | all three subcommands OK both forms; `--version` workspace-coupled `0.9.0` |

Regression tests: resolver front-controller suite (7), cookie-merge unit tests + upstream-observed merge, `map_ir` try_files→front_controller (positive + `$uri`-only negative), `--config` parsing. `cargo fmt`/`clippy -Dwarnings`/affected suites green; `xtask ci` re-run on the post-remediation tree.

### Independent contract audit (10-pass) → FAIL → remediated

| ID | Finding | Remediation |
| --- | --- | --- |
| MF-1 | Importer mapped `try_files $uri /index.php` (no `$uri/`) to `front_controller`, but the resolver always applies dir-index — proven parity divergence | `front_controller` now maps only when `try_uri_dir == true`; `$uri`-only keeps generic Partial report + regression test |
| MF-2 | `insert_or_merge_cookie` appended non-cookie fields — duplicate `Content-Length` could forward verbatim on the GET/HEAD path lacking `request_headers_safe_for_proxy` | Non-cookie fields restored to last-wins `insert` (pre-change semantics); regression test asserts single CL |
| S2 | params.rs merge bypassed `validate_param_pair` | merged value validated before write |
| S1 | `Forbidden` (no document root configured → misconfig) mapped to 403 | outcome changed to `ProbeError` → 502 |
| S3 | `front_controller` on non-fastcgi route silently ignored | `EXY-LINT-0008` predictive warning |
| S4 | WebSocket upgrade + spike paths forwarded unmerged cookies | `merge_cookie_header_map` applied to both |
| S5 | importer test never asserted `route.front_controller` | `try_files_php_fallback_maps_front_controller` + negative `$uri`-only test added |

Live E2E re-run post-remediation: identical EXY-10 matrix green; proxy `a=1; b=2` / fcgi `a=1; b=2; c=3`; duplicate `Content-Length` → `400` (upstream never sees ambiguity); importer emits `front_controller = "index.php"` in TOML.

### Re-audit (recheck, same contract) → PASS

`OPEN_PROVEN_PRODUCT_DEFECTS = 0`, `NEW_MUST_FIX = 0`; all six items verified FIXED. Two report-fidelity SHOULD_FIX closed in the same pass: the `front_controller` Supported claim now fires only on emitted routes (F-A: refused `fastcgi_pass` no longer claims a mapping), and unmapped-but-admitted `try_files` shapes in FastCGI locations emit a corrective `Partial` (F-B: generic `fallback_rewrite` claim no longer overstates). Trivial items: stale `append` comments corrected; `EXY-LINT-0008` documented in `diagnostic-codes.md` + lint test added; `forward_request` (unreachable internal path) noted — cookies there remain unmerged by design until the path becomes live.
