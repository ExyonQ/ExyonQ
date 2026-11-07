# OL-04 — External CAPTCHA providers

The challenge supports the native proof-of-work (default), reCAPTCHA v2 checkbox,
reCAPTCHA v3 score-based and hCaptcha. The provider secret is never stored in
TOML; the configuration names an environment variable:

```toml
[waf.abuse]
mode = "challenge"
provider = "recaptcha"       # native | recaptcha | recaptcha_v3 | hcaptcha
site_key = "public-site-key"
secret_env = "RECAPTCHA_SECRET"
# optional; defaults to the provider verification endpoint
verify_url = "https://www.google.com/recaptcha/api/siteverify"
```

For hCaptcha use `provider = "hcaptcha"`, `secret_env = "HCAPTCHA_SECRET"` and the
default endpoint `https://hcaptcha.com/siteverify`. For reCAPTCHA v3 use
`provider = "recaptcha_v3"`, a v3 site key and the v3 secret. A non-native provider
requires a non-empty `site_key` and `secret_env`; configuration validation fails
otherwise. `verify_url` may be HTTP for local tests, but production endpoints are
HTTPS.

The challenge page performs the native SHA-256 proof-of-work and loads the official
provider integration. reCAPTCHA v2 and hCaptcha render their widget and wait for a
non-empty widget token (bounded to 120 s). reCAPTCHA v3 uses
`api.js?render=<site_key>` and calls
`grecaptcha.execute(site_key, {action: "challenge"})`; it has no visible widget.
The token is posted with the native PoW and `provider_token`. The native HMAC,
expiry, PoW and replay checks run first; only then does ExyonQ call the provider.
This prevents an unauthenticated invalid proof from driving outbound provider
requests.

Provider verification is a bounded server-side `POST` (5 s, 16 KiB response limit)
using OS trust roots. For v2 and hCaptcha, `success: true` accepts the token. For
reCAPTCHA v3, the response must also contain `action = "challenge"` and a finite
`score` in `0.0..=1.0` that is **at least 0.5**. A missing score, a different
action, a score below 0.5, a missing/empty secret, provider rejection, timeout,
non-200 response or malformed JSON is fail-closed (`403`); the native grant cookie
is never issued. The threshold and action are fixed for this challenge; the
site-specific key and secret are configured per deployment. Provider secrets are
read from the process environment named by `secret_env`; they are not serialized,
logged or returned in errors.

The no-JavaScript path remains the explicit `<noscript>` failure page, not a bypass:
without a provider token and the PoW, the request stays rejected. It does not grant a
cookie or redirect.

## Measured E2E

`scripts/e2e/ol04-challenge-e2e.sh` runs the real product over HTTP with native,
monitor, reCAPTCHA v2, reCAPTCHA v3, hCaptcha and invalid-secret configurations.
The provider cases use the vendors' official test site/secret keys and call the
real `siteverify` endpoints; no local fake endpoint is used.

**Current v3 revalidation (2026-11-06T21:31Z):** Darwin arm64,
`target/debug/exyonq` SHA-256
`c03f7c1da91a01ee11cb5bd26b2fd2028f52c17e0a36f8881829999d0e7632cf`
(`source_revision=ed778ee`, `source_tag=v0.9.1`, HEAD
`ed778eec4b9d305a959d2968d26f276f7723225e`, sin commit), harness SHA-256
`a5e2fb702d5d4d065def31067a1d92e625c9c7c832b043458df1560597710007`:
**97/97** checks (native 16, monitor 16, reCAPTCHA v2 19, reCAPTCHA v3 14,
hCaptcha 19, invalid-secret 13), `pass=13 fail=0`, `RESULT=PASS OL-04`, exit 0.
Log `v3-20261106T213141Z/console.log`, SHA-256
`21de7a25a6f3d3e59ded984302d5de1349dd77bc45bafd0282fdb7b7080eb567`.

**Regression rerun (2026-11-06T21:45Z):** the final post-v3 regression kept
the same binary and **97/97** checks, `pass=13 fail=0`; the v3 phase remained
**14/14** fail-closed. Log
`.exyonq-local/evidence/regression-20261106T214536Z/ol04-console.log`, SHA-256
`095ff1c08d887f9bb45eebbc099aa161150a8b6f0b1926c895731f8f21e451fa`.
AP-04, OL-03 and OL-05 were rerun in the same regression; the positive v3 token
is still **NOT EXECUTED**.

**Native Linux matrix re-run (2026-11-07):** the same harness was executed on
both native Linux hosts, with the binary built on each host from this tree
(HEAD `ed778ee` plus the uncommitted v3 change; product diff
`git diff -- '*.rs' '*.toml' 'Cargo.lock'` SHA-256 `e4716e71…`). Netcup amd64: `exyonq`
`19b5a06906a9c6de279cff6a6e1fc377e849ac4f14bca24de1d65400d828e965`, **97/97**,
`pass=13 fail=0`, log
`.exyonq-local/evidence/ol04-close-20261107/amd64-netcup/ol04-console.log`
SHA-256 `90ac685f6b992ad88e0b683624d81e34dc13623cd42108cfda503d3d2c55f3cb`.
Oracle arm64: `exyonq`
`744a56fe04faf5ab697ffbf897f5eedeebabfc456b826ef9bfd8d9c6a3a99303`, **97/97**,
`pass=13 fail=0`, log
`.exyonq-local/evidence/ol04-close-20261107/arm64-oracle/ol04-console.log`
SHA-256 `2d874681489eeb1e69539379fcb409cdc5ba758a41e94a94b38459228d965a3e`.
Both runs report `RESULT=PASS OL-04`, exit 0, and the embedded `binary_sha256`
matches the guarded artifact. The positive v3 token remains **NOT EXECUTED**.

The v3 phase fetches the real v3 page (render script, site key, action and no v2
widget), performs the PoW, calls the real Google `siteverify` endpoint, and checks
that the response is rejected without a grant. It is intentionally a **fail-closed
provider test**: Google's official v2 test key is not a v3 key and its response
does not carry v3 `score`/`action`. A positive v3 token needs a real v3 key/secret
for the deployment and is **NOT EXECUTED** here; the response contract is covered
by `core/src/captcha.rs::tests::v3_requires_success_action_and_score` (accepted
score/action plus missing score, missing action, wrong action, low score and
out-of-range score). No fake endpoint or injected score is used.

**Historical v2/pre-v3 revalidation (2026-11-06):** Darwin arm64, `exyonq`
`9e300afc…`, harness `6f4b3117…`: 83/83 checks, `pass=11 fail=0`,
`RESULT=PASS OL-04`, log `current-20261106T203803Z/console.log`, SHA-256
`b026bd01dc24a92aed557b7591352b8cc4465bdcbb578043564a54bf96aed792`.

Darwin arm64 (`exyonq` `c5769c7e…`): 83/83 checks, `RESULT=PASS OL-04`, log
`full-20261104T165031Z.log` SHA-256 `58d95f52…`. Linux arm64 container
(`exyonq` `0cc88c56…`): 83/83, `RESULT=PASS OL-04`, log
`linux-full-20261104T165057Z.log` SHA-256 `441ca5d4…`.

Linux nativo (2026-11-04, SSH, binary built on each host): Netcup amd64
(`exyonq` `7052c656…`) 83/83, `RESULT=PASS OL-04`, exit 0, log
`netcup-native-20261104T233527Z.log` SHA-256
`6b3a26248c059f50f5d53bccfa8d4c782112f50d644be01176a6d76edf0e32ab`; Oracle
arm64 (`exyonq` `2fd2085c…`) 83/83, `RESULT=PASS OL-04`, exit 0, log
`oracle-native-20261104T233532Z.log` SHA-256
`2a3b0e7e990d37ad16636e4a307162959591da1cb6bcc3864e34d8e560b410f1`. The
invalid-secret case gets `success: false` from the provider and ExyonQ returns
403 without a grant.

Limits: official test keys only (no production provider account); v3 positive
provider E2E needs a real v3 key and is **NOT EXECUTED**; v3 score threshold is
fixed at 0.5 and action at `challenge`; the 2026-11-06 v3 run is Darwin arm64,
and the Linux native matrix was re-executed on 2026-11-07 with 97/97 on both
amd64 and arm64. See `docs/waf/challenge.md`
for the OpenLiteSpeed differentials (activation source, default widget type,
max-tries/whitelist).
