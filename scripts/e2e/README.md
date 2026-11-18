# ExyonQ real E2E gates (authoritative)

```text
AUTHORITY = QUALIFICATION / CAPABILITY CLOSE (when dual-arch PASS)
Authoritative evidence = scripts/e2e/** only (ZERO_FAKE / REAL_E2E).
POLICY = scripts/gates/zero-smoke-category-gate.sh + scripts/gates/no-smoke-as-proof-gate.sh + scripts/gates/real-evidence-gate.sh
LINUX_EVIDENCE = Netcup amd64 + Oracle arm64 (rule 126)
```

These scripts prove real product behavior end-to-end. There is no
operational smoke category in this workspace (FORBIDDEN).

| Script | Capability | Class target |
| --- | --- | --- |
| `static-e2e.sh` | static serving | C+ |
| `security-headers-redirect-e2e.sh` | EXY-17/18: HTML cache/security headers and `www` 301 with path/query preservation | C+ |
| `proxy-e2e.sh` | reverse proxy | C+ (controlled HTTP peer) |
| `h01-l01-canonical-host-e2e.sh` | H-01/L-01: canonical path reaches the same auth/route gate, traversal is rejected, and a host-scoped route without `Host` falls to Hyper `400` | C+ (real external auth + origin peers; portable Darwin/Linux) |
| `ap04-e2e.sh` | AP-04: external auth subrequest, real HS256 JWT and opaque bearer-token checks, decisions, timeout, static bypass, user propagation | C+ (controlled HTTP auth peer) |
| `fastcgi-php-fpm-e2e.sh` | FastCGI + real PHP-FPM | C+ |
| `ol03-full-e2e.sh` | OL-03: OLS `perClientConnLimit` translation + static/dyn RPS, in/out bandwidth, soft/hard, blockBadReq; runs on Darwin and Linux | C+ |
| `ol03-ols-compare.sh` | OL-03 live comparison against the pinned `litespeedtech/openlitespeed@sha256:91ca324386be3fe8cd91591112dbdd8e41af48cccbe4a48e3e8673a2921c8504` OLS 1.9.4 oracle: single, A+B, A+B+C and soft-grace raw-TCP scenarios | E2E (external oracle) |
| `ol04-challenge-e2e.sh` | OL-04: native/monitor/reCAPTCHA v2/hCaptcha challenge, no-JS fallback, replay, forged MAC/grant and fail-closed provider rejection | C+ |
| `ol04-ol03-artifact-e2e.sh` | OL-04 + OL-03 under one artifact hash: records both binaries, runs focused tests, re-hashes after tests and after each E2E, and rejects a log whose `binary_sha256` or `compat_sha256` differs (`scripts/gates/artifact-integrity.sh`) | C+ |
| `ws-real-apps-e2e.sh` | Real Home Assistant auth handshake and authenticated n8n push/run frames through ExyonQ (requires running apps and `N8N_COOKIE_FILE`) | E2E |
| `ol05-waf-metrics-e2e.sh` | OL-05/Q-10: real CRS 942100 hit, per-rule Prometheus counter, block and detection-only modes | C+ |
| `ip82-waf-ip-filter-e2e.sh` | GitHub #82: `[waf.ip]` allow/deny list — blocking, CIDR entries, TTL expiry, `reason` and matched entry in the violation log, per-entry counters and active/expired gauges, whitelist fail-closed, fail-closed config validation | C+ |
| `f6-wp02-saturation-e2e.sh` | FastCGI saturation envelope (16 vs 40, 20 concurrent, per-pool metrics); Linux required | C+ |
| `f6-wp03-config-e2e.sh` | Published WordPress config copy: cache headers, wp-config deny, front controller; Linux required | C+ |
| `sec-h03-cache-forwarded-e2e.sh` | H-03: a cached response must not replay client-supplied forwarding headers (`X-Forwarded-Host`/`-Port`, `X-Real-IP`, `Forwarded`); two clean requests must not hit the origin again | C+ |
| `sec-h04-waf-body-tail-e2e.sh` | H-04: default WAF policy inspects the whole proxy body — a benign 2 MiB body reaches the origin and a real CRS 942100 payload at its tail is blocked, without `EXY-LIMIT-BODY-001` | C+ |
| `sec-h04-route-body-limit-e2e.sh` | H-04 cost bound: a 2 MiB `[[route]].max_request_body_bytes` answers `413` for a 64 MiB body before WAF/origin (< 5 s), while the same body on the unset route is still fully inspected; the 2 MiB boundary and a 942100 tail stay 200/403 | C+ |
| `tls-e2e.sh` | TLS ALPN + HTTPS GET + reload | C |
| `http2-e2e.sh` | HTTP/2 multiplex + body integrity | C |
| `http3-protocol-e2e.sh` | HTTP/3 wire product path | C |
| `oci-runtime-e2e.sh` | container runtime (not version-only) | C |
| `run-linux-matrix.sh` | dual-arch orchestrator | — |

`scripts/e2e/http3-product-e2e.sh` runs the committed product build (s2n) by
default. Another compiled provider is exercised without editing the harness by
exporting `EXYONQ_E2E_FEATURES` and/or `EXYONQ_E2E_NO_DEFAULT_FEATURES` (the
cargo feature set) and, for the modern alternate, `EXYONQ_E2E_H3_PROVIDER=quiche`
(the IR `[http3] provider` key). The Quinn rollback build is selected by feature
alone and ignores the IR key. Every run prints `provider_under_test` and the
`listener_banner` the provider itself logs (`s2n-quic provider`, `quiche
provider`), so the provider under test is evidence, not an assumption.

On an arm64 Linux host the same gate certifies the arm64 **server**: the product
binary is built and executed there by that host's own toolchain. The
`client_http3` line records the client backend and, for the container client,
the digest it resolved to.

The pinned `ymuski/curl-http3` image publishes only `linux/amd64`, so the
default (`EXYONQ_E2E_CLIENT=curl`) runs under the host's binfmt emulation on
arm64 while the server stays native. `EXYONQ_E2E_CLIENT=ngtcp2` replaces it
with a native client binary on the host (`EXYONQ_H3_NATIVE_CLIENT`, default
`gtlsclient` from the distro's `ngtcp2-client` package): same QUIC exchange and
same assertions, no docker and no emulation. That client cannot set request
headers, so `HEADER_TRANSLATION_SPOTCHECK`, `H3_HEADER_WITHIN_BUDGET_200` and
`H3_HEADER_OVER_BUDGET_431` print `NOT_EXECUTED ... client=ngtcp2` in that mode
(never PASS) and must be read from a `curl`-mode run. It also cannot send an
explicit zero-length body, so the empty POST is issued without data.

Known gap of the native client on the Quinn rollback provider: `gtlsclient`
does not observe the `413` for the 64 MiB oversize POST (`POST oversize
expected 413 got 000`, deterministic). The product does emit it — the same
provider and host pass `POST_413` with the `curl` client — because Quinn resets
the request stream right after the early error response and this client drops a
response whose stream was reset. Run the Quinn pass with `EXYONQ_E2E_CLIENT=curl`
when `POST_413` must be part of the evidence.

The OL-04/OL-03 artifact-bound runner is the artifact-bound E2E entry point in CI
(`.github/workflows/e2e.yml`). The job builds `exyonq` and `exyonq-compat`, runs
the runner, and fails on `ARTIFACT_DRIFT`, `ARTIFACT_LOG_UNBOUND` or
`ARTIFACT_LOG_DRIFT`; the evidence directory is uploaded even on failure. The
OL-03 log binds `binary_sha256` for `exyonq` and `compat_sha256` for
`exyonq-compat`.

A second CI job is configured to run `scripts/e2e/ols-oracle-ci.sh`, which pulls the
OpenLiteSpeed oracle only by the digest above and starts it with the fixture
configuration in `scripts/e2e/fixtures/ols-oracle/`. The comparison rejects a
container whose `Config.Image` does not contain that digest, so a future
`:latest` change cannot silently become the measured oracle.

Before the runner, the job runs
`scripts/gates/artifact-integrity-negative-control.sh`. It records a copy of
the built binaries, tampers with a copy of each binary and with a bound log
containing both hashes, and requires `ARTIFACT_DRIFT`, `ARTIFACT_LOG_DRIFT` and
`ARTIFACT_LOG_UNBOUND`; if the guard accepts either tampered artifact, the step
fails. This is a negative control for the guard, not E2E evidence.

To add the positive reCAPTCHA v3 phase, export
`OL04_RECAPTCHA_V3_SITE_KEY`, `OL04_RECAPTCHA_V3_SECRET` and a fresh
`OL04_RECAPTCHA_V3_TOKEN` generated for action `challenge` with that site key
(score >= 0.5); the site key must allow the test host. Without all three the
runner prints `NOT EXECUTED positive reCAPTCHA v3` and does not count it as a
pass.

```text
R3_RESUME_ALLOWED remains NO until owner accepts NO-SMOKE gaps closed.
```
