# ExyonQ native WAF — product overview

Status: **implemented and integrated** into the product composition root.
Not Capability 015 `VERIFIED_REAL_PRODUCTION`.

```text
WAF_IMPLEMENTED = YES
CAPABILITY_015_VERIFIED_REAL_PRODUCTION = NO
V044_WAF_INTEGRATION_STATUS = COMPLETE
```

## What it is

- Native Rust signature + abuse engine (`exyonq-waf`) behind a sync API (`exyonq-waf-api`).
- No Pingora / `ProxyHttp` runtime dependency.
- `exyonq-core` depends on **`exyonq-waf-api` only**.
- The CLI composition root (`exyonq`) installs the concrete `NativeWafEngine`,
  challenge service, and abuse gate (`SyncTokenBucket` from `exyonq-ratelimit`).

## Modes (signature engine)

| `waf.mode` | Detectors | Host reject of signature Block |
|------------|-----------|--------------------------------|
| `disabled` (or `enabled = false`) | No-op compile path | No |
| `monitor` | Run; signature `Block` coerced to `Log` | No (abuse may still Challenge/429) |
| `block` | Run; signature `Block` enforceable | Yes when enforce is on (`EXYONQ_WAF_ENFORCE` / default with mode=block) |

Abuse (`[waf.abuse]` `rate_limit` / `challenge`) is **independent** of signature mode.
A valid challenge grant **never** bypasses an independent signature Block.

## Configuration surface

Canonical TOML (see [configuration.md](./configuration.md)):

- `[waf]` — engine root
- `[waf.abuse]` — **singleton** global abuse policy
- `[[waf.ruleset]]` — named rulesets / optional custom rules
- `[[waf.exclusion]]` — scoped rule suppressions

```text
WAF_ABUSE_CARDINALITY = SINGLETON
WAF_INVALID_CONFIG_BEHAVIOR = FAIL_CLOSED
```

## Challenge

Real protocol (not an HTML stub): HMAC-SHA256 authenticated proof-of-work + grant cookie.
Details: [challenge.md](./challenge.md).

```text
WAF_CHALLENGE_REAL = YES
WAF_CHALLENGE_STATIC_STUB_ALLOWED = NO
```

## Cap015 boundary

Integration-time probes (for example benign `200` / XSS `403` on a local serve) are
**integration evidence only**. They do **not** close Capability 015.

```text
CAP015_WAF_IMPLEMENTATION_STATUS = READY_FOR_REAL_PRODUCTION_QUALIFICATION
CAPABILITY_015_STARTED = NO
```
