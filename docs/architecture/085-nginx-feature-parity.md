# 085 — NGINX feature parity: what ExyonQ covers, what it should not clone

Audit of ExyonQ against the public nginx feature list (nginx.org/en/#features),
read from the working tree of this checkout (HEAD `27629a5`, workspace `0.9.3`).
Read-only: no build, no cargo, no network. Line numbers were verified against the
tree as read; the symbol quoted in each cell is the stable key if a file moves.

This document answers a product question, not a marketing one: *for each nginx
line item, does ExyonQ have the behaviour, a narrower version of it, or nothing —
and should it have it?* It is **not** a claim of substitution. The compatibility
scope is authoritative and forbids that claim:

> `TOTAL_COMPAT_PROMISE` is **FORBIDDEN** for Apache and NGINX until the
> corresponding gate corpus passes on Netcup AMD64 and Oracle ARM64 —
> `docs/compat/compatibility-scope-v1.md:49-51`

## Status vocabulary

| Status | Means |
| --- | --- |
| **IMPLEMENTED** | A real code path plus a test or E2E that exercises it. |
| **PARTIAL** | Exists but narrower than nginx; the limit is named in the row. |
| **ABSENT** | No code path. Nothing in the tree suggests it is planned. |
| **NOT PLANNED** | Absent *and* a deliberate scope decision is documented (or the importer refuses it explicitly). |
| **OUT OF SCOPE / LEGACY** | nginx-specific legacy mechanism ExyonQ should not reproduce. |
| **NOT VERIFIED** | A claim exists that this checkout cannot substantiate. Never counted as a pass. |

Rows follow the nginx list's semicolon-delimited items, in its order, grouped
exactly as nginx groups them. Every evidence cell is a real `path:line`; test and
E2E names are the ones in the tree.

## 1. Basic HTTP server features

| # | Item | Status | Evidence (path:line, test/E2E) | Limit / note |
| --- | --- | --- | --- | --- |
| 1.1 | Serving static and index files | **IMPLEMENTED** | `config/ir/src/lib.rs:433` (`root`/`index` on `RouteConfig`), `crates/exyonq-mod-static/src/root.rs`; tests `core/tests/plan11_directory_index_test.rs`, `crates/exyonq-mod-static/src/conditional.rs:16` (ETag/IMS→304); E2E `scripts/e2e/static-e2e.sh:51-84` (byte-exact hashes, HEAD, 404, directory index, traversal rejected) | nginx `root` vs `alias` is a per-route mode, not two directives: `root_mode` (`config/ir/src/lib.rs:537`) selecting `RootMode` (`:566`). |
| 1.2 | Autoindexing | **ABSENT** | A test pins the absence: `crates/exyonq-cfd-dataplane/src/static_serve.rs:502` `serves_directory_index_without_autoindex` (a directory without an index is `NotFound`). No `autoindex` key in `config/ir/`, no directive in `compat/nginx/src/`. | Directory listing is not reachable from the IR at all. See §8. |
| 1.3 | Open file descriptor cache | **PARTIAL** | `crates/exyonq-mod-static/src/sendfile_fd_cache.rs:16-30` (generation-scoped, thread-local FD reuse with `fstatat` revalidation), `:49` `FD_CACHE_MAX_ENTRIES = 64`; tree preload `config/ir/src/static_preload.rs:151`; test `crates/exyonq-mod-static/tests/static_preload_memory_fix.rs` | Not the nginx `open_file_cache` contract: no `valid`/`inactive`/`max` knobs, 64 entries per thread, no negative caching. |
| 1.4 | Accelerated reverse proxying with caching, load balancing and fault tolerance | **IMPLEMENTED** | Cache: `crates/exyonq-cache/src/lib.rs:16-40`, test `core/tests/plan12_proxy_cache_test.rs:262` (second GET is a hit, one origin hit); LB: `crates/exyonq-mod-proxy/src/selector.rs:19` smooth WRR, `:258` `FailoverMode`, `:616` `least_conn`; fault tolerance: `crates/exyonq-mod-proxy/src/retry.rs:16-21` (connect-only, max 1 retry) + passive health `crates/exyonq-mod-proxy/tests/ng08_passive_health.rs:16`; E2E `scripts/e2e/proxy-e2e.sh` (Linux-gated) | Retry is deliberately **connect-only** (`crates/exyonq-mod-proxy/src/retry.rs:16`): a 5xx or a timeout is never replayed, because request bytes may already have reached the peer. nginx `proxy_next_upstream` on 502/504 is therefore not reproduced. |
| 1.5 | FastCGI, uwsgi, SCGI, memcached via caching, load balancing, fault tolerance | **PARTIAL** | FastCGI: `crates/exyonq-mod-fastcgi/src/lib.rs:24-66` (pool, cache, diagnostics), `crates/exyonq-mod-fastcgi/tests/wire_unix_roundtrip.rs`; E2E `scripts/e2e/fastcgi-php-fpm-e2e.sh` | **FastCGI only.** `uwsgi_pass`/`scgi_pass` are refused by the importer (`compat/nginx/src/proxy_mvp.rs:32-33`); memcached is absent (no hit in `core/`, `crates/`, `modules/`). `[[fcgi_pool]]` has one `address` (`config/ir/src/lib.rs:841-846`), so there is no FastCGI multi-endpoint LB/failover. |
| 1.6 | Modular architecture with filters (gzip, byte ranges, chunked responses, XSLT, SSI, image transformation, parallel SSI) | **PARTIAL** | Module API `module-api/src/`, `addon-api/`; gzip/br/zstd/deflate `modules/compression/src/lib.rs:16`; single byte range `crates/exyonq-mod-static/src/byte_range.rs:106-123`; chunked framing delegated to hyper `core/src/server/io.rs:276`; `gunzip` `core/src/gunzip.rs:16`, `sub_filter` `core/src/sub_filter.rs:16`, `mirror` `core/src/mirror.rs:16` | **XSLT, SSI, parallel SSI and image transformation are absent** (no hits for `xslt`, `ssi`, `image_filter`). Byte ranges are **single-range only**: `parse_single_bytes_range` (`crates/exyonq-mod-static/src/byte_range.rs:123`) and multiple `Range` headers → `Ignore` (`:335-354`), so no `multipart/byteranges`. |
| 1.7 | SSL and TLS SNI | **IMPLEMENTED** | `crates/exyonq-mod-tls/src/lib.rs:353` `SniCert`, `:366-388` `SniResolver` (exact name, RFC 6125 wildcard, deterministic default, `strict_sni`), tests `:912` `sni_resolver_selects_certificate_by_server_name`, `:1015` `strict_sni_aborts_unknown_names_and_keeps_known`; per-`[[server]]` cert/key/ACME/OCSP/mTLS `config/ir/src/lib.rs:310-357` (TLS block) and `:359-395` (ACME); E2E `scripts/e2e/tls-e2e.sh`, `scripts/e2e/suites/tls-alpn-suite.sh:31-35`, `scripts/e2e/f08-mtls-crl-e2e.sh` | The dedicated multi-cert E2E named in the release note (`docs/release/v0.9.3.md:179`, `scripts/e2e/tls-multi-cert-e2e.sh`) **is not in this tree**, so multi-cert SNI is proven here by crate tests, not by a committed E2E. `scripts/e2e/suites/tls-sni-suite.sh:3` still records the older `SUPPORT_LIMIT=SINGLE_CERT_SNI` and is stale against `SniResolver`. |
| 1.8 | HTTP/2 with weighted and dependency-based prioritization | **PARTIAL** | HTTP/2: hyper `features = ["http2"]` (`Cargo.toml` workspace deps), E2E `scripts/e2e/http2-e2e.sh:2` (ALPN h2, multiplex, body integrity), `scripts/e2e/suites/http2-multiplex-suite.sh:31-46` | **Prioritization is absent.** No priority/dependency/exclusive-stream code (`grep -ri priorit crates/exyonq-mod-http3 crates/exyonq-http3-* core/src/server` returns only a quiche `PriorityUpdate` event pass-through). RFC 7540 priorities were deprecated in RFC 9113; hyper does not implement them. |
| 1.9 | HTTP/3 | **IMPLEMENTED** | `crates/exyonq-mod-http3/`, providers `crates/exyonq-http3-provider-s2n` (default), `-quiche` (optional), Quinn rollback-only `crates/exyonq-mod-http3/src/lib.rs:20-33`; E2E `scripts/e2e/http3-product-e2e.sh`, `http3-protocol-e2e.sh`, `http3-post-body-e2e.sh`, `http3-lifecycle-security-e2e.sh` | Beyond nginx's list here: HTTP/3 is **per listener**, not per `[[server]]` — `[http3] listen` is the global UDP default and a per-server `http3_listen` overrides it (`config/ir/src/lib.rs:206-220`, `:242-251`); every server resolving to the same address shares **one** UDP socket whose cert pairs are selected by SNI (`core/src/server/mod.rs:853-926`). H3 has no hot reload: the listener spawns only when every certificate on that socket is on disk (`core/src/server/mod.rs:892-900`), so enabling it after first ACME issuance needs a restart. TLS-ALPN sharing: `docs/architecture/066-acme-http01-cross-process-share.md`. |

## 2. Other HTTP server features

| # | Item | Status | Evidence (path:line, test/E2E) | Limit / note |
| --- | --- | --- | --- | --- |
| 2.1 | Name-based and IP-based virtual servers | **IMPLEMENTED** | `config/ir/src/lib.rs:268` `ServerConfig` (`listen`, `server_name`), `:433` `RouteConfig`; host matching `crates/exyonq-runtime-plan/src/router.rs:302`; E2E `scripts/e2e/h01-l01-canonical-host-e2e.sh` (host-scoped route without `Host` → `400`) | Divergence worth knowing: request→route selection uses **`route.match.host`**, not `[[server]].server_name` — `crates/exyonq-control-api/src/mutate.rs:284` ("server_name alone is not consulted"). `server_name` feeds SNI (`core/src/reload/mod.rs:446`) and the HTTP/3 cert set (`core/src/server/mod.rs:879`). IP-based vhosts are separate `listen` binds. |
| 2.2 | Keep-alive and pipelined connections | **PARTIAL** | Keep-alive: `docs/architecture/071-tuning-knobs.md:41-60` (one deadline governs every H1.1 path, h2 PING probe, h3 header deadline), epoll keep-alive carry-over state `crates/exyonq-platform-linux/src/epoll_worker.rs:294-295`; tests `core/tests/tuning_accept_workers.rs`, `core/tests/reload_lifecycle.rs` | **Pipelining is explicitly rejected, not supported**: `crates/exyonq-cfd-dataplane/src/http_parse.rs:68` ("unsupported pipelining; the caller owns that policy") and `crates/exyonq-cfd-dataplane/src/shard.rs:889` answers `400 pipelining unsupported` and records a framing-reject observation. On the Hyper path two requests in one read are not exercised by any test, so that half is NOT VERIFIED. |
| 2.3 | Access log formats, buffered log writing, fast log rotation, syslog logging | **PARTIAL** | Access events `crates/exyonq-observability/src/access.rs:22`; format is `LogFormat::{Text,Json}` `config/ir/src/logging.rs:111`; sinks file/syslog/journald/OTLP `config/ir/src/logging.rs:67-89`, `crates/exyonq-observability/src/{syslog_sink.rs:16,journald_sink.rs:16}`; size rotation with `keep` `crates/exyonq-observability/src/size_rotate.rs:16-24`; E2E `scripts/e2e/exq033-logging-retention-e2e.sh`, doc `docs/architecture/080-logging-and-retention.md:11-20` | No nginx `log_format` strings, no `access_log` per server/location with variables, no explicit write-buffer tuning. Rotation is **size-based** (`max_bytes`/`keep`), not signal-triggered reopen. |
| 2.4 | 3xx-5xx error code redirection | **PARTIAL** | `core/src/server/handler.rs:4937` `maybe_error_page`; IR `error_pages: HashMap<String,String>` `config/ir/src/lib.rs:278-281`; ADR `docs/architecture/050-error-pages-ng04.md:1`; importer `compat/nginx/src/map_ir.rs:521-548` | Keys are validated to `400..=599` (ADR-050), so **3xx pages do not exist**. No `=code` status rewrite, no `@named` target, no URL target — only an absolute path with the original status preserved. |
| 2.5 | Rewrite module (regex URI changing) | **IMPLEMENTED** | IR `config/ir/src/lib.rs:691` `RewriteConfig`, `:732` `RewriteRuleConfig` (pattern/replacement; flags `last`/`break`/`redirect`/`permanent`), `:755` `RewriteCondConfig`; importer `compat/nginx/src/location.rs:255`; ADR `docs/architecture/048-rewrite-captures-ng02.md:1`; test `core/src/server/handler.rs:6062` `patterned_rewrite_redirects_host_and_preserves_query` | Conditions are less expressive than nginx: `when` is **AND-only** over `path`/`host`/`query`/`header` (`config/ir/src/lib.rs:755-763`); no `[OR]`, no `!`, no arbitrary variables, no `QSA`/`PT`/`END` (also refused in `docs/compat/compatibility-scope-v1.md:41`). |
| 2.6 | Functions depending on client address | **PARTIAL** | `[[map]]` with `input = "remote_addr"` `config/ir/src/lib.rs:680-686` and resolution `:3301`; ADR `docs/architecture/051-variables-map-match-header-ng05.md:1`; CIDR access `config/ir/src/access.rs:52-60`; country policy `config/ir/src/geoip.rs:49` | `remote_addr` map matching is **exact string**, not CIDR (`map.values.get(input)`, `config/ir/src/lib.rs:3316-3318`). No nginx `geo $var { 10.0.0.0/8 x; }` CIDR blocks, and no `$remote_addr` condition inside `rewrite.when`. |
| 2.7 | Access control by client IP, by HTTP Basic auth, by subrequest result | **IMPLEMENTED** | IP/CIDR + country + ordered first-match rules `config/ir/src/access.rs:42-60`, tests `config/ir/src/access.rs:442,474`; Basic auth (htpasswd bcrypt/SHA/plain, constant-time) `core/src/auth_basic.rs:16`; `auth_request`-style subrequest `core/src/auth_external.rs:16` (fail-closed, 2xx/401/403 only), E2E `scripts/e2e/ap04-e2e.sh` | `satisfy any` is refused (`docs/compat/compatibility-scope-v1.md:35`). Subrequest auth is `GET`-only with a bounded forwarded-header set (`core/src/auth_external.rs:26-40`). |
| 2.8 | Referer validation | **ABSENT** | No `valid_referers`/`referer` implementation: `grep -rn "valid_referers\|referer" --include=*.rs core/src crates modules config/ir/src compat/nginx/src` returns nothing. | See §8 — spoofable by design; not worth a product feature. |
| 2.9 | PUT/DELETE/MKCOL/COPY/MOVE methods | **PARTIAL** | Proxy method mapping `core/src/server/handler.rs:4552-4554`, `crates/exyonq-mod-proxy/src/runtime.rs:821-823`; static rejects writes `core/src/server/handler.rs:5604` (test `static_method_from_hyper(&Method::PUT).is_none()`), E2E `scripts/e2e/static-admission-reject-write-e2e.sh`, `scripts/e2e/wire-reject-write-e2e.sh` | **PUT/DELETE** pass through to an upstream only. **MKCOL/COPY/MOVE and any WebDAV semantics are absent** (no hits). ExyonQ serves static files read-only. |
| 2.10 | FLV and MP4 streaming | **OUT OF SCOPE / LEGACY** | No `flv`/`mp4` module anywhere; the only `mp4` hit is a WAF test URL (`crates/exyonq-waf/tests/crs_real_probe.rs:116`). Range serving already covers progressive download (`crates/exyonq-mod-static/src/byte_range.rs:106`). | The nginx FLV/MP4 modules are pseudo-streaming shims for a dead delivery model. See §8. |
| 2.11 | Response rate limiting | **PARTIAL** | Per-client outbound bandwidth `config/ir/src/modules.rs:532` `bandwidth_bytes_per_second`, input bandwidth `:535`, soft/hard connection ceilings `:525-529`; impl `core/src/client_throttle.rs:16-18`; E2E `scripts/e2e/ol03-full-e2e.sh` | This is the OLS-shaped `perClientConnLimit` model (per **IP**, all routes). It is not nginx `limit_rate`/`limit_rate_after` (per request/location response shaping), and no such directive exists in the IR. |
| 2.12 | Limits on simultaneous connections/requests per address | **IMPLEMENTED** | Hard ceiling `config/ir/src/modules.rs:529` `max_connections_per_ip`, soft limit + grace/ban `:523-543`; request-rate token bucket per client IP `modules/ratelimit/src/lib.rs:16-32` (bounded shards, idle eviction) | Counts **HTTP requests**, not TCP flows (documented in `config/ir/src/modules.rs:515-516`); over-limit is `429`, not a socket close. |
| 2.13 | IP-based geolocation | **PARTIAL** | `config/ir/src/geoip.rs:49` `GeoIpConfig`, offline MMDB reader `crates/exyonq-geoip/src/lib.rs:141-205`, test `:263` `resolves_country_and_continent_from_real_mmdb`; policy `country:XX` in `config/ir/src/access.rs:44,140-147`; doc `docs/architecture/084-geoip-country-policy.md:1` | **Country/continent only** (`docs/architecture/084-geoip-country-policy.md:124-134`): no city, ASN, ISP or online lookup. Not migratable from nginx: the importer always emits empty geoip (`compat/nginx/src/map_ir.rs:411`), so `geo`/GeoIP directives are not mapped. |
| 2.14 | A/B testing | **ABSENT** | `split_clients` appears only as a refused directive (`compat/nginx/src/proxy_mvp.rs:40`, `compat/nginx/src/static_mvp.rs:40`, `compat/nginx/src/fastcgi_php_mvp.rs:40`); no product A/B or split-traffic feature. | Route-level canary is expressible by hand with two upstreams + host/path routes; there is no splitter primitive. |
| 2.15 | Request mirroring | **IMPLEMENTED** | `core/src/mirror.rs:16-33` (`mirror_request_body` semantics, bounded, main response never waits, failure isolated), counters (`:32-44`), ADR `docs/architecture/064-body-transforms-ng18.md` | Instrumented (`mirror_started/completed/failed`) with `cancelled` pinned at 0 by design (`core/src/mirror.rs:37-38`). |
| 2.16 | Embedded Perl | **NOT PLANNED** | No Perl anywhere in `core/`, `crates/`, `modules/`, `addon-api/`; the extension path is the wasm host `wasm/exyonq-wasm-host/` plus the module API. | See §8. |

## 3. Mail proxy server features

| # | Item | Status | Evidence | Limit / note |
| --- | --- | --- | --- | --- |
| 3.1 | IMAP/POP3/SMTP proxy with external HTTP auth, connection redirection, POP3/IMAP/SMTP auth methods, SSL, STARTTLS/STLS | **NOT PLANNED** | No mail protocol code: `grep -rn -i "imap\|pop3\|smtp\|starttls\|stls" --include=*.rs core/src crates modules config/ir/src` returns nothing; the product statement is an HTTP server/reverse proxy (`README.md:3`). The L4 proxy already covers a transparent TCP passthrough (`core/src/stream_proxy.rs:16`). | A mail proxy is an MTA-adjacent product with its own auth, protocol and abuse model. Not a gap; a different product. See §8. |

## 4. TCP/UDP proxy server features

| # | Item | Status | Evidence (path:line, test/E2E) | Limit / note |
| --- | --- | --- | --- | --- |
| 4.1 | Generic TCP and UDP proxying | **IMPLEMENTED** | `config/ir/src/stream.rs:52-68` (`[[stream]]`: listen, protocol, upstream, idle timeout, UDP session cap), dataplane `core/src/stream_proxy.rs:16-28`; doc `docs/architecture/072-l4-stream.md:1-60`; tests `core/tests/ng20_stream_proxy_test.rs`, `core/tests/ng20_quic_relay_test.rs`; E2E `scripts/e2e/ng20-stream-e2e.sh` | Deliberately pins a TCP connection / UDP source address to one endpoint; bounded UDP queue (64) and 65,535-byte datagram buffer (`core/src/stream_proxy.rs:40-43`). Stream config changes require a restart (`docs/architecture/072-l4-stream.md:80-84`). |
| 4.2 | TLS/SNI for TCP | **ABSENT** | `docs/architecture/072-l4-stream.md:5` — "ExyonQ does not parse the application protocol and **does not terminate TLS**"; `StreamConfig` has no TLS field (`config/ir/src/stream.rs:52-68`). | Transparent byte relay only. See §7 P1 for why this is the most credible stream gap. |
| 4.3 | Load balancing and fault tolerance | **IMPLEMENTED** | Reuses the proxy selector (smooth WRR, priority bands, `admin_state`): `docs/architecture/072-l4-stream.md:64-74`; `crates/exyonq-mod-proxy/src/selector.rs:19,258`; a failed connect retries with the failed index excluded (`core/src/stream_proxy.rs:87`, callers `:299,534`). | HTTP-only policies (`least_conn`, cookie/header affinity) are not applied at L4, deliberately (`docs/architecture/072-l4-stream.md:66`). |
| 4.4 | Access control by client address | **ABSENT** | `StreamConfig` carries no access list (`config/ir/src/stream.rs:52-68`); the HTTP access policy (`config/ir/src/access.rs:52`) is not consulted on the stream path. | Closest gap to a real deployment risk: a `[[stream]]` to MySQL/Redis is open to any peer that can reach the port. See §7 P1. |
| 4.5 | Functions depending on client address | **ABSENT** | Same as 4.4: no per-stream variables, maps or conditions exist. | — |
| 4.6 | Limit simultaneous connections per address | **ABSENT** | Only a **per-listener UDP session cap** exists (`max_sessions`, `config/ir/src/stream.rs:66-67`); the only selection state is a per-connection "excluded endpoint" set (`core/src/stream_proxy.rs:87`), so no per-client-IP concurrency counter exists on the TCP or UDP path. | See §7 P1. |
| 4.7 | Access log formats, buffered writing, fast rotation, syslog | **ABSENT** | Stream relays log through `tracing` (`core/src/stream_proxy.rs:41`); there is no `[[stream]]` log sink/format/rotation configuration. | Shares the process log pipeline (`docs/architecture/080-logging-and-retention.md`) but has no per-stream access records. |
| 4.8 | IP geolocation | **ABSENT** | GeoIP resolution is wired to the HTTP access path only (`config/ir/src/access.rs:140-147`, `core/src/server/handler.rs:949`); the stream path has no country context. | — |
| 4.9 | A/B testing | **ABSENT** | Same as 2.14; nothing on the stream path. | — |

**Stream group: 2 IMPLEMENTED, 7 ABSENT.** This is the largest intentional
asymmetry in the audit: ExyonQ built a correct minimal L4 relay, not nginx
`stream {}` in full.

## 5. Architecture and scalability

| # | Item | Status | Evidence (path:line, test/E2E) | Limit / note |
| --- | --- | --- | --- | --- |
| 5.1 | Master + worker processes with unprivileged workers | **NOT PLANNED** | Single process, many threads: `docs/architecture/v044-core-architecture-reconciliation-competitive-frontier.md:51`; accept workers are OS threads in one address space (`core/src/worker_memory.rs:16-22`); no privilege drop — `grep -rn "setuid\|setgid\|setgroups\|prctl" --include=*.rs core/src crates/exyonq-platform-linux` is empty. | Deliberate. Privilege separation is delegated to the service manager (systemd unit in `packaging/systemd/`), not to a master/worker fork model. See §9. |
| 5.2 | Flexible configuration | **IMPLEMENTED** | IR `config/ir/` (server/route/upstream/stream/cache/geoip/logging), merge + surface + schema crates, `exyonq validate` / `exyonqctl config test`; docs `docs/architecture/071-tuning-knobs.md`, `080`, `084`; gates `scripts/gates/config-compat-promise-gate.sh` | TOML + IR is a different (and stronger) contract than nginx's directive tree: unknown keys are rejected, not ignored. Migrants come through the offline importers, never by reading nginx config at runtime (`docs/compat/compatibility-scope-v1.md:4`). |
| 5.3 | Reconfiguration and hot upgrade without dropping serving | **PARTIAL** | Reload: `core/src/reload/mod.rs:16-25` (offline prepare → fingerprint → generation publish, `listen` changes rejected as `EXY-RELOAD-0005`), watcher `crates/exyonq-reload-runtime/src/config_watcher.rs`, tests `core/tests/reload_lifecycle.rs`, E2E `scripts/e2e/suites/tls-reload-suite.sh`, drain `core/src/server/drain_probes.rs` | **Reconfiguration: yes. Hot binary upgrade: no.** No `execve`/`SIGUSR2` anywhere, and `SIGHUP` is explicitly unsupported (`crates/exyonq-reload-runtime/src/signal_runtime.rs:21`). Reload also cannot add or move a listener: a new `listen`/`http3_listen` requires a restart. |
| 5.4 | kqueue/epoll//dev/poll/event ports/select/poll | **PARTIAL** | Own epoll pool on Linux: `crates/exyonq-platform-linux/src/epoll_worker.rs:364` (`epoll_create1`), `:400` (`epoll_wait`), `:594` (`epoll_ctl`), `:668` `conn_epoll_events`; `SO_REUSEPORT` bind `crates/exyonq-platform-linux/src/linux_bind.rs:16,32`; kqueue on macOS is reached through tokio/mio, not programmed directly | No `/dev/poll`, no event ports, no `select`/`poll` backend. Accept-worker count is configurable `[tuning].accept_workers` 1..=16 (`docs/architecture/071-tuning-knobs.md:24-39`). |
| 5.5 | kqueue features (EV_CLEAR, EV_DISABLE, NOTE_LOWAT, EV_EOF) | **OUT OF SCOPE / LEGACY** | No `EV_CLEAR`/`EV_DISABLE`/`NOTE_LOWAT`/`EV_EOF` in the tree; macOS readiness is mio's. | BSD-specific tuning ExyonQ does not sit on. |
| 5.6 | epoll features (EPOLLRDHUP, EPOLLEXCLUSIVE) | **PARTIAL** | `EPOLLIN \| EPOLLRDHUP \| EPOLLET` `crates/exyonq-platform-linux/src/epoll_worker.rs:668-670`, constants `crates/exyonq-mod-static/src/sendfile_fsm.rs:33-34` | **`EPOLLEXCLUSIVE` is not used** (`grep -rn EPOLLEXCLUSIVE` → nothing); each worker owns its own listener via `SO_REUSEPORT` instead (`crates/exyonq-platform-linux/src/linux_bind.rs:32`). |
| 5.7 | sendfile/sendfile64/sendfilev | **PARTIAL** | `libc::sendfile64` with 64-bit offsets `crates/exyonq-mod-static/src/sendfile.rs:450,535,640`, `SENDFILE_MIN_BYTES = 65536` `:27`; FSM `crates/exyonq-mod-static/src/sendfile_fsm.rs` | No **`sendfilev`** (Solaris vectored variant; `grep -rn sendfilev` → nothing). Single in-fd per call. |
| 5.8 | File AIO | **NOT PLANNED** | No `aio_*` API and no `O_DIRECT` data path; the module named io_uring is **not** io_uring: `crates/exyonq-platform-linux/src/io_uring_worker.rs:62-85` loops on `listener.accept()` with `TransportKind::IoUring` as a label, and `grep -rn "io_uring_setup\|io_uring_enter"` finds nothing. Decision recorded: `IO_URING_RECOMMENDATION = NOT_JUSTIFIED_YET` (`docs/governance/v044-architecture-fitness-review-before-p4.md:227`, `docs/governance/v044-cross-profile-performance-root-cause-review-p1-p4.md:376`). | Static file I/O is `sendfile`/preload; there is no async-file submission path and the governance docs say not to build one yet. The `io_uring` naming is a real evidence-integrity risk — see §7 P6. |
| 5.9 | DIRECTIO | **OUT OF SCOPE / LEGACY** | No `O_DIRECT` data-path flag (the only `O_DIRECT*` token is `O_DIRECTORY` for containment-safe opens, `crates/exyonq-mod-static/src/root.rs:790`) and no DIRECTIO code path; `sendfile` already avoids user-space copies. | Cache-bypass tuning for workloads ExyonQ does not target. |
| 5.10 | Accept-filters and TCP_DEFER_ACCEPT | **OUT OF SCOPE / LEGACY** | No `TCP_DEFER_ACCEPT`/accept-filter socket options; the accept path sets `SO_REUSEADDR`/`SO_REUSEPORT` only (`crates/exyonq-platform-linux/src/linux_bind.rs:16-40`). | FreeBSD/Linux latency micro-optimisation; not applicable to the async accept model. |
| 5.11 | 10k inactive keep-alive connections ≈ 2.5 MB | **NOT VERIFIED** | The repo deliberately has no such figure: per-worker memory is the whole-process `VmRSS` with `scope="shared"` (`core/src/worker_memory.rs:16-22`), and the static/tuning memory measurement is explicitly NOT EXECUTED (`docs/architecture/071-tuning-knobs.md:87-92`). Harness exists: `scripts/e2e/worker-memory-e2e.sh`. | Cannot be claimed or refuted from this checkout. Measuring it needs native Linux amd64/arm64, which the last release note records as not run (`docs/release/v0.9.3.md:173-180`). |
| 5.12 | Minimised data copies | **PARTIAL** | `sendfile` zero-copy for static bodies (`crates/exyonq-mod-static/src/sendfile.rs:16`, call `:450`), preloaded/identity-checked response headers (`crates/exyonq-mod-static/src/precooked.rs`), `TCP_CORK` batching `crates/exyonq-platform-linux/src/tcp_cork.rs:16` | No `splice`, no registered/ring buffers, no vectored write of pipelined responses. Cached/proxied bodies are materialised in memory before they are written. |

## 6. Status summary

| Group | IMPLEMENTED | PARTIAL | ABSENT | NOT PLANNED | OUT OF SCOPE / LEGACY | NOT VERIFIED | Rows |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1. Basic HTTP server | 4 | 4 | 1 | 0 | 0 | 0 | 9 |
| 2. Other HTTP server | 5 | 7 | 2 | 1 | 1 | 0 | 16 |
| 3. Mail proxy | 0 | 0 | 0 | 1 | 0 | 0 | 1 |
| 4. TCP/UDP proxy | 2 | 0 | 7 | 0 | 0 | 0 | 9 |
| 5. Architecture / scalability | 1 | 5 | 0 | 2 | 3 | 1 | 12 |
| **Total** | **12** | **16** | **10** | **4** | **4** | **1** | **47** |

Reading of the table: ExyonQ is not a partial nginx. It is **strong on the
request-handling core** (static, proxy, cache, TLS/SNI, H2/H3, auth, ACL,
limits) and **intentionally empty on the module zoo** (mail, Perl, XSLT/SSI,
FLV/MP4, A/B splitter) and on **nginx's process mechanics** (master/worker,
privilege drop, binary hot upgrade, AIO/DIRECTIO/accept-filters).

## 7. What matters for an ExyonQ 0.9.x / 1.0

Prioritised, with the reason and a rough cost (S ≈ days, M ≈ 1–2 weeks of
engineering plus real E2E, L ≈ multi-week / architectural).

1. **P1 — L4 stream hardening: per-client-IP connection cap + CIDR/country access
   on `[[stream]]`.** *Why:* the stream group is 7/9 absent and a `[[stream]]` to
   MySQL/Redis today accepts any peer that can reach the port (rows 4.4–4.6).
   This is the only place in the audit where the gap is a live abuse surface
   rather than a missing convenience. *Cost:* **S** — reuse `RouteAccessConfig`
   (`config/ir/src/access.rs:52`) and the per-IP counters already proven for HTTP
   (`config/ir/src/modules.rs:529`); add stream-side enforcement plus an E2E
   alongside `scripts/e2e/ng20-stream-e2e.sh`.
2. **P2 — `error_page` completeness: 3xx keys, `=code`, named-location target.**
   *Why:* the 4xx/5xx-only, status-preserving subset (row 2.4) is the most
   common migration refusal after `if`; ADR-050 already defines the rematch
   machinery, so the residual is IR vocabulary, not a new engine. *Cost:* **S**.
3. **P3 — nginx-compatible access-log records: per-server `access_log` sink and a
   bounded `log_format` template (common/combined + custom names).** *Why:*
   operators migrating from nginx keep parsers, dashboards and log-shipping rules
   built on the combined format (row 2.3); today the only formats are
   `Text`/`Json` with a fixed schema. This is operability, not parity theatre.
   *Cost:* **M** — template grammar, bounded rendering, plus an update to
   `docs/architecture/080-logging-and-retention.md`.
4. **P4 — reload that can add or change a listener (`listen`, `http3_listen`)
   without dropping traffic.** *Why:* reload currently rejects bind changes
   (`EXY-RELOAD-0005`, row 5.3), so adding a vhost with its own address, or
   enabling HTTP/3 after first ACME issuance, needs a restart — the one
   remaining "reconfiguration without dropping serving" hole. `SO_REUSEPORT`
   (`crates/exyonq-platform-linux/src/linux_bind.rs:32`) already exists and makes an incremental bind realistic.
   *Cost:* **M**.
5. **P5 — multi-endpoint FastCGI pool with LB + passive failover.** *Why:* the
   HTTP proxy already has WRR, priority bands and passive health
   (`crates/exyonq-mod-proxy/src/selector.rs:19`, `crates/exyonq-mod-proxy/tests/ng08_passive_health.rs`), but a FastCGI pool is one
   address (`config/ir/src/lib.rs:841`), so PHP-FPM HA is impossible (row 1.5).
   *Cost:* **M** — reuse `EndpointSelector` and the FastCGI pool, then extend
   `scripts/e2e/f6-wp02-saturation-e2e.sh`.
6. **P6 — make the `io_uring` naming honest.** *Why:* the code path advertised as
   io_uring is a blocking accept-thread pool, and the governance docs say real
   io_uring is NOT_JUSTIFIED_YET (row 5.8). Either rename
   (`TransportKind::BlockingAccept` / a renamed `EXYONQ_IO_URING` gate,
   `core/src/server/io_uring_start.rs:21`) or implement it;
   leaving the name is an evidence-integrity defect in a project whose contract
   is real-only evidence. *Cost:* **S** to rename, **L** to implement.
7. **P7 — commit the multi-cert SNI/ACME E2E.** *Why:* `docs/release/v0.9.3.md:179`
   cites `scripts/e2e/tls-multi-cert-e2e.sh`, which is not in this tree, so the
   flagship SNI feature is proven only by crate tests (row 1.7) and
   `scripts/e2e/suites/tls-sni-suite.sh:3` still claims `SINGLE_CERT_SNI`. *Cost:* **S**.

Deliberately **not** on this list: HTTP/2 prioritisation, autoindex, multi-range
byte ranges, `valid_referers`, `sendfilev`, `EPOLLEXCLUSIVE`. Each is either
deprecated, a security trade-off in the wrong direction, or invisible to
ExyonQ's workload.

## 8. Do not clone

Each item is absent or unplanned today; the point is to keep it that way, and to
record why so a future "add nginx feature X" request has the argument ready.

| Do not build | Why |
| --- | --- |
| **Embedded Perl** (and any embedded interpreter) | The extension contract is the module API (`module-api/`, `addon-api/`) and the wasm host (`wasm/exyonq-wasm-host/`). Embedding a language runtime inside the request path adds an unbounded attack surface and a second dependency/ABI policy for a feature nginx itself treats as legacy. Lua/JS embeddings fall under the same rule (`compat/nginx/src/proxy_mvp.rs:51-53` already refuses `lua`/`content_by_lua`). |
| **Mail proxy** (IMAP/POP3/SMTP, STARTTLS/STLS, external HTTP auth) | A different product with different protocol, auth and abuse semantics (row 3.1). The transparent need — "proxy a TCP service" — is already met by `[[stream]]` (`core/src/stream_proxy.rs:16`); terminating mail protocols adds an MTA-adjacent codebase that this project has no requirement, test corpus or security model for. |
| **XSLT, SSI, parallel SSI, image transformation** | Content *transformation* is application/CDN work. Each is a parser plus a resource-amplification surface (XSLT especially), and the real migration cases are already covered by `sub_filter`, `gunzip` and `mirror` (`core/src/{sub_filter,gunzip,mirror}.rs`). Image resizing in particular belongs to an image service, not an HTTP front. |
| **FLV and MP4 pseudo-streaming** | Dead delivery model for two deprecated nginx modules; progressive delivery over `Range` already works (`crates/exyonq-mod-static/src/byte_range.rs:106`). Implementing them would mean carrying media container parsers in the hot path. |
| **nginx/Apache legacy mechanisms: `autoindex`, `valid_referers`, `if` inside location, `satisfy any`, embedded Lua, `/dev/poll` and event ports, `select`/`poll` backends, `sendfilev`, accept filters / `TCP_DEFER_ACCEPT`, DIRECTIO** | Two distinct reasons, both documented here: (a) **security** — directory listings (1.2), referer checks (2.8, trivially spoofable) and `satisfy any` (`docs/compat/compatibility-scope-v1.md:35`) all weaken a guarantee ExyonQ prefers to keep fail-closed, and `if` in location is refused today (`compat/nginx/src/analyze.rs:725-732`); (b) **portability/legacy** — Solaris/BSD event and I/O mechanisms (5.4, 5.5, 5.9, 5.10) are not where ExyonQ runs, and `sendfile`/`EPOLLET` already covers the copy and readiness wins. |
| **Master/worker with unprivileged workers + binary hot upgrade** | Contradicts the single-process async design (`docs/architecture/v044-core-architecture-reconciliation-competitive-frontier.md:51`) that makes generation reload and shared caches cheap, and copies nginx's *workaround* for a C process model ExyonQ does not have. Privilege reduction belongs to systemd sandboxing; a binary upgrade means inherited-socket `execve` choreography with a large failure surface for a benefit a restart + reload already delivers (row 5.3). If isolation is ever needed, the documented path is the separate proxy dataplane in the CFR review, not a nginx-style master. |

## 9. Architecture verdict

**The design diverges on purpose, and that is the right goal.** Matching nginx
1:1 is the wrong target for three independent reasons.

1. **The compatibility contract is migration, not mechanism.** ExyonQ reads
   nginx/Apache/OLS configuration **offline** and never at runtime
   (`docs/compat/compatibility-scope-v1.md:4`), and `TOTAL_COMPAT_PROMISE` is
   forbidden (`docs/compat/compatibility-scope-v1.md:49-51`). The measured importer matrix is 79 exact / 13
   approximated / 5 refused directives over five real configs, re-measured by a
   gate (`docs/compat/COMPAT_NGINX.md:5-19`). Parity means "the migrated config
   behaves the same", not "the same syscalls run".
2. **The execution model is different by construction.** One process, one Tokio
   multi-thread runtime, an in-process epoll worker pool, Hyper for H1/H2
   (`docs/architecture/v044-core-architecture-reconciliation-competitive-frontier.md:51`); accept parallelism comes from threads plus `SO_REUSEPORT`
   (`crates/exyonq-platform-linux/src/linux_bind.rs:32`), readiness from edge-triggered epoll with `EPOLLRDHUP`
   (`crates/exyonq-platform-linux/src/epoll_worker.rs:668`). There is no master/worker, no privilege drop, and no
   hot binary upgrade (`crates/exyonq-reload-runtime/src/signal_runtime.rs:21`); reload is an offline-prepare →
   fingerprint → generation-swap with connection draining
   (`core/src/reload/mod.rs:16-25`, `core/tests/reload_lifecycle.rs`). That buys cheap
   atomic reloads and shared caches, at the cost of nginx's last-resort
   privilege separation — a trade this project has made explicitly.
3. **The differentiators are not on nginx's list at all.** Native HTTP/3 with
   s2n/quiche providers, per-server ACME (HTTP-01 + TLS-ALPN), WAF with OWASP CRS
   import and body inspection, GeoIP country policy, cache purge/`stale-if-error`
   /key equivalence, `auth_request`-style external auth, a control API and
   Prometheus metrics (`README.md:24-39`, `docs/architecture/`). A feature-parity
   scorecard that ignores these measures the wrong product.

Status of the mechanisms nginx's list asks about, stated plainly: **epoll and
`sendfile64` are real** (`crates/exyonq-platform-linux/src/epoll_worker.rs:364-400`,
`crates/exyonq-mod-static/src/sendfile.rs:450`); **epoll is edge-triggered with `EPOLLRDHUP` but without
`EPOLLEXCLUSIVE`** (5.6); **kqueue is delegated to mio and its knobs are not
used** (5.5); **`io_uring` is not implemented — the code named that way is a
blocking accept-thread pool, and the governance record says
NOT_JUSTIFIED_YET** (5.8); **no file AIO, no DIRECTIO, no accept filters**
(5.8–5.10); **binary hot upgrade does not exist, config hot reload does** (5.3);
**the 10k-keep-alive memory figure is unverified in this tree** (5.11).

Where the parity gap is real and worth spending on, it is **not** in nginx's
module zoo: it is in the L4 stream surface (4.4–4.6), error-page vocabulary
(2.4), access-log compatibility (2.3), listener-changing reload (5.3) and
FastCGI HA (1.5) — the five items in §7 that a maintainer can defend line by
line against this matrix.

## References

- `docs/compat/compatibility-scope-v1.md` — authoritative scope; importers only.
- `docs/compat/COMPAT_NGINX.md` — measured nginx directive matrix (79/13/5),
  re-measured by `scripts/gates/compat-matrices-gate.sh`.
- `docs/compat/COMPAT_MATRIX_v0.9.2.md` — matrix measured from published
  binaries.
- `docs/architecture/047…084` — per-feature ADRs cited in the rows.
- `scripts/e2e/README.md` — which E2E is authoritative for which capability, and
  which are Linux-gated or NOT EXECUTED.
