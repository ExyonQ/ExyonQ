# OL-04 — External CAPTCHA providers

The challenge supports the native proof-of-work (default), reCAPTCHA v2 checkbox
and hCaptcha. reCAPTCHA v3 is **not** implemented or claimed. The provider secret is
never stored in TOML; the configuration names an environment variable:

```toml
[waf.abuse]
mode = "challenge"
provider = "recaptcha"       # native | recaptcha | hcaptcha
site_key = "public-site-key"
secret_env = "RECAPTCHA_SECRET"
# optional; defaults to the provider verification endpoint
verify_url = "https://www.google.com/recaptcha/api/siteverify"
```

For hCaptcha use `provider = "hcaptcha"`, `secret_env = "HCAPTCHA_SECRET"` and the
default endpoint `https://hcaptcha.com/siteverify`. A non-native provider requires a
non-empty `site_key` and `secret_env`; configuration validation fails otherwise.
`verify_url` may be HTTP for local tests, but production endpoints are HTTPS.

The challenge page performs the native SHA-256 proof-of-work and loads the official
provider widget. It waits for a non-empty widget token (bounded to 120 s) before
posting both the PoW and `provider_token`. The native HMAC, expiry, PoW and replay
checks run first; only then does ExyonQ call the provider. This prevents an
unauthenticated invalid proof from driving outbound provider requests.

Provider verification is a bounded server-side `POST` (5 s, 16 KiB response limit)
using OS trust roots. A missing/empty secret, provider rejection, timeout, non-200
response or malformed JSON is fail-closed (`403`); the native grant cookie is never
issued. Provider secrets are read from the process environment named by `secret_env`;
they are not serialized, logged or returned in errors.

The no-JavaScript path remains the explicit `<noscript>` failure page, not a bypass:
without a provider token and the PoW, the request stays rejected. It does not grant a
cookie or redirect.

## Measured E2E (2026-11-04)

`scripts/e2e/ol04-challenge-e2e.sh` runs the real product over HTTP with native,
monitor, reCAPTCHA v2, hCaptcha and invalid-secret configurations. The provider
cases use the vendors' official test site/secret keys and call the real
`siteverify` endpoints; no local fake endpoint is used.

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

Limits: official test keys only (no production provider account), reCAPTCHA v3
not implemented. See
`docs/waf/challenge.md` for the OpenLiteSpeed differentials (activation source,
default widget type, max-tries/whitelist).
