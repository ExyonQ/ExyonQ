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
|-------|----------|
| Budget | `waf.max_inspection_body_bytes` (default 1 MiB) |
| No body | Header phase only |
| Collected body | Inspected after bounded collect; abuse not applied on body phase |
| Over budget | Controlled by `on_inspection_limit`; may emit `EXY-LIMIT-BODY-001` |
| Truncation | Must not be reported as fully inspected success when truncated |
| Streaming | Only inspected when the host presents collected/chunked bytes to the engine; unbounded buffering is not performed |

Proxy request-body transport/frame errors fail closed (`502`) and do **not**
present an empty body to WAF or upstream (Cap015 fix for former WAF-LOGIC-P3-B).

## Narrow bypasses (exact)

These are handled **before** WAF on the request path:

| Path / case | Scope |
|-------------|--------|
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

## Current limitations (product-true)

Documented from current integrated code — not copied from obsolete package notes:

- No ModSecurity CRS import.
- No ML / anomaly scoring engine.
- No response-phase (H6) detectors or hooks.
- No CAPTCHA SaaS / external challenge provider.
- Abuse cardinality is a single global policy only.
- Cap015 dual-Linux production matrix **not** executed yet.

```text
CAPABILITY_015_VERIFIED_REAL_PRODUCTION = NO
```

## Logic findings (Cap015)

Former `WAF-LOGIC-P3-A` / `WAF-LOGIC-P3-B` were reconciled and fixed at Cap015
start. Status record:
[`docs/governance/waf-open-logic-findings.md`](../governance/waf-open-logic-findings.md).
