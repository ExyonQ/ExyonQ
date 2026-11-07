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
| `ap04-e2e.sh` | AP-04: external auth subrequest, real HS256 JWT and opaque bearer-token checks, decisions, timeout, static bypass, user propagation | C+ (controlled HTTP auth peer) |
| `fastcgi-php-fpm-e2e.sh` | FastCGI + real PHP-FPM | C+ |
| `ol03-full-e2e.sh` | OL-03: OLS `perClientConnLimit` translation + static/dyn RPS, in/out bandwidth, soft/hard, blockBadReq; runs on Darwin and Linux | C+ |
| `ol03-ols-compare.sh` | OL-03 live comparison against the `eq-ols-oracle` OLS 1.9.4 container: single, A+B, A+B+C and soft-grace raw-TCP scenarios | E2E (external oracle) |
| `ol04-challenge-e2e.sh` | OL-04: native/monitor/reCAPTCHA v2/hCaptcha challenge, no-JS fallback, replay, forged MAC/grant and fail-closed provider rejection | C+ |
| `ol04-ol03-artifact-e2e.sh` | OL-04 + OL-03 under one artifact hash: records both binaries, runs focused tests, re-hashes after tests and after each E2E, and rejects a log whose `binary_sha256` or `compat_sha256` differs (`scripts/gates/artifact-integrity.sh`) | C+ |
| `ws-real-apps-e2e.sh` | Real Home Assistant auth handshake and authenticated n8n push/run frames through ExyonQ (requires running apps and `N8N_COOKIE_FILE`) | E2E |
| `ol05-waf-metrics-e2e.sh` | OL-05/Q-10: real CRS 942100 hit, per-rule Prometheus counter, block and detection-only modes | C+ |
| `f6-wp02-saturation-e2e.sh` | FastCGI saturation envelope (16 vs 40, 20 concurrent, per-pool metrics); Linux required | C+ |
| `f6-wp03-config-e2e.sh` | Published WordPress config copy: cache headers, wp-config deny, front controller; Linux required | C+ |
| `tls-e2e.sh` | TLS ALPN + HTTPS GET + reload | C |
| `http2-e2e.sh` | HTTP/2 multiplex + body integrity | C |
| `http3-protocol-e2e.sh` | HTTP/3 wire product path | C |
| `oci-runtime-e2e.sh` | container runtime (not version-only) | C |
| `run-linux-matrix.sh` | dual-arch orchestrator | — |

The OL-04/OL-03 artifact-bound runner is the artifact-bound E2E entry point in CI
(`.github/workflows/e2e.yml`). The job builds `exyonq` and `exyonq-compat`, runs
the runner, and fails on `ARTIFACT_DRIFT`, `ARTIFACT_LOG_UNBOUND` or
`ARTIFACT_LOG_DRIFT`; the evidence directory is uploaded even on failure. The
OL-03 log binds `binary_sha256` for `exyonq` and `compat_sha256` for
`exyonq-compat`.

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
