# WAF operations

## Serve

```bash
exyonq serve ./config.toml
```

Canonical policy lives in TOML (`[waf]`, `[waf.abuse]`, …). Optional env overrides
are documented in [configuration.md](./configuration.md).

Enforce signature Block:

```bash
# mode=block in TOML, or:
EXYONQ_WAF_MODE=block EXYONQ_WAF_ENFORCE=1 exyonq serve ./config.toml
```

## Body inspection

| Topic | Behavior |
| --- | --- |
| Budget | `waf.max_inspection_body_bytes` (default 1 MiB) |
| No body | Header phase only |
| Collected body | Inspected after bounded collect; abuse not applied on body phase |
| Over budget | Controlled by `on_inspection_limit`; may emit `EXY-LIMIT-BODY-001` |
| Truncation | Allowed-but-truncated bodies log access `outcome=waf_body_limit` (never `ok`); `on_inspection_limit` decides allow vs block |
| Streaming | Only inspected when the host presents collected/chunked bytes to the engine; unbounded buffering is not performed |

Proxy request-body transport/frame errors fail closed (`502`) and do **not**
present an empty body to WAF or upstream (Cap015 fix for former WAF-LOGIC-P3-B).

## Narrow bypasses (exact)

These are handled **before** WAF on the request path:

| Path / case | Scope |
| --- | --- |
| `/health` | Exact path (liveness alias) |
| `/live` | Exact path |
| `/ready` | Exact path |
| ACME HTTP-01 | Only paths for which the registered ACME integration service returns a key authorization (product ACME path form `/.well-known/acme-challenge/{token}`). **Not** a broad `/.well-known/` WAF off-switch. |

Challenge verify `/.exyonq/waf-challenge` is **not** a WAF bypass: signature
inspection still runs; abuse is skipped only to avoid recursion.

## Rate limit responses

When abuse mode is `rate_limit` and the bucket denies:

- HTTP **429**
- `Retry-After` from the token bucket (seconds, minimum 1)

Abuse is evaluated once per request and is independent of signature `mode`;
`monitor` still applies the configured Challenge/429. A valid challenge grant never
bypasses an independent signature Block.

## Challenge providers and no-JS

- Native HMAC-PoW remains the default; `provider = "recaptcha"` (v2 checkbox),
  `provider = "recaptcha_v3"` (score-based) and `provider = "hcaptcha"` are
  supported with `site_key` + `secret_env`.
- Provider verification is server-side, bounded (5 s, 16 KiB), HTTPS with OS trust
  roots, and fail-closed. Secrets come from the named environment variable and are
  never logged. v3 requires success, action `challenge` and score `>= 0.5`.
- The positive v3 provider E2E with a real v3 key is **NOT EXECUTED**; the official
  test key is a v2 key and does not return v3 score/action.
- With JavaScript disabled, the 403 page shows an explicit `<noscript>` message and
  does not grant a cookie, redirect, or alternate access.

## Current limitations (product-true)

Documented from current integrated code — not copied from obsolete package notes:

- ModSecurity/CRS rule files can be imported via `[[waf.ruleset]] rules_file`; this is
  not full ModSecurity parity (not every operator/phase/action is represented).
- No ML / anomaly scoring engine.
- No response-phase (H6) detectors or hooks.
- External providers are limited to reCAPTCHA v2 checkbox and hCaptcha; reCAPTCHA v3
  is not implemented. Provider verification needs OS CA roots (fail-closed without
  them).
- Abuse cardinality is a single global policy only.
- Cap015 dual-Linux production matrix **not** executed yet.

```text
CAPABILITY_015_VERIFIED_REAL_PRODUCTION = NO
```

## Logic findings (Cap015)

Former `WAF-LOGIC-P3-A` / `WAF-LOGIC-P3-B` were reconciled and fixed at Cap015
start. Status record:
[`docs/governance/waf-open-logic-findings.md`](../governance/waf-open-logic-findings.md).

## CRS differential evidence (OL-05)

Pinned inputs for the real OWASP CRS differential:

- OWASP CRS `v4.25.0`, commit
  `aabf675fcfc5e489b424b844e6c9f1b39802df69`. The gate rejects a checkout whose
  `HEAD` differs.
- Corpus `scripts/gates/fixtures/crs-v4.25.0-corpus.json`, 4628 cases,
  SHA-256 `1fb44865af1691e6f92bab41a2690d41c684c4ec9c38f6022edde7309ecb4c54`.
- Reference engine
  `owasp/modsecurity-crs@sha256:873ce68a658d2e3b61484a8a24d215f7d15456e7ce5aec5e7114bf000d743365`.
  The image reports nginx `1.30.6` and `libmodsecurity.so.3.0.17` (Alpine
  `3.24.2`). It is a third-party comparison oracle, not an ExyonQ-built image.
- `scripts/gates/fixtures/ol05-exclusions.json`: 33 evidence-backed parser or
  reference differences. A product defect must not be added to this list.

Reproduce both passes from the repository root:

```bash
EXYONQ_CRS_DIR="$PWD/target-linux/crs-v4.25.0" \
  CRS_DIFF_WORK="$PWD/target/crs-diff-block" \
  bash scripts/gates/crs-diff-gate.sh

CRS_DRY_RUN=1 \
  EXYONQ_CRS_DIR="$PWD/target-linux/crs-v4.25.0" \
  CRS_DIFF_WORK="$PWD/target/crs-diff-dryrun" \
  bash scripts/gates/crs-diff-gate.sh
```

The single entry point for the local verification, including both metric probes
and the `FILES`/`XML` differential, is:

```bash
EXYONQ_CRS_DIR="$PWD/target-linux/crs-v4.25.0" \
  PYTHON=python3.13 \
  bash scripts/gates/ol05-crs-verify.sh
```

On 2026-11-05 this exited `0` with `OL05_VERIFY=PASS`: block
`failures=0/4628` and `excluded_with_evidence=33/4628`; dry-run
`failures=0/4628`, `excluded_with_evidence=0/4628`,
`violations_942100=52`, `violations_949110=5947`, `would_block=5947`,
`decisions_log=22932` and `inspections=9205`; block/detection-only per-rule
probes passed; `FILES`/`XML` was 11/11 with `failures=0`. The measured binary
was `0a7536…`. The run, logs and SHA-256 manifest are recorded in
[`ol05-evidence-20261105.md`](./ol05-evidence-20261105.md). This is local
execution, not GitHub-hosted CI.

The first pass compares decisions against ModSecurity; the second is the real
monitor pass and requires no `403`/`429`, a `942100` hit and the `949110`
anomaly hit. Local results on 2026-11-04: block `failures=0/4628` with
`excluded_with_evidence=33/4628`; dry-run `failures=0/4628` with
`excluded_with_evidence=0/4628`, `violations_942100=52`,
`violations_949110=5926`, `would_block=5926`, `decisions_log=22856`,
`inspections=9148` and `latency_samples=22856`. An earlier local re-run is in
`.exyonq-local/evidence/ol05-final-gates-20261104T154104Z.log` (SHA-256
`e7c8fe586baa36b0d36b248f2bf3376b4c6c15c33c63e885f14e9cc9891f35e9`;
`BLOCK_EXIT=0`, `DRYRUN_EXIT=0`). The per-rule E2E log is
`.exyonq-local/evidence/ol05-final-metrics-20261104T154124Z.log` (SHA-256
`2a64c597fff84517c3633b3767684e7660abcda2ec30528d889f6414cdb7b438`;
`METRICS_EXITS=0,0`), and the earlier focused Rust checks log is
`.exyonq-local/evidence/ol05-final-tests-20261104T155726Z.log` (SHA-256
`6fafe44efc06c50e6a91f13fd0197afae197f72794f76437a034652f037c5d47`;
`FINAL_CODECHECKS=0,0,0`). The workflow
`.github/workflows/crs-diff.yml` runs both passes. On 2026-11-04 it was executed
end to end with `act` from a disposable checkout (`--bind`, isolated empty
`CARGO_TARGET_DIR`); the job finished `Job succeeded` with `ACT_EXIT=0`. Block:
`failures=0/4628`, `excluded_with_evidence=33/4628`; dry-run:
`failures=0/4628`, `excluded_with_evidence=0/4628`, `would_block=5926`. The Linux
binary was `617abecb57acf6368d56ca7ad083d82e46743f52a6783b2b4d42e74d6e6beb06`;
`source_revision=5958909a9de4a3e0098a4f0e1242488fea88207c` is the disposable
checkout commit, not a repository commit. Log:
`.exyonq-local/evidence/ol05-act-bind-isolated-20261104T153347Z.log` (SHA-256
`50d3fc6020797e1c7bcdfa61064fa308be86412664c5e4727583b65b30ba85da`). The runner
command was:

```bash
act workflow_dispatch -j crs-diff --bind \
  -P ubuntu-latest=catthehacker/ubuntu:act-latest \
  --env CARGO_TARGET_DIR="$PWD/target"
```

This is a local `act` CI-runner execution, not GitHub-hosted Actions; hosted CI
still requires an authorized push.

Per-rule Prometheus metrics are exercised by the public E2E script:

```bash
EXYONQ_CRS_DIR="$PWD/target-linux/crs-v4.25.0" \
  bash scripts/e2e/ol05-waf-metrics-e2e.sh block
EXYONQ_CRS_DIR="$PWD/target-linux/crs-v4.25.0" \
  bash scripts/e2e/ol05-waf-metrics-e2e.sh detection-only
```

Block mode returns `403` and records `exyonq_waf_violations_total{rule_id="942100"}`
plus `decisions_total{action="block"}`; detection-only returns `200` from the
real origin and records the same rule hit with a `log` decision.
